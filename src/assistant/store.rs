//! Disk persistence for Assistant conversations.
//!
//! Layout (workspace-scoped, channel-aware — never a literal `.plexi/`):
//!
//! ```text
//! <workspace>/<workspace_channel_dir>/assistant/
//!   state.toml                     — active_conversation = "<id>"
//!   conversations/<id>.jsonl       — one serialized `Turn` per line
//! ```
//!
//! JSON lines match the host event-log persistence style: append-only writes
//! per turn, tolerant line-by-line reads (a corrupt line is logged and
//! skipped, never fatal).

use std::io::Write;
use std::path::{Path, PathBuf};

use super::model::Turn;

#[derive(serde::Serialize, serde::Deserialize)]
struct StateToml {
    active_conversation: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    session_name: Option<String>,
    /// `/thoughts` toggle: show reasoning sections in the transcript.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    show_thoughts: Option<bool>,
}

/// Handle to the on-disk Assistant store for one workspace.
pub struct AssistantStore {
    dir: PathBuf,
}

impl AssistantStore {
    /// Store rooted in the workspace's channel dir.
    pub fn new(workspace_root: &Path) -> Self {
        Self {
            dir: workspace_root
                .join(crate::config::workspace_channel_dir())
                .join("assistant"),
        }
    }

    fn state_path(&self) -> PathBuf {
        self.dir.join("state.toml")
    }

    fn conversation_path(&self, id: &str) -> PathBuf {
        self.dir.join("conversations").join(format!("{id}.jsonl"))
    }

    /// Parse the current `state.toml`, if present and valid.
    fn read_state(&self) -> Option<StateToml> {
        let raw = std::fs::read_to_string(self.state_path()).ok()?;
        match toml::from_str::<StateToml>(&raw) {
            Ok(state) => Some(state),
            Err(e) => {
                log::error!(
                    "assistant store: invalid {}: {e}",
                    self.state_path().display()
                );
                None
            }
        }
    }

    fn write_state(&self, state: &StateToml) -> Result<(), String> {
        std::fs::create_dir_all(&self.dir)
            .map_err(|e| format!("create {}: {e}", self.dir.display()))?;
        let raw = toml::to_string(state).map_err(|e| format!("serialize state.toml: {e}"))?;
        std::fs::write(self.state_path(), raw)
            .map_err(|e| format!("write {}: {e}", self.state_path().display()))
    }

    /// The persisted active conversation id, if any.
    pub fn active_conversation(&self) -> Option<String> {
        Some(self.read_state()?.active_conversation)
    }

    /// Persist `id` as the active conversation, along with an optional
    /// session name. Other persisted preferences are preserved.
    pub fn set_active_conversation(&self, id: &str, session_name: Option<&str>) -> Result<(), String> {
        let mut state = self.read_state().unwrap_or(StateToml {
            active_conversation: String::new(),
            session_name: None,
            show_thoughts: None,
        });
        state.active_conversation = id.to_string();
        state.session_name = session_name.map(str::to_string);
        self.write_state(&state)
    }

    /// The persisted session name for the active conversation, if any.
    pub fn active_session_name(&self) -> Option<String> {
        self.read_state()?.session_name
    }

    /// The persisted `/thoughts` toggle. Default: hidden.
    pub fn show_thoughts(&self) -> bool {
        self.read_state()
            .and_then(|s| s.show_thoughts)
            .unwrap_or(false)
    }

    /// Persist the `/thoughts` toggle, preserving the rest of the state.
    pub fn set_show_thoughts(&self, show: bool) -> Result<(), String> {
        let mut state = self.read_state().unwrap_or(StateToml {
            active_conversation: String::new(),
            session_name: None,
            show_thoughts: None,
        });
        state.show_thoughts = Some(show);
        self.write_state(&state)
    }

    /// Append one turn to the conversation's JSONL file.
    pub fn append_turn(&self, id: &str, turn: &Turn) -> Result<(), String> {
        let path = self.conversation_path(id);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("create {}: {e}", parent.display()))?;
        }
        let line =
            serde_json::to_string(turn).map_err(|e| format!("serialize turn for {id}: {e}"))?;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| format!("open {}: {e}", path.display()))?;
        writeln!(file, "{line}").map_err(|e| format!("write {}: {e}", path.display()))
    }

    /// Load every turn of a conversation. Missing file = empty conversation.
    /// Corrupt lines are logged and skipped so one bad write cannot make the
    /// whole transcript unloadable.
    pub fn load_turns(&self, id: &str) -> Vec<Turn> {
        let path = self.conversation_path(id);
        let raw = match std::fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
            Err(e) => {
                log::error!("assistant store: read {}: {e}", path.display());
                return Vec::new();
            }
        };
        let mut turns = Vec::new();
        for (i, line) in raw.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<Turn>(line) {
                Ok(turn) => turns.push(turn),
                Err(e) => log::error!(
                    "assistant store: skipping corrupt line {} in {}: {e}",
                    i + 1,
                    path.display()
                ),
            }
        }
        turns
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assistant::model::TurnRole;

    #[test]
    fn round_trip_resumes_the_same_conversation() {
        let ws = tempfile::tempdir().unwrap();
        let store = AssistantStore::new(ws.path());
        assert_eq!(store.active_conversation(), None);

        let id = "conv-test-1";
        store.set_active_conversation(id, None).unwrap();
        store.append_turn(id, &Turn::now(TurnRole::User, "hello")).unwrap();
        store
            .append_turn(id, &Turn::now(TurnRole::Assistant, "hi there"))
            .unwrap();

        // A fresh store handle (same workspace) resumes the same state.
        let reopened = AssistantStore::new(ws.path());
        assert_eq!(reopened.active_conversation().as_deref(), Some(id));
        let turns = reopened.load_turns(id);
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].role, TurnRole::User);
        assert_eq!(turns[0].text, "hello");
        assert_eq!(turns[1].role, TurnRole::Assistant);
        assert_eq!(turns[1].text, "hi there");
    }

    #[test]
    fn store_path_is_channel_aware() {
        let ws = tempfile::tempdir().unwrap();
        let store = AssistantStore::new(ws.path());
        store.set_active_conversation("c1", None).unwrap();
        let expected = ws
            .path()
            .join(crate::config::workspace_channel_dir())
            .join("assistant")
            .join("state.toml");
        assert!(expected.is_file(), "missing {}", expected.display());
    }

    #[test]
    fn show_thoughts_round_trips_and_survives_conversation_switch() {
        let ws = tempfile::tempdir().unwrap();
        let store = AssistantStore::new(ws.path());
        assert!(!store.show_thoughts(), "default is hidden");

        store.set_show_thoughts(true).unwrap();
        assert!(store.show_thoughts());

        // Switching conversations must not clobber the preference.
        store.set_active_conversation("c2", Some("notes")).unwrap();
        assert!(store.show_thoughts());
        assert_eq!(store.active_conversation().as_deref(), Some("c2"));
        assert_eq!(store.active_session_name().as_deref(), Some("notes"));

        store.set_show_thoughts(false).unwrap();
        assert!(!store.show_thoughts());
        assert_eq!(store.active_conversation().as_deref(), Some("c2"));
    }

    #[test]
    fn missing_conversation_loads_empty_and_corrupt_lines_are_skipped() {
        let ws = tempfile::tempdir().unwrap();
        let store = AssistantStore::new(ws.path());
        assert!(store.load_turns("nope").is_empty());

        let id = "conv-corrupt";
        store.append_turn(id, &Turn::now(TurnRole::User, "ok")).unwrap();
        let path = ws
            .path()
            .join(crate::config::workspace_channel_dir())
            .join("assistant")
            .join("conversations")
            .join(format!("{id}.jsonl"));
        let mut raw = std::fs::read_to_string(&path).unwrap();
        raw.push_str("not json\n");
        std::fs::write(&path, raw).unwrap();
        store.append_turn(id, &Turn::now(TurnRole::User, "after")).unwrap();

        let turns = store.load_turns(id);
        assert_eq!(turns.len(), 2, "corrupt line skipped, valid lines kept");
        assert_eq!(turns[1].text, "after");
    }
}

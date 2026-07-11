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
//! JSON lines with tolerant line-by-line reads (a corrupt line is logged and
//! skipped, never fatal). Each save rewrites the whole transcript: turns can
//! finish out of submission order (a reply commits at its anchor while
//! slash-view rows append), so disk order mirrors memory order by rewrite.

use std::io::Write;
use std::path::{Path, PathBuf};

use super::model::Turn;
use crate::plexi_ai::broker::ReasoningEffort;

#[derive(Default, serde::Serialize, serde::Deserialize)]
struct StateToml {
    active_conversation: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    session_name: Option<String>,
    /// `/thoughts` toggle: show reasoning sections in the transcript.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    show_thoughts: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    active_agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    effort_override: Option<ReasoningEffort>,
    #[serde(default)]
    turn_in_flight: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationSummary {
    pub id: String,
    pub title: String,
    pub turn_count: usize,
    pub active: bool,
    updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CheckpointMetadata {
    pub id: String,
    pub label: String,
    pub created_at: String,
    pub turn_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CompactionMetadata {
    pub checkpoint_id: String,
    pub created_at: String,
    pub compacted_turns: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ConversationHistory {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub checkpoints: Vec<CheckpointMetadata>,
    #[serde(default)]
    pub compactions: Vec<CompactionMetadata>,
    #[serde(default)]
    pub interruptions: Vec<String>,
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

    fn history_path(&self, id: &str) -> PathBuf {
        self.dir.join("history").join(format!("{id}.json"))
    }

    fn checkpoint_path(&self, conversation_id: &str, checkpoint_id: &str) -> PathBuf {
        self.dir
            .join("checkpoints")
            .join(conversation_id)
            .join(format!("{checkpoint_id}.jsonl"))
    }

    fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
        let parent = path
            .parent()
            .ok_or_else(|| format!("write {}: missing parent", path.display()))?;
        std::fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
        let temp = parent.join(format!(
            ".{}.tmp-{}",
            path.file_name().unwrap_or_default().to_string_lossy(),
            uuid::Uuid::new_v4()
        ));
        let result = (|| {
            let mut file = std::fs::File::create(&temp)
                .map_err(|e| format!("create {}: {e}", temp.display()))?;
            file.write_all(bytes)
                .map_err(|e| format!("write {}: {e}", temp.display()))?;
            file.sync_all()
                .map_err(|e| format!("sync {}: {e}", temp.display()))?;
            std::fs::rename(&temp, path)
                .map_err(|e| format!("rename {} to {}: {e}", temp.display(), path.display()))
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temp);
        }
        result
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
        let raw = toml::to_string(state).map_err(|e| format!("serialize state.toml: {e}"))?;
        Self::atomic_write(&self.state_path(), raw.as_bytes())
    }

    /// The persisted active conversation id, if any.
    pub fn active_conversation(&self) -> Option<String> {
        Some(self.read_state()?.active_conversation)
    }

    /// Persist `id` as the active conversation, along with an optional
    /// session name. Other persisted preferences are preserved.
    pub fn set_active_conversation(
        &self,
        id: &str,
        session_name: Option<&str>,
        active_agent_id: &str,
        effort_override: Option<ReasoningEffort>,
    ) -> Result<(), String> {
        let mut state = self.read_state().unwrap_or_default();
        state.active_conversation = id.to_string();
        state.session_name = session_name.map(str::to_string);
        state.active_agent_id = Some(active_agent_id.to_string());
        state.effort_override = effort_override;
        self.write_state(&state)?;
        let mut history = self.load_history(id)?;
        history.name = session_name.map(str::to_string);
        history.updated_at = crate::host::event_log::now_timestamp();
        self.write_history(id, &history)
    }

    /// The persisted session name for the active conversation, if any.
    pub fn active_session_name(&self) -> Option<String> {
        self.read_state()?.session_name
    }

    pub fn active_agent_id(&self) -> Option<String> {
        self.read_state()?.active_agent_id
    }

    pub fn effort_override(&self) -> Option<ReasoningEffort> {
        self.read_state()?.effort_override
    }

    /// The persisted `/thoughts` toggle. Default: hidden.
    pub fn show_thoughts(&self) -> bool {
        self.read_state()
            .and_then(|s| s.show_thoughts)
            .unwrap_or(false)
    }

    /// Persist the `/thoughts` toggle, preserving the rest of the state.
    pub fn set_show_thoughts(&self, show: bool) -> Result<(), String> {
        let mut state = self.read_state().unwrap_or_default();
        state.show_thoughts = Some(show);
        self.write_state(&state)
    }

    pub fn set_turn_in_flight(&self, in_flight: bool) -> Result<(), String> {
        let mut state = self.read_state().unwrap_or_default();
        state.turn_in_flight = in_flight;
        self.write_state(&state)
    }

    /// Clear a persisted in-flight marker and record that restart interrupted it.
    pub fn recover_interrupted_turn(&self, conversation_id: &str) -> Result<bool, String> {
        let mut state = self.read_state().unwrap_or_default();
        if !state.turn_in_flight || state.active_conversation != conversation_id {
            return Ok(false);
        }
        let mut history = self.load_history(conversation_id)?;
        history
            .interruptions
            .push(crate::host::event_log::now_timestamp());
        self.write_history(conversation_id, &history)?;
        state.turn_in_flight = false;
        self.write_state(&state)?;
        Ok(true)
    }

    /// Write the full conversation transcript, replacing the file. A turn
    /// finishing mid-conversation inserts at its anchor rather than at the
    /// end, so disk order can only be kept equal to memory order by
    /// rewriting — append-only would freeze the order of the racing rows.
    pub fn write_turns(&self, id: &str, turns: &[Turn]) -> Result<(), String> {
        let path = self.conversation_path(id);
        let mut raw = String::new();
        for turn in turns {
            let line =
                serde_json::to_string(turn).map_err(|e| format!("serialize turn for {id}: {e}"))?;
            raw.push_str(&line);
            raw.push('\n');
        }
        Self::atomic_write(&path, raw.as_bytes())
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

    pub fn list_conversations(&self) -> Result<Vec<ConversationSummary>, String> {
        let active = self.active_conversation();
        let dir = self.dir.join("conversations");
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(format!("list {}: {e}", dir.display())),
        };
        let mut items = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| format!("list {}: {e}", dir.display()))?;
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("jsonl") {
                continue;
            }
            let Some(id) = path.file_stem().and_then(|value| value.to_str()) else {
                continue;
            };
            let turns = self.load_turns(id);
            let title = turns
                .iter()
                .find(|turn| matches!(turn.role, super::model::TurnRole::User))
                .map(|turn| turn.text.chars().take(60).collect())
                .unwrap_or_else(|| "Untitled conversation".to_string());
            let history = self.load_history(id)?;
            items.push(ConversationSummary {
                id: id.to_string(),
                title: history.name.unwrap_or(title),
                turn_count: turns.len(),
                active: active.as_deref() == Some(id),
                updated_at: history.updated_at,
            });
        }
        items.sort_by(|a, b| {
            b.updated_at
                .cmp(&a.updated_at)
                .then_with(|| b.id.cmp(&a.id))
        });
        Ok(items)
    }

    pub fn load_history(&self, id: &str) -> Result<ConversationHistory, String> {
        let path = self.history_path(id);
        match std::fs::read_to_string(&path) {
            Ok(raw) => match serde_json::from_str(&raw) {
                Ok(history) => Ok(history),
                Err(e) => {
                    log::error!(
                        "assistant store: invalid {}: {e}; using empty history metadata",
                        path.display()
                    );
                    Ok(ConversationHistory::default())
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Ok(ConversationHistory::default())
            }
            Err(e) => Err(format!("read {}: {e}", path.display())),
        }
    }

    fn write_history(&self, id: &str, history: &ConversationHistory) -> Result<(), String> {
        let raw = serde_json::to_vec_pretty(history)
            .map_err(|e| format!("serialize history for {id}: {e}"))?;
        Self::atomic_write(&self.history_path(id), &raw)
    }

    pub fn write_checkpoint(
        &self,
        conversation_id: &str,
        label: &str,
        turns: &[Turn],
    ) -> Result<CheckpointMetadata, String> {
        let id = format!("checkpoint-{}", uuid::Uuid::new_v4());
        let mut raw = String::new();
        for turn in turns {
            raw.push_str(
                &serde_json::to_string(turn)
                    .map_err(|e| format!("serialize checkpoint {id}: {e}"))?,
            );
            raw.push('\n');
        }
        Self::atomic_write(&self.checkpoint_path(conversation_id, &id), raw.as_bytes())?;
        let metadata = CheckpointMetadata {
            id,
            label: label.to_string(),
            created_at: crate::host::event_log::now_timestamp(),
            turn_count: turns.len(),
        };
        let mut history = self.load_history(conversation_id)?;
        history.checkpoints.push(metadata.clone());
        self.write_history(conversation_id, &history)?;
        Ok(metadata)
    }

    pub fn load_checkpoint(
        &self,
        conversation_id: &str,
        checkpoint_id: &str,
    ) -> Result<Vec<Turn>, String> {
        let history = self.load_history(conversation_id)?;
        let checkpoint_id = history
            .checkpoints
            .iter()
            .find(|checkpoint| checkpoint.id == checkpoint_id)
            .map(|checkpoint| checkpoint.id.as_str())
            .ok_or_else(|| format!("unknown checkpoint `{checkpoint_id}`"))?;
        let path = self.checkpoint_path(conversation_id, checkpoint_id);
        let raw =
            std::fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
        raw.lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                serde_json::from_str(line).map_err(|e| format!("parse {}: {e}", path.display()))
            })
            .collect()
    }

    pub fn record_compaction(
        &self,
        conversation_id: &str,
        checkpoint_id: &str,
        compacted_turns: usize,
    ) -> Result<(), String> {
        let mut history = self.load_history(conversation_id)?;
        history.compactions.push(CompactionMetadata {
            checkpoint_id: checkpoint_id.to_string(),
            created_at: crate::host::event_log::now_timestamp(),
            compacted_turns,
        });
        self.write_history(conversation_id, &history)
    }

    pub fn export_conversation(
        &self,
        id: &str,
        turns: &[Turn],
        audit: &str,
    ) -> Result<PathBuf, String> {
        let path = self.dir.join("exports").join(format!("{id}.md"));
        let mut raw = format!("# Assistant conversation {id}\n\n");
        for turn in turns {
            raw.push_str(&format!(
                "## {:?}, {}\n\n{}\n\n",
                turn.role, turn.created_at, turn.text
            ));
            if let Some(thoughts) = &turn.thoughts {
                raw.push_str(&format!("### Reasoning\n\n{thoughts}\n\n"));
            }
        }
        raw.push_str("# Tool and permission audit\n\n```jsonl\n");
        raw.push_str(audit);
        raw.push_str("\n```\n");
        Self::atomic_write(&path, raw.as_bytes())?;
        Ok(path)
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
        store
            .set_active_conversation(id, None, "default", None)
            .unwrap();
        store
            .write_turns(
                id,
                &[
                    Turn::now(TurnRole::User, "hello"),
                    Turn::now(TurnRole::Assistant, "hi there"),
                ],
            )
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

        // A later save replaces the transcript — no stale tail lines.
        reopened
            .write_turns(id, &[Turn::now(TurnRole::User, "only")])
            .unwrap();
        let turns = reopened.load_turns(id);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].text, "only");
    }

    #[test]
    fn store_path_is_channel_aware() {
        let ws = tempfile::tempdir().unwrap();
        let store = AssistantStore::new(ws.path());
        store
            .set_active_conversation("c1", None, "default", None)
            .unwrap();
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
        store
            .set_active_conversation("c2", Some("notes"), "default", None)
            .unwrap();
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
        store
            .write_turns(
                id,
                &[
                    Turn::now(TurnRole::User, "ok"),
                    Turn::now(TurnRole::User, "after"),
                ],
            )
            .unwrap();
        let path = ws
            .path()
            .join(crate::config::workspace_channel_dir())
            .join("assistant")
            .join("conversations")
            .join(format!("{id}.jsonl"));
        // Inject a corrupt line between the two valid ones.
        let raw = std::fs::read_to_string(&path).unwrap();
        let (first, rest) = raw.split_once('\n').unwrap();
        std::fs::write(&path, format!("{first}\nnot json\n{rest}")).unwrap();

        let turns = store.load_turns(id);
        assert_eq!(turns.len(), 2, "corrupt line skipped, valid lines kept");
        assert_eq!(turns[1].text, "after");
    }

    #[test]
    fn lists_multiple_conversations_with_active_marker() {
        let ws = tempfile::tempdir().unwrap();
        let store = AssistantStore::new(ws.path());
        store
            .write_turns("older", &[Turn::now(TurnRole::User, "first topic")])
            .unwrap();
        store
            .write_turns("newer", &[Turn::now(TurnRole::User, "second topic")])
            .unwrap();
        store
            .set_active_conversation("newer", Some("Notes"), "default", None)
            .unwrap();

        let conversations = store.list_conversations().unwrap();
        assert_eq!(conversations.len(), 2);
        assert!(conversations
            .iter()
            .any(|item| item.id == "newer" && item.active));
        assert!(conversations
            .iter()
            .any(|item| item.id == "older" && !item.active));
    }

    #[test]
    fn checkpoint_preserves_raw_turns_and_history_metadata() {
        let ws = tempfile::tempdir().unwrap();
        let store = AssistantStore::new(ws.path());
        let turns = vec![
            Turn::now(TurnRole::User, "one"),
            Turn::now(TurnRole::Assistant, "two"),
        ];
        let checkpoint = store
            .write_checkpoint("conv", "rewind-safety", &turns)
            .unwrap();
        assert_eq!(
            store.load_checkpoint("conv", &checkpoint.id).unwrap(),
            turns
        );
        let history = store.load_history("conv").unwrap();
        assert_eq!(history.checkpoints.len(), 1);
        assert_eq!(history.checkpoints[0].turn_count, 2);
    }

    #[test]
    fn export_contains_transcript_and_audit_material() {
        let ws = tempfile::tempdir().unwrap();
        let store = AssistantStore::new(ws.path());
        let turns = vec![Turn::now(TurnRole::User, "hello")];
        let path = store
            .export_conversation("conv", &turns, "{\"kind\":\"tool_call\"}\n")
            .unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("# Assistant conversation conv"));
        assert!(raw.contains("hello"));
        assert!(raw.contains("tool_call"));
        assert!(path.starts_with(ws.path().join(crate::config::workspace_channel_dir())));
    }
}

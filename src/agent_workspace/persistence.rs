//! Last-CLI-per-repo persistence (#349).
//!
//! Tracks the most recently used `AgentCli` per repository so the modal
//! picker can pre-select the dropdown next time the user opens an Agent
//! Workspace from the same repo.
//!
//! File: `~/.plexi-<channel>/agent-workspaces.json`. Schema:
//! ```json
//! {
//!   "<repo_path>": {
//!     "last_cli": "claude_code" | "codex" | "gemini_cli",
//!     "last_used": <unix_seconds>
//!   }
//! }
//! ```
//!
//! Missing file is treated as "no history" — never an error. Parse failures
//! also degrade silently to empty so a corrupted record never blocks the
//! pane spawn.

use crate::agent_workspace::AgentCli;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LastCliEntry {
    pub last_cli: AgentCli,
    /// Unix-epoch seconds for "most recent spawn against this repo".
    pub last_used: u64,
}

/// Map from repo path → last-used CLI metadata.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LastCliMap {
    #[serde(flatten)]
    pub entries: HashMap<String, LastCliEntry>,
}

impl LastCliMap {
    pub fn lookup(&self, repo_path: &Path) -> Option<&LastCliEntry> {
        self.entries.get(&repo_path.to_string_lossy().into_owned())
    }

    pub fn record(&mut self, repo_path: &Path, cli: AgentCli) {
        let key = repo_path.to_string_lossy().into_owned();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        self.entries.insert(
            key,
            LastCliEntry {
                last_cli: cli,
                last_used: now,
            },
        );
    }

    /// Recent repos in last-used order (most recent first), capped at `limit`.
    pub fn recent_repos(&self, limit: usize) -> Vec<PathBuf> {
        let mut entries: Vec<(&String, &LastCliEntry)> = self.entries.iter().collect();
        entries.sort_by(|a, b| b.1.last_used.cmp(&a.1.last_used));
        entries
            .into_iter()
            .take(limit)
            .map(|(k, _)| PathBuf::from(k))
            .collect()
    }
}

/// Path to the persistence file. Channel-aware via `crate::config::config_dir`.
pub fn store_path() -> PathBuf {
    crate::config::config_dir().join("agent-workspaces.json")
}

/// Read the last-used map from disk. Missing file → default empty map.
/// Parse failures also return default + log a warning. Never returns an
/// error: a corrupted history is not a reason to block the modal.
pub fn load() -> LastCliMap {
    load_from(&store_path())
}

/// Write the last-used map to disk atomically (write-temp + rename). Logs on
/// failure; never panics.
pub fn save(map: &LastCliMap) {
    if let Err(e) = save_to(&store_path(), map) {
        log::warn!(
            "agent_workspace::persistence: save failed at {}: {e}",
            store_path().display()
        );
    }
}

// ── Test-injectable variants ────────────────────────────────────────────────

pub fn load_from(path: &Path) -> LastCliMap {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return LastCliMap::default(),
        Err(e) => {
            log::warn!(
                "agent_workspace::persistence: read failed at {}: {e}",
                path.display()
            );
            return LastCliMap::default();
        }
    };
    match serde_json::from_slice(&bytes) {
        Ok(map) => map,
        Err(e) => {
            log::warn!(
                "agent_workspace::persistence: parse failed at {}: {e} — starting empty",
                path.display()
            );
            LastCliMap::default()
        }
    }
}

pub fn save_to(path: &Path, map: &LastCliMap) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(map)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod last_cli_persistence_tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn round_trips_through_disk() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("agent-workspaces.json");

        let mut map = LastCliMap::default();
        map.record(Path::new("/Users/me/repo-a"), AgentCli::ClaudeCode);
        map.record(Path::new("/Users/me/repo-b"), AgentCli::Codex);
        save_to(&path, &map).unwrap();

        let restored = load_from(&path);
        assert_eq!(
            restored.lookup(Path::new("/Users/me/repo-a")).map(|e| e.last_cli),
            Some(AgentCli::ClaudeCode)
        );
        assert_eq!(
            restored.lookup(Path::new("/Users/me/repo-b")).map(|e| e.last_cli),
            Some(AgentCli::Codex)
        );
    }

    #[test]
    fn missing_file_treated_as_empty() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("does-not-exist.json");
        let map = load_from(&path);
        assert!(map.entries.is_empty());
    }

    #[test]
    fn corrupted_file_treated_as_empty() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("agent-workspaces.json");
        std::fs::write(&path, b"{not valid json").unwrap();
        let map = load_from(&path);
        assert!(map.entries.is_empty(), "corrupted file must degrade to empty");
    }

    #[test]
    fn record_updates_last_used_on_repeat_spawn() {
        let mut map = LastCliMap::default();
        map.record(Path::new("/r"), AgentCli::Codex);
        let first = map.lookup(Path::new("/r")).unwrap().last_used;
        // record again — last_used must be >= first (clock resolution may tie)
        map.record(Path::new("/r"), AgentCli::ClaudeCode);
        let second = map.lookup(Path::new("/r")).unwrap();
        assert_eq!(second.last_cli, AgentCli::ClaudeCode);
        assert!(second.last_used >= first);
    }

    #[test]
    fn recent_repos_returns_in_descending_order() {
        let mut map = LastCliMap::default();
        // Build entries with explicit timestamps to avoid clock-tie.
        map.entries.insert(
            "/a".to_string(),
            LastCliEntry {
                last_cli: AgentCli::ClaudeCode,
                last_used: 100,
            },
        );
        map.entries.insert(
            "/b".to_string(),
            LastCliEntry {
                last_cli: AgentCli::Codex,
                last_used: 200,
            },
        );
        map.entries.insert(
            "/c".to_string(),
            LastCliEntry {
                last_cli: AgentCli::GeminiCli,
                last_used: 150,
            },
        );

        let recent = map.recent_repos(10);
        assert_eq!(
            recent,
            vec![PathBuf::from("/b"), PathBuf::from("/c"), PathBuf::from("/a")]
        );
    }
}

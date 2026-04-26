use crate::agent_workspace::AgentCli;
use crate::tiling::PaneId;
use egui_tiles::{TileId, Tree};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::PathBuf;

#[derive(Serialize, Deserialize)]
pub struct WorkspaceFile {
    pub version: u32,
    pub active_context: usize,
    pub sidebar_visible: bool,
    pub next_pane_id: u64,
    pub contexts: Vec<SavedContext>,
}

#[derive(Serialize, Deserialize)]
pub struct SavedContext {
    pub name: String,
    pub path: PathBuf,
    pub tree: Tree<PaneId>,
    pub panes: Vec<SavedPane>,
    pub focused_pane: Option<TileId>,
}

#[derive(Serialize, Deserialize)]
pub struct SavedPane {
    pub id: u64,
    #[serde(default)]
    pub kind: SavedPaneKind,
    pub cwd: PathBuf,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub app_id: Option<String>,
    #[serde(default)]
    pub app_state: Option<serde_json::Value>,
    /// Agent Workspace (#348) — populated only when `kind == AgentWorkspace`.
    /// `None` for every other variant; restoring a non-AgentWorkspace pane
    /// ignores this field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_workspace: Option<SavedAgentWorkspace>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Default, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum SavedPaneKind {
    #[default]
    Terminal,
    App,
    Agent,
    AgentWorkspace,
}

/// Persisted state for `Pane::AgentWorkspace`. The worktree is NOT recreated
/// on restore — `worktree_path` already exists on disk; the PTY just relaunches
/// the CLI inside it. If the worktree was removed externally between runs the
/// PTY spawn will surface ENOENT in scrollback (#349 modal will pre-flight).
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct SavedAgentWorkspace {
    pub cli: AgentCli,
    pub repo_path: PathBuf,
    pub branch_name: String,
    pub worktree_path: PathBuf,
    pub task_label: String,
}

fn workspace_path() -> PathBuf {
    crate::config::config_dir()
        .join("workspaces")
        .join("default.json")
}

impl WorkspaceFile {
    pub fn save(&self) -> io::Result<()> {
        let path = workspace_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self).map_err(io::Error::other)?;
        std::fs::write(&path, json)
    }

    pub fn load() -> Option<Self> {
        let path = workspace_path();
        let data = match std::fs::read_to_string(&path) {
            Ok(d) => d,
            Err(_) => return None,
        };
        match serde_json::from_str(&data) {
            Ok(ws) => Some(ws),
            Err(e) => {
                log::warn!("Failed to parse workspace file: {e}");
                let backup = path.with_extension(format!(
                    "backup-{}.json",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0)
                ));
                let _ = std::fs::rename(&path, &backup);
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `SavedPaneKind` must serialise every Pane variant produced by mirror-split
    /// (Cmd+N / Cmd+Shift+N): Terminal, App, Agent. Round-tripping through JSON
    /// preserves the kind so a workspace saved after a split restores the same
    /// pane type on next launch.
    #[test]
    fn workspace_save_restore_preserves_split_panes() {
        let kinds = [
            SavedPaneKind::Terminal,
            SavedPaneKind::App,
            SavedPaneKind::Agent,
        ];
        for kind in kinds {
            let pane = SavedPane {
                id: 42,
                kind,
                cwd: PathBuf::from("/tmp"),
                name: Some("split-pane".to_string()),
                app_id: matches!(kind, SavedPaneKind::App)
                    .then(|| "snake".to_string()),
                app_state: None,
                agent_workspace: None,
            };
            let json = serde_json::to_string(&pane).expect("serialize");
            let restored: SavedPane = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(restored.kind, kind, "kind {kind:?} must round-trip");
            assert_eq!(restored.id, 42);
            assert_eq!(restored.cwd, PathBuf::from("/tmp"));
        }
    }

    /// #348 — Agent Workspace pane round-trips through workspace persistence:
    /// kind, CLI, repo path, branch name, worktree path, task label all
    /// preserved across serialize/deserialize.
    #[test]
    fn saved_agent_workspace_round_trips_through_persistence() {
        let pane = SavedPane {
            id: 7,
            kind: SavedPaneKind::AgentWorkspace,
            cwd: PathBuf::from("/repo/.git/worktrees/plexi-abcd1234"),
            name: None,
            app_id: None,
            app_state: None,
            agent_workspace: Some(SavedAgentWorkspace {
                cli: AgentCli::ClaudeCode,
                repo_path: PathBuf::from("/repo"),
                branch_name: "plexi/agent-fix-bug-abcd1234".to_string(),
                worktree_path: PathBuf::from("/repo/.git/worktrees/plexi-abcd1234"),
                task_label: "fix the auth bug".to_string(),
            }),
        };
        let json = serde_json::to_string(&pane).expect("serialize");
        let restored: SavedPane = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(restored.kind, SavedPaneKind::AgentWorkspace);
        let aw = restored.agent_workspace.expect("agent_workspace must round-trip");
        assert_eq!(aw.cli, AgentCli::ClaudeCode);
        assert_eq!(aw.repo_path, PathBuf::from("/repo"));
        assert_eq!(aw.branch_name, "plexi/agent-fix-bug-abcd1234");
        assert_eq!(
            aw.worktree_path,
            PathBuf::from("/repo/.git/worktrees/plexi-abcd1234")
        );
        assert_eq!(aw.task_label, "fix the auth bug");
    }

    /// Regression guard: existing variants still round-trip after the new
    /// optional field was added (`agent_workspace: Option<...>` must not
    /// break legacy saved-workspace files that omit it).
    #[test]
    fn existing_round_trip_test_still_passes_after_variant_added() {
        // Synthesize a JSON blob without the new field — simulates a
        // workspace.json written by a prior Plexi version.
        let legacy_json = r#"{
            "id": 1,
            "kind": "terminal",
            "cwd": "/tmp"
        }"#;
        let restored: SavedPane = serde_json::from_str(legacy_json)
            .expect("legacy SavedPane without agent_workspace must deserialize");
        assert_eq!(restored.kind, SavedPaneKind::Terminal);
        assert!(restored.agent_workspace.is_none());
    }
}

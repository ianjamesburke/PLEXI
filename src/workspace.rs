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
}

#[derive(Serialize, Deserialize, Clone, Copy, Default, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum SavedPaneKind {
    #[default]
    Terminal,
    App,
    Agent,
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
            };
            let json = serde_json::to_string(&pane).expect("serialize");
            let restored: SavedPane = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(restored.kind, kind, "kind {kind:?} must round-trip");
            assert_eq!(restored.id, 42);
            assert_eq!(restored.cwd, PathBuf::from("/tmp"));
        }
    }
}

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
    pub cwd: PathBuf,
    #[serde(default)]
    pub name: Option<String>,
    /// If this pane had an active app, its type_id (e.g. "file_browser").
    #[serde(default)]
    pub active_app_type: Option<String>,
    /// Serialised app state for restoration.
    #[serde(default)]
    pub active_app_state: Option<serde_json::Value>,
    /// PaneId of the linked terminal (for split restoration).
    #[serde(default)]
    pub linked_terminal_pane: Option<u64>,
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
        let json = serde_json::to_string_pretty(self)
            .map_err(io::Error::other)?;
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

    #[test]
    fn workspace_round_trip() {
        let ws = WorkspaceFile {
            version: 1,
            active_context: 2,
            sidebar_visible: false,
            next_pane_id: 42,
            contexts: vec![SavedContext {
                name: "Test Context".into(),
                path: PathBuf::from("/tmp/test"),
                tree: {
                    let mut tiles = egui_tiles::Tiles::default();
                    let root = tiles.insert_pane(1u64);
                    Tree::new("test", root, tiles)
                },
                panes: vec![
                    SavedPane {
                        id: 1,
                        cwd: PathBuf::from("/tmp"),
                        name: Some("my-pane".into()),
                        active_app_type: None,
                        active_app_state: None,
                        linked_terminal_pane: None,
                    },
                    SavedPane {
                        id: 2,
                        cwd: PathBuf::from("/home"),
                        name: None,
                        active_app_type: None,
                        active_app_state: None,
                        linked_terminal_pane: None,
                    },
                ],
                focused_pane: None,
            }],
        };

        let json = serde_json::to_string(&ws).unwrap();
        let restored: WorkspaceFile = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.version, 1);
        assert_eq!(restored.active_context, 2);
        assert!(!restored.sidebar_visible);
        assert_eq!(restored.next_pane_id, 42);
        assert_eq!(restored.contexts.len(), 1);
        assert_eq!(restored.contexts[0].name, "Test Context");
        assert_eq!(restored.contexts[0].panes.len(), 2);
        assert_eq!(restored.contexts[0].panes[0].name, Some("my-pane".into()));
        assert_eq!(restored.contexts[0].panes[1].name, None);
    }
}

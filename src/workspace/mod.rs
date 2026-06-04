pub mod router;
pub mod secrets;

use crate::host::context::Context;
use crate::spatial::tiling::PaneId;
use egui_tiles::{TileId, Tree};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io;
use std::path::PathBuf;

#[derive(Serialize, Deserialize)]
pub struct WorkspaceFile {
    pub version: u32,
    /// Active context index (sidebar item).
    pub active_context: usize,
    pub sidebar_visible: bool,
    pub next_pane_id: u64,
    pub contexts: Vec<Context>,
    pub windows: Vec<SavedWindow>,
    /// context_id → last active window_id for that context.
    #[serde(default)]
    pub context_active_window: HashMap<u64, u64>,
}

#[derive(Serialize, Deserialize)]
pub struct SavedWindow {
    pub name: String,
    pub path: PathBuf,
    pub tree: Tree<PaneId>,
    pub panes: Vec<SavedPane>,
    pub focused_pane: Option<TileId>,
    #[serde(default)]
    pub grid_x: u32,
    #[serde(default)]
    pub grid_y: u32,
    #[serde(default)]
    pub window_id: u64,
    #[serde(default)]
    pub context_id: u64,
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
    #[serde(default)]
    pub hidden: bool,
}

#[derive(Serialize, Deserialize, Clone, Default, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum SavedPaneKind {
    #[default]
    Terminal,
    App,
    #[serde(alias = "sub_context")]
    Portal { context_id: u64 },
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
        let ws: Self = match serde_json::from_str(&data) {
            Ok(ws) => ws,
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
                return None;
            }
        };
        if ws.version != 2 {
            log::info!("Ignoring old workspace file version {}; starting fresh", ws.version);
            let backup = path.with_extension(format!(
                "backup-v{}-{}.json",
                ws.version,
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0)
            ));
            let _ = std::fs::rename(&path, &backup);
            return None;
        }
        Some(ws)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `SavedPaneKind` must serialise every Pane variant produced by mirror-split
    /// (Cmd+N / Cmd+Shift+N): Terminal, App. Round-tripping through JSON
    /// preserves the kind so a workspace saved after a split restores the same
    /// pane type on next launch.
    #[test]
    fn workspace_save_restore_preserves_split_panes() {
        let kinds = [SavedPaneKind::Terminal, SavedPaneKind::App];
        for kind in kinds {
            let pane = SavedPane {
                id: 42,
                kind: kind.clone(),
                cwd: PathBuf::from("/tmp"),
                name: Some("split-pane".to_string()),
                app_id: matches!(kind, SavedPaneKind::App)
                    .then(|| "snake".to_string()),
                app_state: None,
                hidden: false,
            };
            let json = serde_json::to_string(&pane).expect("serialize");
            let restored: SavedPane = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(restored.kind, kind, "kind {kind:?} must round-trip");
            assert_eq!(restored.id, 42);
            assert_eq!(restored.cwd, PathBuf::from("/tmp"));
        }
        // Portal round-trips with embedded context_id.
        let portal_pane = SavedPane {
            id: 99,
            kind: SavedPaneKind::Portal { context_id: 42 },
            cwd: PathBuf::new(),
            name: None,
            app_id: None,
            app_state: None,
            hidden: false,
        };
        let json = serde_json::to_string(&portal_pane).expect("serialize portal");
        let restored: SavedPane = serde_json::from_str(&json).expect("deserialize portal");
        assert!(
            matches!(restored.kind, SavedPaneKind::Portal { context_id: 42 }),
            "Portal kind must round-trip with correct context_id"
        );
        assert_eq!(restored.id, 99);

        // Backward compat: old "sub_context" JSON still deserializes to Portal.
        let legacy_json = r#"{"id":99,"kind":{"sub_context":{"context_id":42}},"cwd":"","name":null,"app_id":null,"app_state":null}"#;
        let legacy: SavedPane = serde_json::from_str(legacy_json).expect("deserialize legacy sub_context");
        assert!(
            matches!(legacy.kind, SavedPaneKind::Portal { context_id: 42 }),
            "legacy sub_context must deserialize to Portal"
        );
    }

    /// SavedWindow omits `grid_x` / `grid_y` defaults cleanly.
    #[test]
    fn grid_coords_default_to_zero_on_load() {
        use egui_tiles::Tree;

        let tree: Tree<PaneId> = Tree::empty("test");
        let tree_json = serde_json::to_string(&tree).expect("tree must serialize");

        let json = format!(
            r#"{{
                "name": "Default",
                "path": "/tmp",
                "tree": {},
                "panes": [],
                "focused_pane": null
            }}"#,
            tree_json
        );

        let restored: SavedWindow = serde_json::from_str(&json)
            .expect("SavedWindow without grid_x/grid_y must deserialize");
        assert_eq!(restored.grid_x, 0, "grid_x must default to 0");
        assert_eq!(restored.grid_y, 0, "grid_y must default to 0");
    }

    /// Legacy workspace files without optional fields still deserialize cleanly.
    #[test]
    fn legacy_saved_pane_deserializes_cleanly() {
        let legacy_json = r#"{
            "id": 1,
            "kind": "terminal",
            "cwd": "/tmp"
        }"#;
        let restored: SavedPane = serde_json::from_str(legacy_json)
            .expect("legacy SavedPane must deserialize");
        assert_eq!(restored.kind, SavedPaneKind::Terminal);
    }

    /// Context (formerly SavedContext) round-trips through JSON and handles
    /// legacy files missing optional fields (root, description, parent_id, depth).
    #[test]
    fn context_serde_round_trip() {
        let ctx = Context {
            name: "dev".to_string(),
            path: PathBuf::from("/projects/dev"),
            root: Some(PathBuf::from("/projects/dev/src")),
            description: Some("main workspace".to_string()),
            context_id: 42,
            parent_id: Some(1),
            depth: 1,
            parked: false,
        };
        let json = serde_json::to_string(&ctx).expect("serialize");
        let restored: Context = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(restored.name, "dev");
        assert_eq!(restored.context_id, 42);
        assert_eq!(restored.parent_id, Some(1));
        assert_eq!(restored.depth, 1);
        assert_eq!(restored.root, Some(PathBuf::from("/projects/dev/src")));
    }

    /// Legacy workspace JSON without optional Context fields deserializes with defaults.
    #[test]
    fn legacy_context_without_optional_fields_deserializes() {
        let legacy_json = r#"{
            "name": "old-ctx",
            "path": "/tmp",
            "context_id": 7
        }"#;
        let restored: Context = serde_json::from_str(legacy_json)
            .expect("legacy Context must deserialize");
        assert_eq!(restored.name, "old-ctx");
        assert_eq!(restored.context_id, 7);
        assert_eq!(restored.parent_id, None);
        assert_eq!(restored.depth, 0);
        assert_eq!(restored.root, None);
        assert_eq!(restored.description, None);
    }
}

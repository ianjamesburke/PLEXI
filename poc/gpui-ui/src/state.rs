#[derive(Clone, Debug)]
pub struct PaneState {
    pub id: usize,
    pub name: String,
    pub kind: PaneKind,
    pub status: PaneStatus,
    pub cwd: String,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PaneKind {
    Terminal,
    App,
    Agent,
}

impl PaneKind {
    pub fn label(&self) -> &'static str {
        match self {
            PaneKind::Terminal => "term",
            PaneKind::App => "app",
            PaneKind::Agent => "agent",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PaneStatus {
    Idle,
    Running,
    Busy,
    Error,
    Exited,
}

impl PaneStatus {
    pub fn label(&self) -> &'static str {
        match self {
            PaneStatus::Idle => "idle",
            PaneStatus::Running => "running",
            PaneStatus::Busy => "busy",
            PaneStatus::Error => "error",
            PaneStatus::Exited => "exited",
        }
    }

    pub fn is_active(&self) -> bool {
        matches!(self, PaneStatus::Running | PaneStatus::Busy)
    }
}

#[derive(Clone, Debug)]
pub struct ContextInfo {
    pub id: usize,
    pub name: String,
    pub cwd: String,
    pub pane_ids: Vec<usize>,
    pub children: Vec<ContextInfo>,
}

/// Recursive tiling layout — mirrors Plexi's egui_tiles tree.
#[derive(Clone, Debug)]
pub enum TileLayout {
    Leaf(usize),
    HSplit { ratio: f32, left: Box<TileLayout>, right: Box<TileLayout> },
    VSplit { ratio: f32, top: Box<TileLayout>, bottom: Box<TileLayout> },
}

impl TileLayout {
    pub fn leaf_ids(&self) -> Vec<usize> {
        match self {
            TileLayout::Leaf(id) => vec![*id],
            TileLayout::HSplit { left, right, .. } => {
                let mut ids = left.leaf_ids();
                ids.extend(right.leaf_ids());
                ids
            }
            TileLayout::VSplit { top, bottom, .. } => {
                let mut ids = top.leaf_ids();
                ids.extend(bottom.leaf_ids());
                ids
            }
        }
    }

    pub fn contains(&self, id: usize) -> bool {
        match self {
            TileLayout::Leaf(leaf_id) => *leaf_id == id,
            TileLayout::HSplit { left, right, .. } => left.contains(id) || right.contains(id),
            TileLayout::VSplit { top, bottom, .. } => top.contains(id) || bottom.contains(id),
        }
    }

    /// Remove a leaf by id. If the leaf is one side of a split, the other side
    /// collapses up. If the leaf isn't found, returns self unchanged.
    pub fn remove_leaf(self, target: usize) -> TileLayout {
        match self {
            TileLayout::Leaf(id) => {
                if id == target {
                    // Caller must ensure they don't remove the last pane;
                    // return a placeholder leaf 0 as a safe fallback.
                    TileLayout::Leaf(0)
                } else {
                    TileLayout::Leaf(id)
                }
            }
            TileLayout::HSplit { ratio, left, right } => {
                if matches!(&*left, TileLayout::Leaf(id) if *id == target) {
                    right.remove_leaf(target)
                } else if matches!(&*right, TileLayout::Leaf(id) if *id == target) {
                    left.remove_leaf(target)
                } else {
                    TileLayout::HSplit {
                        ratio,
                        left: Box::new(left.remove_leaf(target)),
                        right: Box::new(right.remove_leaf(target)),
                    }
                }
            }
            TileLayout::VSplit { ratio, top, bottom } => {
                if matches!(&*top, TileLayout::Leaf(id) if *id == target) {
                    bottom.remove_leaf(target)
                } else if matches!(&*bottom, TileLayout::Leaf(id) if *id == target) {
                    top.remove_leaf(target)
                } else {
                    TileLayout::VSplit {
                        ratio,
                        top: Box::new(top.remove_leaf(target)),
                        bottom: Box::new(bottom.remove_leaf(target)),
                    }
                }
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct NotificationState {
    pub title: String,
    pub body: String,
    pub source_pane: Option<usize>,
    pub dismiss_at: Option<std::time::Instant>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ActiveOverlay {
    CommandPalette,
    ContextInspector,
    Help,
    QuickNote,
}

/// A single entry in the command palette.
#[derive(Clone, Debug)]
pub struct CommandEntry {
    pub id: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub keys: Vec<&'static str>,
    pub category: &'static str,
}

pub fn all_commands() -> Vec<CommandEntry> {
    vec![
        CommandEntry { id: "new_terminal", label: "New Terminal Pane", description: "Open a new terminal in current context", keys: vec!["⌘", "T"], category: "Panes" },
        CommandEntry { id: "split_h", label: "Split Horizontal", description: "Split active pane horizontally", keys: vec!["⌘", "-"], category: "Panes" },
        CommandEntry { id: "split_v", label: "Split Vertical", description: "Split active pane vertically", keys: vec!["⌘", "\\"], category: "Panes" },
        CommandEntry { id: "close_pane", label: "Close Pane", description: "Close the active pane", keys: vec!["⌘", "W"], category: "Panes" },
        CommandEntry { id: "focus_next", label: "Focus Next Pane", description: "Move focus to the next pane", keys: vec!["⌘", "]"], category: "Navigation" },
        CommandEntry { id: "focus_prev", label: "Focus Previous Pane", description: "Move focus to the previous pane", keys: vec!["⌘", "["], category: "Navigation" },
        CommandEntry { id: "zoom_pane", label: "Zoom Pane", description: "Toggle pane zoom (fullscreen)", keys: vec!["⌘", "Z"], category: "Navigation" },
        CommandEntry { id: "new_context", label: "New Context", description: "Create a new context", keys: vec!["⌘", "N"], category: "Contexts" },
        CommandEntry { id: "rename_context", label: "Rename Context", description: "Rename the active context", keys: vec!["⌘", "⇧", "R"], category: "Contexts" },
        CommandEntry { id: "next_context", label: "Next Context", description: "Switch to the next context", keys: vec!["⌘", "⇥"], category: "Contexts" },
        CommandEntry { id: "quick_note", label: "Quick Note", description: "Open the quick note input", keys: vec!["⌘", "0"], category: "Tools" },
        CommandEntry { id: "context_inspector", label: "Context Inspector", description: "Open the context inspector", keys: vec!["⌘", "I"], category: "Tools" },
        CommandEntry { id: "open_app", label: "Open App", description: "Launch a Plexi app by name", keys: vec!["⌘", "O"], category: "Apps" },
        CommandEntry { id: "settings", label: "Open Settings", description: "Open config.toml in an editor", keys: vec![], category: "Settings" },
        CommandEntry { id: "install_update", label: "Install Update", description: "Run plexi update", keys: vec![], category: "Settings" },
    ]
}

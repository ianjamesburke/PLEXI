use crate::tiling::PaneId;

pub type Capability = String;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShareRatio {
    pub numerator: f32,
    pub denominator: f32,
}

impl ShareRatio {
    pub fn new(numerator: f32, denominator: f32) -> Option<Self> {
        if numerator > 0.0 && denominator > 0.0 {
            Some(Self { numerator, denominator })
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    Below,
    Right,
    ReplaceFocused,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaneRuntimeKind {
    Terminal,
    App { app_id: String },
    Agent,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OpenPaneRequest {
    pub runtime: PaneRuntimeKind,
    pub placement: Placement,
    pub share: ShareRatio,
    /// Pane group name for PathChanged broadcasts. None = ungrouped.
    pub group: Option<String>,
    /// Capabilities declared in the app's manifest.toml.
    pub declared_capabilities: Vec<Capability>,
}

impl OpenPaneRequest {
    pub fn terminal() -> Self {
        Self {
            runtime: PaneRuntimeKind::Terminal,
            placement: Placement::Right,
            share: ShareRatio::new(1.0, 1.0).expect("1:1 is valid"),
            group: None,
            declared_capabilities: Vec::new(),
        }
    }

    pub fn app(app_id: impl Into<String>) -> Self {
        Self {
            runtime: PaneRuntimeKind::App { app_id: app_id.into() },
            placement: Placement::Right,
            share: ShareRatio::new(1.0, 1.0).expect("1:1 is valid"),
            group: None,
            declared_capabilities: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum HostCommand {
    /// Create a new pane (Terminal, App, or Agent).
    OpenPane(OpenPaneRequest),
    /// Close the pane that currently has focus.
    CloseFocusedPane,
    /// Give focus to a specific pane by ID.
    FocusPane(PaneId),
    /// Move focus directionally within the active context.
    Navigate(Direction),
    /// Split focused pane: new Terminal pane below.
    SplitHorizontal,
    /// Split focused pane: new Terminal pane to the right.
    SplitVertical,
    /// Create a new workspace context (tab).
    NewContext,
    /// Switch to a workspace context by index.
    SwitchContext(usize),
    /// Route a key event to the focused App pane.
    SendKeyToFocusedApp { key: String },
    /// Trigger pane-group broadcast (test seam and PGAP relay).
    SimulatePathChanged { pane_id: PaneId, cwd: String },
    /// Check whether a pane has a capability (triggers prompt if unresolved).
    CheckCapability { pane_id: PaneId, capability: Capability },
    /// Record that the user approved a capability prompt.
    GrantCapability { pane_id: PaneId, capability: Capability },
    /// Record that the user denied a capability prompt.
    DenyCapability { pane_id: PaneId, capability: Capability },
}

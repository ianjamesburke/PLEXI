use serde::Serialize;

pub use crate::keys::Direction;

pub type Capability = String;

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Placement {
    Below,
    Right,
}

/// I-5 (`docs/specs/releases/plexi-v3.0.md §2`): the pane ADT is frozen
/// at these 2 variants for v3.0. New pane-shaped things are PGAP apps,
/// not new variants. Changing this enum requires a spec amendment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PaneRuntimeKind {
    Terminal,
    App { app_id: String },
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


#[derive(Debug, Clone, PartialEq)]
pub enum HostCommand {
    /// Create a new pane (Terminal or App).
    OpenPane(OpenPaneRequest),
    /// Close the pane that currently has focus.
    CloseFocusedPane,
    /// Move focus directionally within the active context.
    Navigate(Direction),
    /// Split focused pane: new Terminal pane below.
    SplitHorizontal,
    /// Split focused pane: new Terminal pane to the right.
    SplitVertical,
}

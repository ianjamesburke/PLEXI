use crate::tiling::PaneId;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShareRatio {
    pub numerator: f32,
    pub denominator: f32,
}

impl ShareRatio {
    pub fn new(numerator: f32, denominator: f32) -> Option<Self> {
        if numerator > 0.0 && denominator > 0.0 {
            Some(Self {
                numerator,
                denominator,
            })
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
}

#[derive(Debug, Clone, PartialEq)]
pub enum HostCommand {
    OpenPane(OpenPaneRequest),
    CloseFocusedPane,
    FocusPane(PaneId),
    FocusNext,
    FocusPrev,
}

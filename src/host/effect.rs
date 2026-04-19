use crate::host::command::{Capability, Placement, PaneRuntimeKind, ShareRatio};
use crate::tiling::PaneId;
use serde::Serialize;

/// A minimal host event for the event bus. Grows as the bus is wired up.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HostEvent {
    PaneOpened { pane_id: PaneId },
    PaneClosed { pane_id: PaneId },
    CapabilityDecided { pane_id: PaneId, capability: Capability, granted: bool },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "effect", rename_all = "snake_case")]
pub enum HostEffect {
    PaneOpened { pane_id: PaneId, kind: PaneRuntimeKind, share: ShareRatio, placement: Placement },
    PaneClosed { pane_id: PaneId },
    FocusChanged { pane_id: Option<PaneId> },
    SplitOpened { pane_id: PaneId, kind: PaneRuntimeKind, placement: Placement },
    ContextCreated { index: usize },
    ContextSwitched { index: usize },
    AppKeyDispatched { pane_id: PaneId, key: String },
    PathBroadcasted { group: String, cwd: String, recipient_pane_ids: Vec<PaneId> },
    CapabilityGranted { pane_id: PaneId, capability: Capability },
    CapabilityDenied { pane_id: PaneId, capability: Capability },
    CapabilityPromptRequired { pane_id: PaneId, capability: Capability },
    EventEmitted(HostEvent),
}

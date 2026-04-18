use crate::host::command::PaneRuntimeKind;
use crate::tiling::PaneId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostEffect {
    PaneOpened {
        pane_id: PaneId,
        kind: PaneRuntimeKind,
    },
    PaneClosed {
        pane_id: PaneId,
    },
    FocusChanged {
        pane_id: Option<PaneId>,
    },
}

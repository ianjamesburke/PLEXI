use crate::tiling::PaneId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusTarget {
    Pane(PaneId),
    Overlay(OverlayId),
    CommandPalette,
    RunPalette,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverlayId(pub u64);

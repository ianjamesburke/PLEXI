//! Thin wrapper that delegates to the shipped #288 agent turn loop in
//! `crate::agent_pane::render_and_drain`.
//!
//! Do not reimplement agent logic here. This module exists only so the tiling
//! render dispatcher can call a `render::agent_pane::render(...)` entry point
//! alongside the terminal and app renderers.

use crate::pane::AgentPane;
use crate::theme::Colors;

/// Render an agent pane for one frame.
///
/// Returns `true` if background work produced new output and the UI should
/// repaint on the next frame.
pub fn render(ui: &mut egui::Ui, pane: &mut AgentPane, colors: &Colors) -> bool {
    crate::agent_pane::render_and_drain(ui, pane, colors)
}

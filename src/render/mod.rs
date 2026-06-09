//! Per-pane-type render entry points used by `tiling::PlexiBehavior::pane_ui`.
//!
//! Each submodule extracts the body that used to live inline in `tiling.rs`
//! so the `Behavior` impl stays focused on layout/focus/dispatch.
//!
//! The outer `pane_ui` path is responsible for painting the pane background
//! and shrinking into the inner UI — these renderers operate on the inner UI
//! and should not repaint the background.

pub mod app_pane;
pub mod app_render;
pub mod cli_renderer_app;
pub mod components;
pub mod headless_renderer;
pub mod minimap;
pub mod terminal_pane;

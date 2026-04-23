//! App pane render path — extracted from `tiling::PlexiBehavior::pane_ui`.
//!
//! Builds the `AppRenderContext` and delegates to the app runtime's `ui`
//! method. The outer `pane_ui` path already painted the pane background and
//! shrunk into the inner UI, so this renderer does not repaint.

use crate::app_trait::AppRenderContext;
use crate::pane::AppPane;
use crate::theme::Colors;

pub fn render(ui: &mut egui::Ui, app_pane: &mut AppPane, colors: &Colors, is_focused: bool) {
    let ctx = AppRenderContext { colors, is_focused };
    app_pane.runtime.ui(ui, &ctx);
}

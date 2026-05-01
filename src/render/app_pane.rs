//! App pane render path — extracted from `tiling::PlexiBehavior::pane_ui`.
//!
//! Builds the `AppRenderContext` and delegates to the app runtime's `ui`
//! method. When the app has a non-empty navigation stack, a slim nav bar is
//! rendered above the app content showing the current view title and a back
//! arrow (‹). Clicking the back arrow emits `PlexiEvent::NavBack` to the app.

use crate::app_protocol::PlexiEvent;
use crate::app_trait::AppRenderContext;
use crate::pane::AppPane;
use crate::style;
use crate::theme::Colors;
use egui::RichText;

/// Height of the nav bar shown when the app has pushed at least one view.
const NAV_BAR_HEIGHT: f32 = 32.0;
/// Horizontal padding inside the nav bar.
const NAV_BAR_PAD: f32 = style::SPACE_SM;

pub fn render(ui: &mut egui::Ui, app_pane: &mut AppPane, colors: &Colors, is_focused: bool) {
    // Check if we need a nav bar (non-empty stack).
    let nav_title: Option<String> = app_pane
        .runtime
        .nav_top_title()
        .map(|t| t.to_owned());

    if let Some(title) = nav_title {
        // Render nav bar at the top, then app content below.
        let nav_back_view_id = app_pane.runtime.nav_back_view_id();

        ui.vertical(|ui| {
            let bar_rect = egui::Rect::from_min_size(
                ui.cursor().left_top(),
                egui::vec2(ui.available_width(), NAV_BAR_HEIGHT),
            );

            // Dim background for the nav bar to distinguish it from app content.
            ui.painter().rect_filled(bar_rect, 0.0, colors.bg_active);

            ui.allocate_ui_at_rect(bar_rect, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.add_space(NAV_BAR_PAD);

                    // Back arrow button — ‹ (single angle quotation mark, U+2039)
                    let back_btn = ui.add(
                        egui::Button::new(
                            RichText::new("‹")
                                .size(style::TEXT_BODY + 2.0)
                                .color(colors.accent),
                        )
                        .frame(false),
                    );
                    if back_btn.clicked() {
                        app_pane.runtime.queue_outbound_event(
                            PlexiEvent::NavBack { view_id: nav_back_view_id },
                        );
                    }

                    ui.add_space(4.0);

                    // Current view title — truncate with ellipsis if needed.
                    ui.label(
                        RichText::new(&title)
                            .size(style::TEXT_CAPTION)
                            .color(colors.text_dim),
                    );
                });
            });

            // App content fills remaining space.
            let ctx = AppRenderContext { colors, is_focused };
            app_pane.runtime.ui(ui, &ctx);
        });
    } else {
        // No nav stack — render app content directly (hot path, no allocation).
        let ctx = AppRenderContext { colors, is_focused };
        app_pane.runtime.ui(ui, &ctx);
    }
}

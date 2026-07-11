//! App pane render path — extracted from `tiling::PlexiBehavior::pane_ui`.
//!
//! Chrome bands (overtake bar, nav bar) are allocated in the parent layout via
//! `allocate_exact_size`, advancing the cursor before the app's `ui()` method
//! is called. This guarantees that `ui.available_height()` inside every app
//! runtime equals `pane_height - visible_chrome_height`, regardless of which
//! chrome bands are active.

use crate::app::app_trait::AppRenderContext;
use crate::app_protocol::PlexiEvent;
use crate::host::pane::AppPane;
use crate::ui::theme::Colors;
use crate::ui::{button, style};
use egui::RichText;

/// Height of the nav bar shown when the app has pushed at least one view.
const NAV_BAR_HEIGHT: f32 = 32.0;
/// Horizontal padding inside the nav bar.
const NAV_BAR_PAD: f32 = style::SPACE_SM;
/// Height of the overtake indicator bar shown when this app replaced another pane.
const OVERTAKE_BAR_HEIGHT: f32 = 24.0;

/// Total height consumed by visible host chrome bands.
pub(crate) fn chrome_height(has_overtake: bool, has_nav: bool) -> f32 {
    (if has_overtake {
        OVERTAKE_BAR_HEIGHT
    } else {
        0.0
    }) + (if has_nav { NAV_BAR_HEIGHT } else { 0.0 })
}

pub fn render(
    ui: &mut egui::Ui,
    app_pane: &mut AppPane,
    colors: &Colors,
    is_focused: bool,
    suppress_overtake: bool,
) {
    let has_overtake = app_pane.overlay_replaced.is_some() && !suppress_overtake;
    let nav_title: Option<String> = app_pane.runtime.nav_top_title().map(|t| t.to_owned());
    // Resolve before any mutable borrow of app_pane.runtime inside closures below.
    let nav_back_view_id: String = if nav_title.is_some() {
        app_pane.runtime.nav_back_view_id()
    } else {
        String::new()
    };

    if has_overtake || nav_title.is_some() {
        log::debug!(
            "app_pane::render: chrome_h={:.0} (overtake={has_overtake} nav={})",
            chrome_height(has_overtake, nav_title.is_some()),
            nav_title.is_some(),
        );
    }

    // -- Overtake bar --
    // allocate_exact_size advances the parent cursor so the app content viewport shrinks.
    if has_overtake {
        let (bar_rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), OVERTAKE_BAR_HEIGHT),
            egui::Sense::hover(),
        );
        ui.painter().rect_filled(bar_rect, 0.0, colors.bg_active);
        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(bar_rect), |ui| {
            ui.horizontal_centered(|ui| {
                ui.add_space(NAV_BAR_PAD);
                let replaced_label = match app_pane.overlay_replaced.as_deref() {
                    Some(crate::host::pane::Pane::Terminal(t)) => {
                        t.name.as_deref().unwrap_or("Terminal").to_string()
                    }
                    Some(crate::host::pane::Pane::App(a)) => a.name.clone(),
                    Some(crate::host::pane::Pane::Portal(_)) => "Portal".to_string(),
                    None => unreachable!(),
                };
                ui.label(
                    RichText::new(format!("← {replaced_label}"))
                        .size(style::TEXT_HINT)
                        .color(colors.text_dim),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(NAV_BAR_PAD);
                    crate::ui::shortcuts::key_chip(
                        ui,
                        "Esc",
                        colors,
                        egui::FontId::monospace(crate::ui::style::TEXT_CAPTION),
                    );
                    ui.label(
                        RichText::new("return")
                            .size(style::TEXT_HINT)
                            .color(colors.text_dim),
                    );
                });
            });
        });
    }

    // -- Nav bar --
    // Same pattern: allocate_exact_size advances cursor, allocate_new_ui draws inside.
    if let Some(title) = &nav_title {
        let (bar_rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), NAV_BAR_HEIGHT),
            egui::Sense::hover(),
        );
        ui.painter().rect_filled(bar_rect, 0.0, colors.bg_active);
        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(bar_rect), |ui| {
            ui.horizontal_centered(|ui| {
                ui.add_space(NAV_BAR_PAD);
                // Back arrow button — ‹ (single angle quotation mark, U+2039)
                let back_btn = button::toolbar_button(
                    ui,
                    RichText::new("‹")
                        .size(style::TEXT_BODY + 2.0)
                        .color(colors.accent),
                    "Back",
                );
                if back_btn.clicked() {
                    app_pane.runtime.queue_outbound_event(PlexiEvent::NavBack {
                        view_id: nav_back_view_id.clone(),
                    });
                }
                ui.add_space(4.0);
                ui.label(
                    RichText::new(title.as_str())
                        .size(style::TEXT_CAPTION)
                        .color(colors.text_dim),
                );
            });
        });
    }

    // -- App content in constrained content viewport --
    // ui.available_height() == pane_height - chrome_height(has_overtake, nav_title.is_some())
    // guaranteed by the allocate_exact_size calls above. Apps read ui.available_height() /
    // available_size() and always see the post-chrome budget with no guesswork.
    let ctx = AppRenderContext { colors, is_focused };
    let content_rect = ui.available_rect_before_wrap();
    app_pane.runtime.ui(ui, &ctx);
    if matches!(app_pane.runtime, crate::host::pane::AppRuntime::Builtin(_)) {
        if let Some(state) = ui.ctx().viewport(|viewport| {
            viewport
                .this_pass
                .accesskit_state
                .as_ref()
                .map(|accesskit| {
                    crate::host::pane::SemanticPaneState::from_accesskit(
                        &accesskit.nodes,
                        content_rect,
                    )
                })
        }) {
            app_pane.semantic_state = state;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chrome_height_no_chrome() {
        assert_eq!(chrome_height(false, false), 0.0);
    }

    #[test]
    fn chrome_height_overtake_only() {
        assert_eq!(chrome_height(true, false), OVERTAKE_BAR_HEIGHT);
    }

    #[test]
    fn chrome_height_nav_only() {
        assert_eq!(chrome_height(false, true), NAV_BAR_HEIGHT);
    }

    #[test]
    fn chrome_height_both_bars() {
        assert_eq!(
            chrome_height(true, true),
            OVERTAKE_BAR_HEIGHT + NAV_BAR_HEIGHT
        );
    }
}

//! View pane render path — extracted from `tiling::PlexiBehavior::pane_ui`.
//!
//! Chrome bands (overtake bar, nav bar) are allocated in the parent layout via
//! `allocate_exact_size`, advancing the cursor before the plugin's `ui()` method
//! is called. This guarantees that `ui.available_height()` inside every plugin
//! runtime equals `pane_height - visible_chrome_height`, regardless of which
//! chrome bands are active.

use crate::app::view_trait::ViewRenderContext;
use crate::host::pane::ViewPane;
use crate::protocol::PlexiEvent;
use crate::ui::theme::Colors;
use crate::ui::{button, style};
use egui::RichText;

/// Height of the nav bar shown when the plugin has pushed at least one view.
const NAV_BAR_HEIGHT: f32 = 32.0;
/// Horizontal padding inside the nav bar.
const NAV_BAR_PAD: f32 = style::SPACE_SM;
/// Height of the overtake indicator bar shown when this plugin replaced another pane.
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
    view_pane: &mut ViewPane,
    colors: &Colors,
    suppress_overtake: bool,
    pending_click: Option<crate::host::pane::PendingPaneClick>,
    context_root: &std::path::Path,
) {
    // Refresh before the plugin pumps its bridge messages: a persist handled in
    // this frame must resolve against this frame's context root.
    view_pane.runtime.refresh_context_root(context_root);
    let has_overtake = view_pane.overlay_replaced.is_some() && !suppress_overtake;
    let nav_title: Option<String> = view_pane.runtime.nav_top_title().map(|t| t.to_owned());
    // Resolve before any mutable borrow of view_pane.runtime inside closures below.
    let nav_back_view_id: String = if nav_title.is_some() {
        view_pane.runtime.nav_back_view_id()
    } else {
        String::new()
    };

    if has_overtake || nav_title.is_some() {
        log::debug!(
            "view_pane::render: chrome_h={:.0} (overtake={has_overtake} nav={})",
            chrome_height(has_overtake, nav_title.is_some()),
            nav_title.is_some(),
        );
    }

    // -- Overtake bar --
    // allocate_exact_size advances the parent cursor so the plugin content viewport shrinks.
    if has_overtake {
        let (bar_rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), OVERTAKE_BAR_HEIGHT),
            egui::Sense::hover(),
        );
        ui.painter().rect_filled(bar_rect, 0.0, colors.bg_active);
        ui.scope_builder(egui::UiBuilder::new().max_rect(bar_rect), |ui| {
            ui.horizontal_centered(|ui| {
                ui.add_space(NAV_BAR_PAD);
                let replaced_label = match view_pane.overlay_replaced.as_deref() {
                    Some(crate::host::pane::Pane::Terminal(t)) => {
                        t.name.as_deref().unwrap_or("Terminal").to_string()
                    }
                    Some(crate::host::pane::Pane::View(a)) => a.name.clone(),
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
        ui.scope_builder(egui::UiBuilder::new().max_rect(bar_rect), |ui| {
            ui.horizontal_centered(|ui| {
                ui.add_space(NAV_BAR_PAD);
                // Back arrow button — ‹ (single angle quotation mark, U+2039)
                let back_btn = button::toolbar_button(
                    ui,
                    button::KitButton::new("‹", colors)
                        .text_size(style::TEXT_BODY + 2.0)
                        .text_color(colors.accent),
                    "Back",
                );
                if back_btn.clicked() {
                    view_pane.runtime.queue_outbound_event(PlexiEvent::NavBack {
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
    // guaranteed by the allocate_exact_size calls above. Plugins read ui.available_height() /
    // available_size() and always see the post-chrome budget with no guesswork.
    let ctx = ViewRenderContext {
        colors,
        pane_id: view_pane.id,
    };
    let content_rect = ui.available_rect_before_wrap();
    view_pane.runtime.ui(ui, &ctx, pending_click);
    if matches!(
        view_pane.runtime,
        crate::host::pane::ViewRuntime::Builtin(_)
    ) {
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
            view_pane.semantic_state = state;
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

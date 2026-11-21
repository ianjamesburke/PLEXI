//! The one scope switcher for pickers and palettes: every scope as a chip,
//! the active one selected, and the Tab hint on the right. Pickers pass data
//! and handle Tab themselves; no picker draws its own scope header text.

use crate::ui::button::{ButtonKind, ButtonSize, KitButton};
use crate::ui::hints::HintGroup;
use crate::ui::style;
use crate::ui::theme::Colors;

const TAB: &[&str] = &["tab"];
const SHIFT_TAB: &[&str] = &["shift", "tab"];
const HINT_LABEL: &str = "switch";

/// Two scopes toggle, so Tab alone says it all. Three or more cycle, and
/// Shift+Tab is the only way back, so it joins the hint.
fn hint_combos(scope_count: usize) -> &'static [&'static [&'static str]] {
    if scope_count > 2 {
        &[TAB, SHIFT_TAB]
    } else {
        &[TAB]
    }
}

pub(crate) struct ScopeSwitcher<'a> {
    id: &'a str,
    scopes: &'a [&'a str],
    active: usize,
    in_scroll_body: bool,
}

impl<'a> ScopeSwitcher<'a> {
    /// `scopes` in Tab order; `active` indexes into it.
    pub(crate) fn new(id: &'a str, scopes: &'a [&'a str], active: usize) -> Self {
        debug_assert!(scopes.len() >= 2, "a scope switcher needs an alternative");
        debug_assert!(active < scopes.len(), "active scope out of range");
        Self {
            id,
            scopes,
            active,
            in_scroll_body: false,
        }
    }

    /// The row sits inside a scroll body, whose viewport may clip a partly
    /// visible row; the layout report then checks labels against their chips,
    /// not the viewport.
    pub(crate) fn in_scroll_body(mut self) -> Self {
        self.in_scroll_body = true;
        self
    }

    /// Returns the scope a click chose, never the active one. Chips stay out
    /// of egui's Tab traversal: Tab belongs to the picker's own key handler.
    /// The hint is optional and drops first; on a row too narrow for the
    /// chips themselves they wrap, never elide past their label.
    pub(crate) fn show(self, ui: &mut egui::Ui, colors: &Colors) -> Option<usize> {
        // The active scope is the one filled, outlined chip; the others are
        // flat labels until hovered, so the row reads as one control.
        let chip = |label: &'a str, selected: bool| {
            let button = KitButton::new(label, colors)
                .kind(ButtonKind::Ghost)
                .size(ButtonSize::Sm)
                .selected(selected)
                .focusable(false);
            if selected {
                button
            } else {
                button.flat()
            }
        };
        let gap = style::SPACE_SM;
        let chips_width = self
            .scopes
            .iter()
            .enumerate()
            .map(|(i, label)| chip(label, i == self.active).natural_width(ui))
            .sum::<f32>()
            + gap * (self.scopes.len() - 1) as f32;
        let combos = hint_combos(self.scopes.len());
        let hint_width = HintGroup::alternatives(combos, HINT_LABEL).width(ui);
        let available = ui.available_width();
        let show_hint = chips_width + style::SPACE_MD + hint_width <= available;
        let wrap = chips_width > available;
        if wrap {
            static WRAPPED: std::sync::Once = std::sync::Once::new();
            WRAPPED.call_once(|| log::info!("ui: scope switcher {} wraps its chips", self.id));
        }

        let mut capture = super::layout_report::Capture::begin(ui.painter(), "scope_switcher");
        if self.in_scroll_body {
            capture.scrolling();
        }
        let start = ui.cursor().min;
        let mut chosen = None;
        let mut actions = Vec::with_capacity(self.scopes.len());
        let mut rect = ui
            .allocate_ui_with_layout(
                egui::vec2(available, style::BUTTON_H_SM),
                egui::Layout::left_to_right(egui::Align::Center).with_main_wrap(wrap),
                |ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(gap, gap);
                    for (i, label) in self.scopes.iter().enumerate() {
                        let id = egui::Id::new(("plexi_scope_switcher", self.id, i));
                        let response = ui
                            .scope_builder(egui::UiBuilder::new().id(id), |ui| {
                                ui.add(chip(label, i == self.active))
                            })
                            .inner;
                        actions.push(super::layout_report::Action {
                            id: format!("{}:{i}", self.id),
                            name: (*label).into(),
                            painted_label: (*label).into(),
                            hit: response.rect,
                            minimum: egui::vec2(0.0, style::BUTTON_H_SM),
                            inactive: false,
                        });
                        if response.clicked() && i != self.active {
                            chosen = Some(i);
                        }
                    }
                },
            )
            .response
            .rect;
        if show_hint {
            // Right-aligned on the chips' row, in a rect measured to fit it.
            let hint_rect = egui::Rect::from_min_size(
                egui::pos2(start.x + available - hint_width, start.y),
                egui::vec2(hint_width, style::BUTTON_H_SM),
            );
            let hint = egui::UiBuilder::new()
                .max_rect(hint_rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center));
            rect |= ui
                .scope_builder(hint, |ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    crate::ui::shortcuts::key_combo_list_inline(
                        ui,
                        combos,
                        Some(HINT_LABEL),
                        colors,
                    );
                })
                .response
                .rect;
        }
        // Kit buttons skip painting outside the viewport, so a row scrolled
        // wholly out of view has no paint to report.
        if ui.is_rect_visible(rect) {
            capture.finish(ui.painter(), rect, colors.bg_sidebar, actions, vec![]);
        }
        if let Some(i) = chosen {
            log::info!(
                "ui: scope switcher {} chip click -> {}",
                self.id,
                self.scopes[i]
            );
        }
        chosen
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_scopes_hint_tab_and_three_add_shift_tab() {
        assert_eq!(hint_combos(2), &[TAB]);
        assert_eq!(hint_combos(3), &[TAB, SHIFT_TAB]);
    }
}

//! Scope choices in the picker's Tab order. Input and scope state belong to the caller.

use super::bounded::{self, Presentation, Slot};
use super::button::{ButtonKind, ButtonSize, KitButton};
use super::hints::HintGroup;
use super::layout_report::{Action, Capture};
use super::{shortcuts, style, theme::Colors};
use egui::{Align, Layout, Rect, Ui, UiBuilder, vec2};

const TAB: &[&str] = &["tab"];
const SHIFT_TAB: &[&str] = &["shift", "tab"];
const HINT: &str = "switch";

fn hint_combos(count: usize) -> &'static [&'static [&'static str]] {
    if count > 2 { &[TAB, SHIFT_TAB] } else { &[TAB] }
}

pub(crate) struct ScopeSwitcher<'a> {
    scopes: &'a [&'a str],
    active: usize,
}

impl<'a> ScopeSwitcher<'a> {
    pub(crate) fn new(scopes: &'a [&'a str], active: usize) -> Self {
        debug_assert!(scopes.len() >= 2 && active < scopes.len());
        Self { scopes, active }
    }

    /// Returns the index chosen by pointer or accessibility activation. The
    /// search field stays the only keyboard focus stop in either picker.
    pub(crate) fn show(self, ui: &mut Ui, colors: &Colors) -> Option<usize> {
        let backdrop = colors.bg_sidebar;
        let buttons: Vec<_> = self
            .scopes
            .iter()
            .enumerate()
            .map(|(index, &label)| {
                let button = KitButton::new(label, colors)
                    .kind(ButtonKind::Ghost)
                    .size(ButtonSize::Sm)
                    .surface(backdrop)
                    .selected(index == self.active)
                    .flat()
                    .focusable(false);
                let width = button.natural_width(ui);
                (label, button, width)
            })
            .collect();
        let gap = style::SPACE_XS;
        let height = ButtonSize::Sm.height();
        let available = ui.available_width().max(0.0);
        let choices_width = buttons.iter().map(|(_, _, width)| width).sum::<f32>()
            + gap * self.scopes.len().saturating_sub(1) as f32;
        let bounds = Rect::from_min_size(ui.next_widget_position(), vec2(available, height));
        let combos = hint_combos(self.scopes.len());
        let hint_slot = Slot {
            id: HINT,
            width: HintGroup::alternatives(combos, HINT).width(ui).ceil(),
            compact: (combos.len() > 1).then(|| HintGroup::new(TAB, HINT).width(ui).ceil()),
            required: false,
        };

        let capture = Capture::begin(ui.painter(), "scope_switcher");
        let mut actions = Vec::with_capacity(buttons.len());
        let mut chosen = None;
        let row = ui.allocate_ui_with_layout(
            bounds.size(),
            Layout::left_to_right(Align::Center).with_main_wrap(choices_width > available),
            |ui| {
                ui.spacing_mut().item_spacing = vec2(gap, gap);
                ui.ctx().accesskit_node_builder(ui.unique_id(), |node| {
                    node.set_role(egui::accesskit::Role::TabList);
                    node.set_label("Scope");
                });
                for (index, (label, button, width)) in buttons.into_iter().enumerate() {
                    let size = vec2(width.min(available), height);
                    let response = ui
                        .allocate_ui_with_layout(size, Layout::left_to_right(Align::Center), |ui| {
                            if width > available {
                                ui.add(button.wrap_mode(egui::TextWrapMode::Wrap))
                            } else {
                                ui.add(button)
                            }
                        })
                        .inner;
                    ui.ctx().accesskit_node_builder(response.id, |node| {
                        node.set_role(egui::accesskit::Role::Tab);
                        node.set_selected(index == self.active);
                    });
                    actions.push(Action {
                        id: format!("scope:{index}"),
                        name: label.into(),
                        painted_label: label.into(),
                        hit: response.rect,
                        minimum: size,
                        inactive: false,
                    });
                    if response.clicked() && index != self.active {
                        chosen = Some(index);
                    }
                }
            },
        );

        let mut rect = row.response.rect;
        if let Ok(hint) = bounded::allocate(bounds, choices_width, style::SPACE_MD, &[hint_slot]) {
            let slot = &hint.slots[0];
            let visible_combos = match slot.presentation {
                Presentation::Full => combos,
                Presentation::Compact => &[TAB],
                Presentation::Hidden => &[],
            };
            if !visible_combos.is_empty() {
                let mut child = ui.new_child(
                    UiBuilder::new()
                        .max_rect(slot.rect)
                        .layout(Layout::left_to_right(Align::Center)),
                );
                child.spacing_mut().item_spacing.x = 0.0;
                shortcuts::key_combo_list_inline(&mut child, visible_combos, Some(HINT), colors);
                rect |= child.min_rect();
            }
        }
        if ui.is_rect_visible(rect) {
            capture.finish(ui.painter(), rect, backdrop, actions, vec![]);
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

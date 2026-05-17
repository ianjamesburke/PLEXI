use egui::{Align2, Color32, CornerRadius, Pos2, RichText, Stroke, StrokeKind, Vec2};

use crate::style;
use crate::theme::Colors;

/// Consistent inner padding applied to every selectable row.
const ROW_PAD_H: f32 = 10.0;
const ROW_PAD_V: f32 = 6.0;

/// Renders a highlighted selectable row whose height is determined by its
/// content. The widget owns horizontal and vertical padding so callers
/// render only raw content without spacing boilerplate.
///
/// Returns `(response, content_return_value)`. Check `response.clicked()` to
/// detect activation and `response.hovered()` to drive keyboard selection.
///
/// Pattern: reserve a shape slot before rendering content, then fill it after
/// measuring — so the highlight sits behind the text in Z-order even though
/// the rect is only known after layout.
pub(crate) fn selectable_row<R>(
    ui: &mut egui::Ui,
    is_selected: bool,
    colors: &Colors,
    content: impl FnOnce(&mut egui::Ui) -> R,
) -> (egui::Response, R) {
    let fill: Color32 = if is_selected {
        colors.bg_active
    } else {
        Color32::TRANSPARENT
    };

    let bg_idx = ui.painter().add(egui::Shape::Noop);

    let scope = ui.scope(|ui| {
        ui.set_width(ui.available_width());
        ui.add_space(ROW_PAD_V);
        // Left-indent the content without using Frame (Frame inner_margin
        // interferes with ScrollArea height measurement).
        let inner = ui
            .horizontal(|ui| {
                ui.add_space(ROW_PAD_H);
                ui.vertical(|ui| content(ui)).inner
            })
            .inner;
        ui.add_space(ROW_PAD_V);
        inner
    });

    let row_rect = scope.response.rect;

    ui.painter().set(
        bg_idx,
        egui::Shape::rect_filled(row_rect, CornerRadius::same(4), fill),
    );

    let response = ui.interact(
        row_rect,
        scope.response.id.with("_selectable_row_hit"),
        egui::Sense::click(),
    );

    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    (response, scope.inner)
}

// ── Keycap primitive ─────────────────────────────────────────────────────
//
// The notification modal shipped with "⌘[/⌘] to cycle" rendered as flat
// text and it looks like run-on typography. Native macOS menus render
// each key as a distinct visual chip; we do the same. One rendering rule,
// one primitive, apply everywhere shortcuts appear.
//
// Usage:
//     key_combo(ui, &["⌘", "["], colors);        // single shortcut
//     key_combo_list(ui, &[["⌘", "["], ["⌘", "]"]], colors, "to cycle");

/// Padding around the key label text inside the chip.
const KEYCAP_PAD_H: f32 = 6.0;
const KEYCAP_PAD_V: f32 = 3.0;

/// Render a single keycap chip. Allocates its own exact-size rect and
/// returns the egui Response so callers can compose with other widgets.
pub(crate) fn key_chip(ui: &mut egui::Ui, label: &str, colors: &Colors) -> egui::Response {
    let font_id = egui::FontId::monospace(style::TEXT_CAPTION);
    let galley = ui
        .fonts(|f| f.layout_no_wrap(label.to_string(), font_id, colors.text_primary));
    let text_w = galley.size().x;
    let text_h = galley.size().y;
    let chip_h = text_h + KEYCAP_PAD_V * 2.0;
    let chip_w = (text_w + KEYCAP_PAD_H * 2.0).max(chip_h);
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(chip_w, chip_h),
        egui::Sense::hover(),
    );
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(3), colors.bg_active);
    painter.rect_stroke(
        rect,
        CornerRadius::same(3),
        Stroke::new(1.0, colors.border),
        StrokeKind::Inside,
    );
    let text_pos = Pos2::new(
        rect.center().x - text_w / 2.0,
        rect.min.y + KEYCAP_PAD_V,
    );
    painter.galley(text_pos, galley, colors.text_primary);
    response
}

/// Gap between chips within a single combo (e.g. ⌘ + [).
const INTRA_COMBO_GAP: f32 = 2.0;
/// Gap between separate combos in a list (e.g. ⌘[ vs ⌘]).
const INTER_COMBO_GAP: f32 = 10.0;
/// Gap between the last combo/chip and the trailing description text.
const TRAILING_GAP: f32 = 10.0;

/// Render several chips forming a single key combo (e.g. ["⌘", "["]).
/// Chips are separated by a "+" label between each pair.
pub(crate) fn key_combo(ui: &mut egui::Ui, keys: &[&str], colors: &Colors) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = INTRA_COMBO_GAP;
        for (i, key) in keys.iter().enumerate() {
            if i > 0 {
                ui.label(
                    egui::RichText::new("+")
                        .size(style::TEXT_HINT)
                        .color(colors.text_dim),
                );
            }
            key_chip(ui, key, colors);
        }
    });
}

/// Render multiple combos inline with an optional trailing description label.
///
/// Layout:    [⌘][[] ··inter·· [⌘][] ··trailing·· "cycle"
pub(crate) fn key_combo_list(
    ui: &mut egui::Ui,
    combos: &[&[&str]],
    trailing: Option<&str>,
    colors: &Colors,
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for (i, keys) in combos.iter().enumerate() {
            if i > 0 {
                ui.add_space(INTER_COMBO_GAP);
            }
            // Chips within a combo sit INTRA_COMBO_GAP apart.
            for (j, key) in keys.iter().enumerate() {
                if j > 0 {
                    ui.add_space(INTRA_COMBO_GAP);
                }
                key_chip(ui, key, colors);
            }
        }
        if let Some(text) = trailing {
            ui.add_space(TRAILING_GAP);
            ui.label(
                egui::RichText::new(text)
                    .size(style::TEXT_HINT)
                    .color(colors.text_dim),
            );
        }
    });
}

/// 📋 / ✓ copy-to-clipboard button. Shows the clipboard icon normally; switches
/// to ✓ for 2 seconds after a successful copy. `id` must be unique per call site.
pub(crate) fn copy_button(ui: &mut egui::Ui, id: egui::Id, text: &str) -> egui::Response {
    let now = ui.ctx().input(|i| i.time);
    let copied_at: Option<f64> = ui.ctx().memory(|m| m.data.get_temp(id));
    let just_copied = copied_at.map_or(false, |t| now - t < 2.0);
    let icon = if just_copied { "✓" } else { "📋" };
    let resp = ui
        .add(
            egui::Button::new(RichText::new(icon).size(style::TEXT_CAPTION))
                .frame(false),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(format!("Copy `{text}`"));
    if resp.clicked() && !just_copied {
        ui.ctx().copy_text(text.to_string());
        ui.ctx().memory_mut(|m| m.data.insert_temp(id, now));
    }
    if just_copied {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(100));
    }
    resp
}

// ── SearchableList ────────────────────────────────────────────────────────
//
// Keyboard-navigable list with optional fuzzy search input. Extracted from
// the command palette pattern so every overlay gets the same j/k + clamping
// + scroll-into-view behavior for free.
//
// Usage:
//     // in your state struct:
//     my_list: crate::widgets::SearchableList,
//
//     // in your overlay draw fn (before rendering):
//     let confirmed = self.my_list.handle_nav(ctx, item_count);
//
//     // inside the egui Area:
//     let query_changed = self.my_list.show_search(ui, "Search…", "my_list", &colors);
//     egui::ScrollArea::vertical().show(ui, |ui| {
//         for (i, item) in items.iter().enumerate() {
//             let is_sel = i == self.my_list.selected;
//             let (resp, _) = selectable_row(ui, is_sel, &colors, |ui| { /* ... */ });
//             if is_sel && self.my_list.selection_changed() { resp.scroll_to_me(None); }
//             if resp.clicked() { /* activate item i */ }
//         }
//     });

/// Shared keyboard-nav + search state for list overlays.
pub(crate) struct SearchableList {
    pub(crate) selected: usize,
    pub(crate) query: String,
    prev_selected: usize,
}

impl Default for SearchableList {
    fn default() -> Self {
        Self {
            selected: 0,
            query: String::new(),
            prev_selected: 0,
        }
    }
}

impl SearchableList {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Reset selection and search query (call when the overlay opens).
    pub(crate) fn reset(&mut self) {
        self.selected = 0;
        self.prev_selected = 0;
        self.query.clear();
    }

    /// Clamp `selected` to `count` items. Call after filtering items so an
    /// existing selection can't point past the end of the filtered list.
    pub(crate) fn clamp(&mut self, count: usize) {
        if count > 0 {
            self.selected = self.selected.min(count - 1);
        }
    }

    /// True if the selected index changed since the last `handle_nav` call.
    /// Use this to decide whether to call `resp.scroll_to_me(None)`.
    pub(crate) fn selection_changed(&self) -> bool {
        self.selected != self.prev_selected
    }

    /// Process j/k/arrow/Enter key events. Returns `Some(idx)` when Enter is
    /// pressed; returns `None` otherwise. Does NOT consume Escape — callers
    /// handle dismiss themselves.
    ///
    /// Call at `ctx` level before the egui Area so key events are consumed
    /// before widgets inside the Area can see them.
    pub(crate) fn handle_nav(
        &mut self,
        ctx: &egui::Context,
        item_count: usize,
    ) -> Option<usize> {
        self.prev_selected = self.selected;
        let mut confirmed: Option<usize> = None;

        ctx.input_mut(|input| {
            if (input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown)
                || input.consume_key(egui::Modifiers::NONE, egui::Key::J))
                && item_count > 0
            {
                self.selected = (self.selected + 1).min(item_count - 1);
            }
            if (input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp)
                || input.consume_key(egui::Modifiers::NONE, egui::Key::K))
                && item_count > 0
            {
                self.selected = self.selected.saturating_sub(1);
            }
            if input.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
                && item_count > 0
            {
                confirmed = Some(self.selected);
            }
        });

        confirmed
    }

    /// Render a styled search text input, updating `self.query`.
    /// Returns `true` if the query changed this frame (callers should
    /// re-filter their item list and call `self.clamp(new_count)`).
    ///
    /// `id_salt` must be unique per call site so egui's Id doesn't collide
    /// when multiple SearchableList instances are shown simultaneously.
    pub(crate) fn show_search(
        &mut self,
        ui: &mut egui::Ui,
        hint: &str,
        id_salt: &str,
        colors: &Colors,
    ) -> bool {
        let te_id = egui::Id::new(id_salt).with("search");
        let changed = ui.scope(|ui| {
            ui.visuals_mut().text_cursor.stroke.width = 1.5;
            ui.visuals_mut().text_cursor.stroke.color = colors.accent;
            ui.visuals_mut().extreme_bg_color = colors.bg_active;
            ui.visuals_mut().widgets.active.bg_stroke =
                egui::Stroke::new(1.0, colors.accent);
            ui.visuals_mut().widgets.inactive.bg_stroke =
                egui::Stroke::new(1.0, colors.border);
            ui.add(
                egui::TextEdit::singleline(&mut self.query)
                    .id(te_id)
                    .desired_width(f32::INFINITY)
                    .hint_text(hint)
                    .font(egui::TextStyle::Body)
                    .margin(egui::Margin::symmetric(8, 5)),
            )
        }).inner;
        if !changed.has_focus() {
            changed.request_focus();
        }
        if changed.changed() {
            self.selected = 0;
            self.prev_selected = 0;
            return true;
        }
        false
    }
}

/// Renders a dismissable centered modal overlay. Handles Escape and click-outside.
/// Returns `true` if the user dismissed it this frame. Callers guard with
/// `if !open { return; }` and apply `if dismissed { open = false; }` after.
pub fn dismissable_modal(
    ctx: &egui::Context,
    id: &str,
    add_contents: impl FnOnce(&mut egui::Ui),
) -> bool {
    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
        return true;
    }

    let screen_rect = ctx.screen_rect();
    let mut dismissed = false;

    let base_id = egui::Id::new(id);
    egui::Area::new(base_id.with("scrim"))
        .fixed_pos(screen_rect.min)
        .order(egui::Order::Middle)
        .show(ctx, |ui| {
            ui.painter()
                .rect_filled(screen_rect, 0.0, Color32::from_black_alpha(80));
            if ui
                .allocate_rect(screen_rect, egui::Sense::click())
                .clicked()
            {
                dismissed = true;
            }
        });

    egui::Area::new(base_id.with("overlay"))
        .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            add_contents(ui);
        });

    dismissed
}

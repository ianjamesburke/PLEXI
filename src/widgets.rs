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

/// Renders a styled singleline text input with the standard modal visual treatment:
/// accent cursor, accent active border, muted inactive border, `bg_active` fill,
/// `Body` font, and `8×5` inner margin. Width fills available space.
///
/// Returns the egui `Response`; callers handle focus requests and key events themselves.
pub(crate) fn styled_text_input(
    ui: &mut egui::Ui,
    buf: &mut String,
    hint: impl Into<egui::WidgetText>,
    id: egui::Id,
    colors: &Colors,
) -> egui::Response {
    ui.scope(|ui| {
        ui.visuals_mut().text_cursor.stroke.width = 1.5;
        ui.visuals_mut().text_cursor.stroke.color = colors.accent;
        ui.visuals_mut().extreme_bg_color = colors.bg_active;
        ui.visuals_mut().widgets.active.bg_stroke = egui::Stroke::new(1.0, colors.accent);
        ui.visuals_mut().widgets.inactive.bg_stroke = egui::Stroke::new(1.0, colors.border);
        ui.add(
            egui::TextEdit::singleline(buf)
                .id(id)
                .desired_width(f32::INFINITY)
                .hint_text(hint)
                .font(egui::TextStyle::Body)
                .margin(egui::Margin::symmetric(8, 5)),
        )
    })
    .inner
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

// ── Overlay layout primitives ────────────────────────────────────────────
//
// Every overlay (inspector, sub-context UX, unified overlays) was hand-rolling
// the same section label, pane-type badge, status chip, and truncating text
// label. Each diverged in color logic and spacing. These four primitives
// centralize that rendering so future overlays look identical with zero extra
// work.
//
// Status color mapping lives here — not in individual overlays — so adding a
// new status value is a one-line change that applies everywhere.

/// Renders a section / group header label (context name, group title, etc.).
/// `is_active` tints the label with `colors.accent` instead of `colors.text_dim`.
pub(crate) fn section_header(ui: &mut egui::Ui, label: &str, is_active: bool, colors: &Colors) {
    let color = if is_active { colors.accent } else { colors.text_dim };
    ui.label(
        egui::RichText::new(label)
            .size(style::TEXT_CAPTION)
            .color(color),
    );
}

/// Renders the pane type as a single-letter monospace chip: `"T"` for Terminal,
/// `"A"` for App, or the first uppercase character of any other kind string.
/// Uses `key_chip` so the visual weight matches keyboard shortcut chips.
pub(crate) fn pane_type_badge(ui: &mut egui::Ui, kind: &str, colors: &Colors) {
    let letter = kind.chars().next().unwrap_or('?').to_uppercase().next().unwrap_or('?');
    key_chip(ui, &letter.to_string(), colors);
}

/// Renders a status string with centralized color mapping:
/// - `"busy"` / `"running"` → `colors.accent`
/// - `"crashed"` / `"hung"` / `"error"` / `"exited"` → `colors.danger`
/// - everything else (`"idle"`, `"booting"`, ...) → `colors.text_dim`
pub(crate) fn status_chip(ui: &mut egui::Ui, status: &str, colors: &Colors) {
    let color = match status {
        "busy" | "running" => colors.accent,
        "crashed" | "hung" | "error" | "exited" => colors.danger,
        _ => colors.text_dim,
    };
    ui.label(
        egui::RichText::new(status)
            .size(style::TEXT_HINT)
            .color(color),
    );
}

/// Renders a single-line label that truncates with an ellipsis at the available
/// width. **Callers must call `ui.set_max_width(n)` before calling this** (or
/// use `ui.scope()` with a constrained width) — `egui::Label::truncate` clips
/// at `available_width()`, so without a bound the label expands naturally and
/// truncation never fires.
pub(crate) fn description_label(ui: &mut egui::Ui, text: &str, colors: &Colors) {
    ui.add(
        egui::Label::new(
            egui::RichText::new(text)
                .size(style::TEXT_HINT)
                .color(colors.text_dim),
        )
        .truncate(),
    );
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

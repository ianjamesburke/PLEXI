use egui::{Align2, Color32, CornerRadius, Pos2, RichText, Vec2};

use crate::ui::style;
use crate::ui::theme::Colors;

/// Consistent inner padding applied to every selectable row. Horizontal
/// matches `ListRow` so the two row primitives align in mixed lists.
const ROW_PAD_H: f32 = style::LIST_ROW_PAD_H;
const ROW_PAD_V: f32 = 8.0;

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
    // Two reserved layers: selection tint + outline (the shared treatment
    // from `list::selection_shapes`), painted behind the content after the
    // row's rect is known.
    let bg_idx = ui.painter().add(egui::Shape::Noop);
    let outline_idx = ui.painter().add(egui::Shape::Noop);

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

    if is_selected {
        let [fill, outline] = crate::ui::list::selection_shapes(row_rect, colors);
        ui.painter().set(bg_idx, fill);
        ui.painter().set(outline_idx, outline);
    }

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
const KEYCAP_PAD_H: f32 = 5.0;
const KEYCAP_PAD_V: f32 = 2.0;

/// Standard chip font for combos and hint rows. Compact — chips are
/// annotations, not content.
fn combo_chip_font() -> egui::FontId {
    egui::FontId::monospace(style::TEXT_CAPTION)
}

fn shortcut_key_color(colors: &Colors) -> egui::Color32 {
    colors.text_primary.gamma_multiply(0.78)
}

fn shortcut_label_color(colors: &Colors) -> egui::Color32 {
    colors.text_primary.gamma_multiply(0.70)
}

pub(crate) fn shortcut_hint_label(text: &str, colors: &Colors) -> egui::RichText {
    egui::RichText::new(text)
        .size(style::TEXT_HINT)
        .color(shortcut_label_color(colors))
}

/// Render a single keycap chip. Allocates its own exact-size rect and
/// returns the egui Response so callers can compose with other widgets.
///
/// Pass `egui::FontId::monospace(style::TEXT_CAPTION)` for the standard size,
/// or `egui::FontId::monospace(style::TEXT_HINT)` for compact footer chips.
pub(crate) fn key_chip(
    ui: &mut egui::Ui,
    label: &str,
    colors: &Colors,
    font_id: egui::FontId,
) -> egui::Response {
    let fg = shortcut_key_color(colors);
    let galley = ui.fonts(|f| f.layout_no_wrap(label.to_string(), font_id, fg));
    let text_w = galley.size().x;
    let text_h = galley.size().y;
    let chip_h = text_h + KEYCAP_PAD_V * 2.0;
    let chip_w = (text_w + KEYCAP_PAD_H * 2.0).max(chip_h);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(chip_w, chip_h), egui::Sense::hover());
    // Borderless badge-style fill — chips are quiet annotations.
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(4), colors.bg_active);
    let text_pos = Pos2::new(rect.center().x - text_w / 2.0, rect.min.y + KEYCAP_PAD_V);
    painter.galley(text_pos, galley, fg);
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
                        .color(shortcut_label_color(colors)),
                );
            }
            key_chip(ui, key, colors, combo_chip_font());
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
                key_chip(ui, key, colors, combo_chip_font());
            }
        }
        if let Some(text) = trailing {
            ui.add_space(TRAILING_GAP);
            ui.label(
                egui::RichText::new(text)
                    .size(style::TEXT_HINT)
                    .color(shortcut_label_color(colors)),
            );
        }
    });
}

pub(crate) enum ButtonKind {
    Primary,
    Secondary,
    Danger,
}

pub(crate) fn chrome_button(
    ui: &mut egui::Ui,
    label: &str,
    kind: ButtonKind,
    colors: &Colors,
    min_width: f32,
) -> egui::Response {
    chrome_button_sized(ui, label, kind, colors, min_width, style::BUTTON_H_MD)
}

pub(crate) fn chrome_button_sized(
    ui: &mut egui::Ui,
    label: &str,
    kind: ButtonKind,
    colors: &Colors,
    min_width: f32,
    height: f32,
) -> egui::Response {
    let (fill, text_color) = match kind {
        ButtonKind::Primary => (colors.bg_active, colors.text_primary),
        ButtonKind::Secondary => (colors.bg_active, colors.text_dim),
        ButtonKind::Danger => (colors.danger, colors.text_on(colors.danger)),
    };

    ui.add(
        egui::Button::new(
            RichText::new(label)
                .size(style::TEXT_CAPTION)
                .color(text_color),
        )
        .fill(fill)
        .min_size(egui::vec2(min_width, height)),
    )
    .on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Measured width of a `key_combo_list` row without rendering it — lets
/// containers (e.g. `HintBar`) center hint rows. Must mirror the layout
/// math in `key_combo_list` exactly.
pub(crate) fn key_combo_list_width(
    ui: &egui::Ui,
    combos: &[&[&str]],
    trailing: Option<&str>,
) -> f32 {
    let measure = |text: &str, font: egui::FontId| {
        ui.fonts(|f| f.layout_no_wrap(text.to_string(), font, Color32::WHITE))
            .size()
    };
    let mut w = 0.0;
    for (i, keys) in combos.iter().enumerate() {
        if i > 0 {
            w += INTER_COMBO_GAP;
        }
        for (j, key) in keys.iter().enumerate() {
            if j > 0 {
                w += INTRA_COMBO_GAP;
            }
            let size = measure(key, combo_chip_font());
            let chip_h = size.y + KEYCAP_PAD_V * 2.0;
            w += (size.x + KEYCAP_PAD_H * 2.0).max(chip_h);
        }
    }
    if let Some(text) = trailing {
        w += TRAILING_GAP;
        w += measure(text, egui::FontId::proportional(style::TEXT_HINT)).x;
    }
    w
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
    styled_text_input_inner(ui, buf, hint, id, colors, false)
}

fn styled_text_input_inner(
    ui: &mut egui::Ui,
    buf: &mut String,
    hint: impl Into<egui::WidgetText>,
    id: egui::Id,
    colors: &Colors,
    password: bool,
) -> egui::Response {
    // Placeholder at half strength — the theme sets `override_text_color`,
    // which makes egui's default hint color (weak_text_color) near-white.
    let hint: egui::WidgetText = hint.into();
    let hint = hint.color(colors.text_primary.gamma_multiply(0.5));
    ui.scope(|ui| {
        ui.visuals_mut().text_cursor.stroke.width = 1.5;
        ui.visuals_mut().text_cursor.stroke.color = colors.accent;
        ui.visuals_mut().extreme_bg_color = colors.bg_active;
        ui.visuals_mut().widgets.active.bg_stroke = egui::Stroke::new(1.0, colors.accent);
        ui.visuals_mut().widgets.inactive.bg_stroke = egui::Stroke::new(1.0, colors.border);
        let edit = egui::TextEdit::singleline(buf)
            .id(id)
            .desired_width(f32::INFINITY)
            .hint_text(hint)
            .font(egui::TextStyle::Body)
            .margin(egui::Margin::symmetric(8, 5))
            .password(password);
        ui.add(edit)
    })
    .inner
}

pub(crate) struct TextField<'a> {
    id: egui::Id,
    hint: egui::WidgetText,
    focused: bool,
    password: bool,
    log_name: &'a str,
}

impl<'a> TextField<'a> {
    pub(crate) fn singleline(id: egui::Id, hint: impl Into<egui::WidgetText>) -> Self {
        Self {
            id,
            hint: hint.into(),
            focused: false,
            password: false,
            log_name: "text_field",
        }
    }

    pub(crate) fn focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }

    pub(crate) fn password(mut self, password: bool) -> Self {
        self.password = password;
        self
    }

    pub(crate) fn log_name(mut self, log_name: &'a str) -> Self {
        self.log_name = log_name;
        self
    }

    pub(crate) fn show(
        self,
        ui: &mut egui::Ui,
        buf: &mut String,
        colors: &Colors,
    ) -> egui::Response {
        let response = styled_text_input_inner(ui, buf, self.hint, self.id, colors, self.password);
        if self.focused && !response.has_focus() {
            response.request_focus();
            log::info!("{}: focus requested for host TextField", self.log_name);
        }
        // Any field that currently owns focus must register with the overlay
        // focus system — without this, widgets rendered later in the frame
        // (app panes) steal focus back every frame, even mid-typing.
        if self.focused || response.has_focus() {
            crate::ui::focus::register_overlay_focus(ui.ctx(), self.id);
        }
        response
    }
}

/// Copy-to-clipboard button. Shows a nerd-font copy icon normally; switches
/// to ✓ for 2 seconds after a successful copy. `id` must be unique per call
/// site. U+F0C5 (nf-fa-copy) is verified present in the bundled JetBrainsMono
/// Nerd Font — 📋 is NOT, and renders as an ugly fallback glyph.
pub(crate) fn copy_button(ui: &mut egui::Ui, id: egui::Id, text: &str) -> egui::Response {
    let now = ui.ctx().input(|i| i.time);
    let copied_at: Option<f64> = ui.ctx().memory(|m| m.data.get_temp(id));
    let just_copied = copied_at.map_or(false, |t| now - t < 2.0);
    let icon = if just_copied { "✓" } else { "\u{f0c5}" };
    let resp = ui
        .add(egui::Button::new(RichText::new(icon).size(style::TEXT_CAPTION)).frame(false))
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(format!("Copy `{text}`"));
    if resp.clicked() && !just_copied {
        ui.ctx().copy_text(text.to_string());
        ui.ctx().memory_mut(|m| m.data.insert_temp(id, now));
    }
    if just_copied {
        crate::platform::frame_diag::note(crate::platform::frame_diag::RepaintCause::WidgetPulse);
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(100));
    }
    resp
}

/// Renders a single-line label that truncates with an ellipsis at the available
/// width. `egui::Label::truncate` clips at `available_width()`, so without a
/// constraint the label expands naturally and truncation never fires.
///
/// **Always wrap in `ui.scope()`** and set `ui.set_max_width(n)` inside the
/// scope — setting max_width on a shared `Ui` corrupts the layout of other
/// widgets already rendered in the same row (especially inside right_to_left
/// layouts):
/// ```ignore
/// ui.scope(|ui| {
///     ui.set_max_width(120.0);
///     description_label(ui, text, colors);
/// });
/// ```
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

pub(crate) fn color_swatch(
    ui: &mut egui::Ui,
    label: &str,
    fill: egui::Color32,
    colors: &Colors,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(84.0, 30.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, style::RADIUS_MD, fill);
    ui.painter().rect_stroke(
        rect,
        style::RADIUS_MD,
        egui::Stroke::new(1.0, colors.border),
        egui::StrokeKind::Inside,
    );
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(style::TEXT_HINT),
        colors.text_on(fill),
    );
    response
}

pub(crate) fn status_chip(ui: &mut egui::Ui, status: &str, colors: &Colors) -> egui::Response {
    let color = match status.to_ascii_lowercase().as_str() {
        "busy" | "running" => colors.accent,
        "crashed" | "hung" | "error" | "exited" => colors.danger,
        _ => colors.text_dim,
    };
    status_chip_with_color(ui, status, color, colors)
}

pub(crate) fn status_chip_with_color(
    ui: &mut egui::Ui,
    label: &str,
    color: egui::Color32,
    colors: &Colors,
) -> egui::Response {
    egui::Frame::new()
        .fill(colors.bg_active)
        .stroke(egui::Stroke::new(1.0, color))
        .corner_radius(style::RADIUS_BADGE)
        .inner_margin(egui::Margin::symmetric(8, 3))
        .show(ui, |ui| {
            ui.label(RichText::new(label).size(style::TEXT_HINT).color(color));
        })
        .response
}

pub(crate) fn empty_state_panel(
    ui: &mut egui::Ui,
    title: &str,
    detail: Option<&str>,
    colors: &Colors,
) -> egui::Response {
    egui::Frame::new()
        .fill(colors.bg_toolbar)
        .stroke(egui::Stroke::new(1.0, colors.border))
        .corner_radius(style::RADIUS_MD)
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new(title)
                        .size(style::TEXT_BODY)
                        .color(colors.text_primary),
                );
                if let Some(detail) = detail {
                    ui.label(
                        RichText::new(detail)
                            .size(style::TEXT_HINT)
                            .color(colors.text_dim),
                    );
                }
            });
        })
        .response
}

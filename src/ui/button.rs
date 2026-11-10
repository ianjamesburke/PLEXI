use egui::{Align2, Color32, RichText, Vec2};

use crate::ui::style;
use crate::ui::theme::Colors;

/// Fill and emphasis of a button. `Primary` is the declared default kind.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ButtonKind {
    #[default]
    Primary,
    Secondary,
    Accent,
    Danger,
    /// Transparent at rest; the theme's hover/press fill appears on
    /// interaction. Used for menu items and text/glyph controls.
    Ghost,
}

#[derive(Clone, Copy)]
pub(crate) struct ChromeButtonState {
    pub(crate) disabled: bool,
    pub(crate) down: bool,
}

/// Height class. `Md` is the declared default.
/// Sm is for compact controls and menus, Md for default actions, Lg for prominent actions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ButtonSize {
    Sm,
    #[default]
    Md,
    Lg,
}

impl ButtonSize {
    pub(crate) fn height(self) -> f32 {
        match self {
            Self::Sm => style::BUTTON_H_SM,
            Self::Md => style::BUTTON_H_MD,
            Self::Lg => style::BUTTON_H_LG,
        }
    }
}

#[derive(Clone, Copy, Default)]
struct LabelStyle {
    size: Option<f32>,
    color: Option<Color32>,
    monospace: bool,
}

/// The one button. Every host, toast and plugin button is painted here, so
/// fills, radii, padding, fonts and the contrast pass cannot drift. Use as an
/// egui Widget so enabled scopes, synthetic IDs and popup handling retain
/// egui's normal response semantics. The label is a plain `&str` on purpose:
/// styled text would be silently flattened; request size, colour or monospace
/// explicitly with `text_size`, `text_color` and `monospace`.
pub(crate) struct KitButton<'a> {
    label: &'a str,
    colors: Colors,
    kind: ButtonKind,
    size: ButtonSize,
    min_size: Vec2,
    selected: bool,
    wrap: egui::TextWrapMode,
    text: LabelStyle,
    compact: bool,
    surface: Option<Color32>,
}

impl<'a> KitButton<'a> {
    /// Single-line by default: an over-long label elides to the available
    /// width. Call `wrap_mode(Wrap)` for labels that must wrap with the name
    /// intact (toasts, plugin buttons).
    pub(crate) fn new(label: &'a str, colors: &Colors) -> Self {
        Self {
            label,
            colors: *colors,
            kind: ButtonKind::default(),
            size: ButtonSize::default(),
            min_size: Vec2::ZERO,
            selected: false,
            wrap: egui::TextWrapMode::Extend,
            text: LabelStyle::default(),
            compact: false,
            surface: None,
        }
    }

    pub(crate) fn kind(mut self, kind: ButtonKind) -> Self {
        self.kind = kind;
        self
    }
    pub(crate) fn size(mut self, size: ButtonSize) -> Self {
        self.size = size;
        self
    }
    pub(crate) fn min_size(mut self, size: Vec2) -> Self {
        self.min_size = size;
        self
    }
    pub(crate) fn min_width(mut self, width: f32) -> Self {
        self.min_size.x = width;
        self
    }
    /// Raise the minimum width to at least `width`, keeping a larger one.
    pub(crate) fn min_width_at_least(mut self, width: f32) -> Self {
        self.min_size.x = self.min_size.x.max(width);
        self
    }
    /// Mark this button as the active member of a group.
    pub(crate) fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }
    pub(crate) fn wrap_mode(mut self, wrap: egui::TextWrapMode) -> Self {
        self.wrap = wrap;
        self
    }
    /// Label font size; the default is `TEXT_CAPTION`.
    pub(crate) fn text_size(mut self, size: f32) -> Self {
        self.text.size = Some(size);
        self
    }
    /// Preferred label colour. It still passes through the contrast
    /// correction against the painted fill.
    pub(crate) fn text_color(mut self, color: Color32) -> Self {
        self.text.color = Some(color);
        self
    }
    pub(crate) fn monospace(mut self) -> Self {
        self.text.monospace = true;
        self
    }
    /// Titlebar and glyph geometry: no padding, the parent row's interact
    /// height, and no elision. Use for toolbar glyphs and breadcrumb segments
    /// inside rows shorter than the Sm class.
    pub(crate) fn compact(mut self) -> Self {
        self.compact = true;
        self
    }
    /// Colour the button sits on when it is not the panel fill (menus,
    /// toasts). Contrast of the disabled and pressed states is resolved over
    /// it.
    pub(crate) fn surface(mut self, surface: Color32) -> Self {
        self.surface = Some(surface);
        self
    }
}

impl egui::Widget for KitButton<'_> {
    #[expect(
        clippy::disallowed_methods,
        reason = "the kit button is the one place that constructs egui::Button"
    )]
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        let kind = self.kind;
        let paint_kind =
            if self.selected && !matches!(kind, ButtonKind::Accent | ButtonKind::Danger) {
                ButtonKind::Accent
            } else {
                kind
            };
        log_first_paint(kind, self.size, self.compact);
        let parent = self.surface.unwrap_or(ui.visuals().panel_fill);
        let state = ChromeButtonState {
            disabled: !ui.is_enabled(),
            down: false,
        };
        let visuals =
            chrome_button_visuals(paint_kind, &self.colors, state, parent, self.text.color);
        let active = chrome_button_visuals(
            paint_kind,
            &self.colors,
            ChromeButtonState {
                down: true,
                ..state
            },
            parent,
            self.text.color,
        );
        let padding = if self.compact {
            Vec2::ZERO
        } else {
            egui::vec2(
                match self.size {
                    ButtonSize::Sm => 10.0,
                    ButtonSize::Md => 12.0,
                    ButtonSize::Lg => 16.0,
                },
                style::SPACE_XS,
            )
        };
        let label_size = self.text.size.unwrap_or_else(|| {
            let height = self.min_size.y.max(self.size.height());
            if height >= style::BUTTON_H_LG {
                style::TEXT_BODY
            } else if height >= style::BUTTON_H_MD {
                style::TEXT_CONTROL
            } else {
                style::TEXT_CAPTION
            }
        });
        let font = if self.text.monospace {
            egui::FontId::monospace(label_size)
        } else {
            crate::ui::theme::font_medium(label_size)
        };
        let shown = if matches!(self.wrap, egui::TextWrapMode::Extend) && !self.compact {
            crate::ui::text::elide(
                ui,
                self.label,
                font.clone(),
                (ui.available_width() - padding.x * 2.0).max(0.0),
                crate::ui::text::Side::Trailing,
            )
        } else {
            self.label.to_owned()
        };
        let mut text = RichText::new(shown).font(font).color(visuals.text);
        if self.text.monospace {
            text = text.monospace();
        }
        let mut min_size = self.min_size;
        if matches!(self.wrap, egui::TextWrapMode::Extend) {
            min_size.x = min_size.x.min(ui.available_width());
        }
        if !self.compact {
            min_size.y = min_size.y.max(self.size.height());
        }
        // Keep the frame geometry identical for every paint state.
        let stroke = if visuals.stroke.width == 0.0 {
            egui::Stroke::new(1.0, Color32::TRANSPARENT)
        } else {
            visuals.stroke
        };
        let button = egui::Button::new(text)
            .stroke(stroke)
            .corner_radius(if self.size == ButtonSize::Lg {
                style::RADIUS_MD
            } else {
                style::RADIUS_SM
            })
            .fill(visuals.fill)
            .min_size(min_size)
            .wrap_mode(self.wrap);
        let hover_fill = match paint_kind {
            ButtonKind::Primary => visuals.fill.lerp_to_gamma(self.colors.text_primary, 0.06),
            ButtonKind::Secondary | ButtonKind::Ghost => self.colors.bg_hover,
            ButtonKind::Accent | ButtonKind::Danger => {
                visuals.fill.lerp_to_gamma(Color32::WHITE, 0.08)
            }
        };
        let response = ui
            .scope(|ui| {
                if !self.compact {
                    ui.spacing_mut().interact_size.y = self.size.height();
                }
                ui.spacing_mut().button_padding = padding;
                tactile_button(ui, button, hover_fill, active.fill, parent)
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
            })
            .inner;
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), self.label)
        });
        response
    }
}

/// One info trace per kind/size/geometry combination: enough to see in
/// `plexi.log` which button classes a session painted, without a line per frame.
fn log_first_paint(kind: ButtonKind, size: ButtonSize, compact: bool) {
    use std::sync::atomic::{AtomicU32, Ordering};
    static SEEN: AtomicU32 = AtomicU32::new(0);
    let bit = 1u32 << ((kind as u32) * 6 + (size as u32) * 2 + u32::from(compact));
    if SEEN.fetch_or(bit, Ordering::Relaxed) & bit == 0 {
        log::info!("ui: kit button first painted kind={kind:?} size={size:?} compact={compact}");
    }
}

/// Context and overflow menu commands. A Ghost button: transparent at rest,
/// the theme hover and press fill on interaction. Callers keep their existing
/// `ui.close()` and command dispatch behavior.
pub(crate) fn menu_item(ui: &mut egui::Ui, label: &str, colors: &Colors) -> egui::Response {
    ui.add(
        KitButton::new(label, colors)
            .kind(ButtonKind::Ghost)
            .size(ButtonSize::Sm)
            .surface(ui.visuals().window_fill),
    )
}

/// A popup menu behind a compact glyph trigger. Popup behavior stays with
/// egui. The caller names the trigger for accessibility and tooltips.
pub(crate) fn overflow_menu<R>(
    ui: &mut egui::Ui,
    label: &str,
    colors: &Colors,
    contents: impl FnOnce(&mut egui::Ui) -> R,
) -> (egui::Response, Option<egui::InnerResponse<R>>) {
    // Titlebar rows can be shorter than the Sm height at large UI scales, so
    // the trigger uses the compact geometry; menu items keep their Sm targets.
    let response = ui.add(
        KitButton::new(label, colors)
            .kind(ButtonKind::Ghost)
            .compact()
            .min_width_at_least(GLYPH_MIN_W)
            .surface(colors.bg_toolbar)
            .text_color(colors.text_dim),
    );
    let inner = egui::Popup::menu(&response).show(contents);
    (response, inner)
}

#[derive(Clone, Copy)]
struct ChromeButtonVisuals {
    fill: Color32,
    text: Color32,
    stroke: egui::Stroke,
}

/// `parent` is the surface the button sits on; transparent and disabled fills
/// resolve contrast over it. `preferred_text` overrides the kind's text colour
/// before the contrast correction.
fn chrome_button_visuals(
    kind: ButtonKind,
    colors: &Colors,
    state: ChromeButtonState,
    parent: Color32,
    preferred_text: Option<Color32>,
) -> ChromeButtonVisuals {
    let secondary_fill = Color32::TRANSPARENT;
    let base_fill = match kind {
        ButtonKind::Primary => colors.bg_active,
        ButtonKind::Secondary => secondary_fill,
        ButtonKind::Accent => colors.accent,
        ButtonKind::Danger => colors.danger,
        ButtonKind::Ghost => Color32::TRANSPARENT,
    };
    let fill = if state.disabled && matches!(kind, ButtonKind::Ghost) {
        Color32::TRANSPARENT
    } else if state.disabled {
        colors.bg_active.gamma_multiply(0.55)
    } else if state.down {
        match kind {
            ButtonKind::Accent | ButtonKind::Danger => base_fill.gamma_multiply(0.90),
            ButtonKind::Secondary | ButtonKind::Ghost => colors.bg_active,
            ButtonKind::Primary => base_fill.lerp_to_gamma(colors.text_primary, 0.12),
        }
    } else {
        base_fill
    };
    let text = if state.disabled {
        colors.text_dim
    } else if let Some(preferred) = preferred_text {
        preferred
    } else {
        match kind {
            ButtonKind::Accent | ButtonKind::Danger => colors.text_on(base_fill),
            ButtonKind::Secondary => colors.text_primary,
            ButtonKind::Primary | ButtonKind::Ghost => colors.text_primary,
        }
    };
    let stroke = match kind {
        ButtonKind::Ghost => egui::Stroke::NONE,
        ButtonKind::Secondary => {
            egui::Stroke::new(1.0_f32, colors.border.lerp_to_gamma(colors.text_dim, 0.45))
        }
        _ => egui::Stroke::NONE,
    };
    let surface = crate::ui::foreground::Surface::over(parent, fill);
    ChromeButtonVisuals {
        fill,
        text: surface.foreground(text, crate::ui::foreground::Role::SmallText),
        stroke,
    }
}

const GLYPH_MIN_W: f32 = style::BUTTON_H_MD - style::SPACE_SM;

/// A titlebar or toolbar glyph: a compact Ghost kit button with a tooltip.
/// Style the label (size, colour) on the `KitButton` passed in. It sits on the
/// toolbar surface unless the caller names another with `surface`.
pub(crate) fn toolbar_button(
    ui: &mut egui::Ui,
    mut button: KitButton<'_>,
    hover_text: &str,
) -> egui::Response {
    button.surface = button.surface.or(Some(button.colors.bg_toolbar));
    ui.add(
        button
            .kind(ButtonKind::Ghost)
            .compact()
            .min_width_at_least(GLYPH_MIN_W),
    )
    .on_hover_text(hover_text)
}

pub(crate) fn icon_button(
    ui: &mut egui::Ui,
    icon: &str,
    hover_text: &str,
    colors: &Colors,
) -> egui::Response {
    toolbar_button(
        ui,
        KitButton::new(icon, colors).text_color(colors.text_dim),
        hover_text,
    )
}

/// Full-width option button for `NotifyKind::Choice` lists.
///
/// Paints a fixed-height rect with a centered label and an optional
/// right-gutter shortcut hint. The focused option receives an accent fill;
/// non-focused options lift slightly on hover.
pub(crate) fn choice_button(
    ui: &mut egui::Ui,
    label: &str,
    shortcut_hint: &str,
    focused: bool,
    colors: &Colors,
) -> egui::Response {
    let width = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(
        Vec2::new(width, style::BUTTON_H_CHOICE),
        egui::Sense::click(),
    );

    let (bg, fg, hint_color) = if focused {
        (
            colors.accent,
            colors.text_on(colors.accent),
            colors.text_on(colors.accent),
        )
    } else {
        (colors.bg_hover, colors.text_primary, colors.text_dim)
    };
    let actual_bg = if resp.hovered() && !focused {
        colors.bg_active
    } else {
        bg
    };

    let amount = style::press_amount(&resp);
    let rect = egui::Rect::from_center_size(
        rect.center(),
        rect.size() * (1.0 - (1.0 - style::PRESS_SCALE) * amount),
    );
    let actual_bg = actual_bg.lerp_to_gamma(actual_bg.gamma_multiply(0.85), amount);
    let padding = style::SPACE_MD;
    let hint = crate::ui::text::elided_galley(
        ui,
        shortcut_hint,
        egui::FontId::proportional(style::TEXT_CAPTION),
        (rect.width() - padding * 2.0).max(0.0) / 3.0,
        crate::ui::text::Side::Trailing,
    );
    let hint_reserved = if shortcut_hint.is_empty() {
        0.0
    } else {
        hint.size().x + style::SPACE_SM
    };
    // Symmetric reservation retains the centered label without colliding with the hint.
    let label = crate::ui::text::elided_galley(
        ui,
        label,
        egui::FontId::proportional(style::TEXT_BODY),
        (rect.width() - (padding + hint_reserved) * 2.0).max(0.0),
        crate::ui::text::Side::Trailing,
    );
    let painter = ui.painter();
    let surface = crate::ui::foreground::Surface::over(colors.bg_sidebar, actual_bg);
    let fg = surface.foreground(fg, crate::ui::foreground::Role::SmallText);
    let hint_color = surface.foreground(hint_color, crate::ui::foreground::Role::SmallText);
    painter.rect_filled(rect, style::RADIUS_MD, actual_bg);
    crate::ui::snap::galley_snapped(painter, rect.center() - label.size() / 2.0, label, fg);
    if !shortcut_hint.is_empty() {
        crate::ui::snap::galley_snapped(
            painter,
            egui::pos2(
                rect.right() - padding - hint.size().x,
                rect.center().y - hint.size().y / 2.0,
            ),
            hint,
            hint_color,
        );
    }
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

pub(crate) fn copy_button(
    ui: &mut egui::Ui,
    id: egui::Id,
    text: &str,
    colors: &Colors,
) -> egui::Response {
    let now = ui.ctx().input(|i| i.time);
    let copied_at: Option<f64> = ui.ctx().memory(|m| m.data.get_temp(id));
    let just_copied = copied_at.is_some_and(|t| now - t < 2.0);
    let feedback = style::micro_motion(
        ui.ctx(),
        id.with("copy_feedback"),
        just_copied,
        style::DURATION_MICRO_MS,
    );
    let resp = ui
        .add(
            KitButton::new("", colors)
                .kind(ButtonKind::Ghost)
                .compact()
                .min_size(egui::vec2(
                    style::BUTTON_H_MD - style::SPACE_SM,
                    style::BUTTON_H_MD - style::SPACE_SM,
                )),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(format!("Copy `{text}`"));
    resp.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), "Copy")
    });
    for (icon, opacity, color) in [
        ("\u{f0c5}", 1.0 - feedback, ui.visuals().text_color()),
        ("✓", feedback, ui.visuals().selection.stroke.color),
    ] {
        if opacity > 0.0 {
            crate::ui::snap::text_snapped(
                ui.painter(),
                resp.rect.center(),
                Align2::CENTER_CENTER,
                icon,
                egui::FontId::proportional(style::TEXT_CAPTION),
                color.gamma_multiply(opacity),
            );
        }
    }
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

/// Keep egui's sizing, accessibility, focus and click semantics, applying the
/// press transform only to the shapes this button painted, never its hit box.
fn tactile_button(
    ui: &mut egui::Ui,
    button: egui::Button<'_>,
    hover_fill: Color32,
    active_fill: Color32,
    parent: Color32,
) -> egui::Response {
    let layer = ui.layer_id();
    let start = ui
        .ctx()
        .graphics(|g| g.get(layer).map_or(0, |p| p.all_entries().len()));
    let backdrop = ui.painter().add(egui::Shape::Noop);
    let response = ui.add(button);
    let amount = style::press_amount(&response);
    if response.hovered() && ui.is_enabled() && amount == 0.0 {
        ui.ctx().graphics_mut(|g| {
            if let Some(list) = g.get_mut(layer) {
                let end = list.all_entries().len();
                for index in start..end {
                    list.mutate_shape(egui::layers::ShapeIdx(index), |entry| {
                        if let egui::Shape::Rect(rect) = &mut entry.shape {
                            rect.fill = hover_fill;
                        }
                    });
                }
            }
        });
    }
    if amount > 0.0 {
        let scale = 1.0 - (1.0 - style::PRESS_SCALE) * amount;
        let transform = egui::emath::TSTransform::from_translation(
            response.rect.center().to_vec2() * (1.0 - scale),
        ) * egui::emath::TSTransform::from_scaling(scale);
        ui.ctx().graphics_mut(|g| {
            if let Some(list) = g.get_mut(layer) {
                let end = list.all_entries().len();
                let mut has_fill = false;
                for index in start..end {
                    list.mutate_shape(egui::layers::ShapeIdx(index), |clipped| {
                        if let egui::Shape::Rect(shape) = &mut clipped.shape {
                            has_fill = true;
                            shape.fill = shape.fill.lerp_to_gamma(active_fill, amount);
                        }
                    });
                }
                if !has_fill {
                    list.set(
                        backdrop,
                        ui.clip_rect(),
                        egui::Shape::rect_filled(
                            response.rect,
                            style::RADIUS_SM,
                            active_fill.gamma_multiply(amount),
                        ),
                    );
                }
                list.transform_range(
                    egui::layers::ShapeIdx(start),
                    egui::layers::ShapeIdx(end),
                    transform,
                );
            }
        });
    }
    // Egui applies disabled/hover tints while creating shapes. Resolve after
    // that paint step (and press interpolation), against the emitted fill.
    ui.ctx().graphics_mut(|graphics| {
        if let Some(list) = graphics.get_mut(layer) {
            let end = list.all_entries().len();
            let mut fill = parent;
            for entry in list.all_entries().skip(start) {
                if let egui::Shape::Rect(rect) = &entry.shape {
                    if rect.rect.contains(response.rect.center()) {
                        fill = fill.blend(rect.fill);
                    }
                }
            }
            let surface = crate::ui::foreground::Surface::over(Color32::BLACK, fill);
            for index in start..end {
                list.mutate_shape(egui::layers::ShapeIdx(index), |entry| {
                    if let egui::Shape::Text(text) = &mut entry.shape {
                        let preferred = text
                            .override_text_color
                            .or_else(|| {
                                text.galley
                                    .job
                                    .sections
                                    .first()
                                    .map(|s| s.format.color)
                                    .filter(|c| *c != Color32::PLACEHOLDER)
                            })
                            .unwrap_or(text.fallback_color);
                        text.override_text_color = Some(
                            surface.foreground(preferred, crate::ui::foreground::Role::SmallText),
                        );
                    }
                });
            }
        }
    });
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run `add` for `frames` frames, moving the pointer to `hover` (when set)
    /// from the second frame on so egui resolves hover from the prior layout.
    fn render(
        frames: usize,
        hover: Option<egui::Pos2>,
        mut add: impl FnMut(&mut egui::Ui) -> egui::Response,
    ) -> (egui::FullOutput, egui::Rect) {
        let host = crate::testing::HostHarness::new();
        let ctx = &host.app.ctx;
        let mut rect = egui::Rect::NOTHING;
        let mut output = None;
        for frame in 0..frames {
            let mut events = Vec::new();
            if let Some(pos) = hover.filter(|_| frame > 0) {
                events.push(egui::Event::PointerMoved(pos));
            }
            output = Some(ctx.run_ui(
                egui::RawInput {
                    time: Some(frame as f64 * 0.1),
                    events,
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show_inside(ui, |ui| {
                        rect = add(ui).rect;
                    });
                },
            ));
        }
        (output.unwrap(), rect)
    }

    fn fills_at(output: &egui::FullOutput, point: egui::Pos2) -> Vec<egui::Color32> {
        output
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Rect(r) if r.rect.contains(point) => Some(r.fill),
                _ => None,
            })
            .collect()
    }

    fn label_shape(output: &egui::FullOutput, label: &str) -> (egui::FontId, Color32) {
        output
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Text(text) if text.galley.job.text.starts_with(label) => {
                    let section = &text.galley.job.sections[0];
                    Some((
                        section.format.font_id.clone(),
                        text.override_text_color.unwrap_or(section.format.color),
                    ))
                }
                _ => None,
            })
            .expect("painted label")
    }

    #[test]
    fn kit_button_size_classes_are_independent_of_parent_spacing() {
        let host = crate::testing::HostHarness::new();
        let mut heights = Vec::new();
        for _ in 0..3 {
            heights.clear();
            let _ = host.app.ctx.run_ui(egui::RawInput::default(), |ui| {
                egui::CentralPanel::default().show_inside(ui, |ui| {
                    ui.spacing_mut().interact_size.y = 80.0;
                    for size in [ButtonSize::Sm, ButtonSize::Md, ButtonSize::Lg] {
                        let response =
                            ui.add(KitButton::new("Action", &host.app.colors).size(size));
                        heights.push(response.rect.height());
                    }
                });
            });
        }
        assert_eq!(
            heights,
            vec![style::BUTTON_H_SM, style::BUTTON_H_MD, style::BUTTON_H_LG]
        );
    }

    #[test]
    fn chrome_button_small_text_meets_contrast_in_every_state() {
        for name in crate::ui::theme::preset_names() {
            let colors = Colors::from_config(&crate::ui::theme::preset_colors(name).unwrap());
            for parent in [colors.bg_sidebar, colors.bg_active, colors.bg_darkest] {
                for kind in [
                    ButtonKind::Primary,
                    ButtonKind::Secondary,
                    ButtonKind::Accent,
                    ButtonKind::Danger,
                    ButtonKind::Ghost,
                ] {
                    for disabled in [false, true] {
                        for down in [false, true] {
                            let visuals = chrome_button_visuals(
                                kind,
                                &colors,
                                ChromeButtonState { disabled, down },
                                parent,
                                None,
                            );
                            let fill = parent.blend(visuals.fill);
                            let ratio = crate::ui::theme::contrast(
                                crate::ui::theme::luminance(fill),
                                crate::ui::theme::luminance(fill.blend(visuals.text)),
                            );
                            assert!(
                                ratio >= 4.5,
                                "{name}: {kind:?} on {parent:?} disabled={disabled} down={down} contrast={ratio}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn selected_paint_is_order_independent() {
        let colors = crate::testing::HostHarness::new().app.colors;
        let fill_of = |first_selected: bool| {
            let (output, rect) = render(2, None, |ui| {
                let base = KitButton::new("Tab", &colors);
                let button = if first_selected {
                    base.selected(true).kind(ButtonKind::Ghost)
                } else {
                    base.kind(ButtonKind::Ghost).selected(true)
                };
                ui.add(button)
            });
            fills_at(&output, rect.center())
        };
        assert_eq!(
            fill_of(true),
            fill_of(false),
            "builder order must not matter"
        );
        assert!(
            fill_of(true).contains(&colors.accent),
            "selected paints Accent"
        );
    }

    #[test]
    fn label_style_is_preserved_not_flattened() {
        let colors = crate::testing::HostHarness::new().app.colors;
        let (plain, _) = render(2, None, |ui| ui.add(KitButton::new("Label", &colors)));
        let (styled, _) = render(2, None, |ui| {
            ui.add(
                KitButton::new("Label", &colors)
                    .kind(ButtonKind::Ghost)
                    .text_size(17.0)
                    .monospace()
                    .text_color(colors.accent),
            )
        });
        let (plain_font, plain_color) = label_shape(&plain, "Label");
        let (font, color) = label_shape(&styled, "Label");
        assert_eq!(plain_font.size, style::TEXT_CONTROL);
        assert_eq!(font.size, 17.0, "text_size is honoured");
        assert_eq!(
            font.family,
            egui::FontFamily::Monospace,
            "monospace is honoured"
        );
        assert_ne!(color, plain_color, "text_color is honoured");
    }

    #[test]
    fn disabled_accent_is_forty_percent_of_rest_paint() {
        let colors = crate::testing::HostHarness::new().app.colors;
        let (rest, rect) = render(3, None, |ui| {
            ui.add(KitButton::new("Action", &colors).kind(ButtonKind::Accent))
        });
        let (disabled, disabled_rect) = render(3, None, |ui| {
            ui.add_enabled(
                false,
                KitButton::new("Action", &colors).kind(ButtonKind::Accent),
            )
        });
        assert_eq!(rect, disabled_rect);
        assert_eq!(
            fills_at(&disabled, rect.center()).last().copied(),
            fills_at(&rest, rect.center())
                .last()
                .map(|color| color.gamma_multiply(0.4))
        );
        assert_eq!(
            label_shape(&disabled, "Action").1,
            label_shape(&rest, "Action").1.gamma_multiply(0.4)
        );
    }

    #[test]
    fn filled_accent_changes_color_on_hover() {
        let colors = crate::testing::HostHarness::new().app.colors;
        let (rest, rect) = render(3, None, |ui| {
            ui.add(KitButton::new("Action", &colors).kind(ButtonKind::Accent))
        });
        let (hover, hover_rect) = render(3, Some(rect.center()), |ui| {
            ui.add(KitButton::new("Action", &colors).kind(ButtonKind::Accent))
        });
        assert_eq!(rect.size(), hover_rect.size());
        assert_ne!(
            fills_at(&rest, rect.center()),
            fills_at(&hover, hover_rect.center())
        );
    }

    #[test]
    fn ghost_menu_items_highlight_on_hover() {
        let colors = crate::testing::HostHarness::new().app.colors;
        let hovered_fill = colors.bg_hover;
        let (rest, rect) = render(3, None, |ui| menu_item(ui, "Rename", &colors));
        assert!(
            !fills_at(&rest, rect.center()).contains(&hovered_fill),
            "no highlight at rest"
        );
        let (hover, rect) = render(3, Some(egui::pos2(40.0, 20.0)), |ui| {
            menu_item(ui, "Rename", &colors)
        });
        assert!(
            rect.contains(egui::pos2(40.0, 20.0)),
            "pointer must land on the item, got {rect:?}"
        );
        assert!(
            fills_at(&hover, rect.center()).contains(&hovered_fill),
            "menu items must paint the theme hover fill"
        );
    }

    #[test]
    fn secondary_is_distinguishable_from_primary_and_the_card_surface() {
        for name in crate::ui::theme::preset_names() {
            let colors = Colors::from_config(&crate::ui::theme::preset_colors(name).unwrap());
            let state = ChromeButtonState {
                disabled: false,
                down: false,
            };
            let fill =
                |kind| chrome_button_visuals(kind, &colors, state, colors.bg_active, None).fill;
            assert_ne!(
                fill(ButtonKind::Secondary),
                colors.bg_active,
                "{name}: Secondary vs Card"
            );
            assert_ne!(
                fill(ButtonKind::Secondary),
                fill(ButtonKind::Accent),
                "{name}"
            );
        }
    }
}

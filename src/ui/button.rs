use egui::{Align2, Color32, RichText, Vec2};

use crate::ui::style;
use crate::ui::theme::Colors;

/// Fill and emphasis of a button. `Primary` is the declared default kind: the
/// accent-filled, strongest button in every theme.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ButtonKind {
    #[default]
    Primary,
    Secondary,
    Danger,
    /// No fill; a faint outline at rest so it reads as a button, and the
    /// theme's hover/press fill on interaction. Menu items (`menu_item`) and
    /// compact glyph controls drop the outline (`flat`).
    Ghost,
}

#[derive(Clone, Copy, Default)]
pub(crate) struct ChromeButtonState {
    pub(crate) disabled: bool,
    pub(crate) down: bool,
    /// Active member of a group. Repaints Primary, Secondary and Ghost with
    /// the active fill and an accent outline; Danger keeps its fill and
    /// gains an inset outline. Never changes the layout box.
    pub(crate) selected: bool,
    /// Ghost without its rest outline: menu items and glyph controls.
    pub(crate) flat: bool,
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

    /// The class a control of at least `height` falls in. A plugin button
    /// stretched taller than its class takes the type and padding of the
    /// resulting class.
    fn from_height(height: f32) -> Self {
        if height >= style::BUTTON_H_LG {
            Self::Lg
        } else if height >= style::BUTTON_H_MD {
            Self::Md
        } else {
            Self::Sm
        }
    }

    fn font_size(self) -> f32 {
        match self {
            Self::Sm => style::TEXT_CAPTION,
            Self::Md => style::TEXT_CONTROL,
            Self::Lg => style::TEXT_BODY,
        }
    }

    fn pad_x(self) -> f32 {
        match self {
            Self::Sm => style::BUTTON_PAD_X_SM,
            Self::Md => style::BUTTON_PAD_X_MD,
            Self::Lg => style::BUTTON_PAD_X_LG,
        }
    }

    fn radius(self) -> egui::CornerRadius {
        match self {
            Self::Lg => style::RADIUS_MD,
            Self::Sm | Self::Md => style::RADIUS_SM,
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
    flat: bool,
    surface: Option<Color32>,
    hint: Option<&'a str>,
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
            flat: false,
            surface: None,
            hint: None,
        }
    }

    /// A dim shortcut hint painted inside the button after the label. It is
    /// part of the one-line button box, so it never wraps or changes the
    /// button's height.
    pub(crate) fn hint(mut self, hint: &'a str) -> Self {
        self.hint = Some(hint);
        self
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
    /// Drop the Ghost rest outline. Menu items and compact glyph controls are
    /// chrome, not buttons that must read as buttons at rest.
    pub(crate) fn flat(mut self) -> Self {
        self.flat = true;
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

impl KitButton<'_> {
    /// The size class that sets type, padding and radius: the declared size,
    /// raised when `min_size` stretches the button into a taller class.
    fn class(&self) -> ButtonSize {
        ButtonSize::from_height(self.min_size.y.max(self.size.height()))
    }

    fn padding(&self) -> Vec2 {
        if self.compact {
            Vec2::ZERO
        } else {
            egui::vec2(self.class().pad_x(), style::SPACE_XS)
        }
    }

    fn label_font(&self) -> egui::FontId {
        let label_size = self.text.size.unwrap_or_else(|| self.class().font_size());
        if self.text.monospace {
            egui::FontId::monospace(label_size)
        } else {
            crate::ui::theme::font_medium(label_size)
        }
    }

    /// Width of the button laid out on one line: the same font and padding
    /// `ui` paints with, so a container sized to it never makes the label
    /// wrap or elide. Callers that reserve room for a button (toast
    /// actions) must measure through this, not re-derive the metrics.
    pub(crate) fn natural_width(&self, ui: &egui::Ui) -> f32 {
        let font = self.label_font();
        let text = crate::ui::text::elided_galley(
            ui,
            self.label,
            font,
            f32::MAX,
            crate::ui::text::Side::Trailing,
        )
        .size()
        .x;
        // Rounded up once, so a box sized to this never elides to float noise.
        (text + self.hint_width(ui) + self.padding().x * 2.0)
            .ceil()
            .max(self.min_size.x)
    }

    /// Room the hint takes after the label: the gap plus its own text.
    fn hint_width(&self, ui: &egui::Ui) -> f32 {
        self.hint.map_or(0.0, |hint| {
            let font = egui::FontId::proportional(style::TEXT_HINT);
            style::SPACE_SM
                + crate::ui::text::elided_galley(
                    ui,
                    hint,
                    font,
                    f32::MAX,
                    crate::ui::text::Side::Trailing,
                )
                .size()
                .x
        })
    }
}

impl egui::Widget for KitButton<'_> {
    #[expect(
        clippy::disallowed_methods,
        reason = "the kit button is the one place that constructs egui::Button"
    )]
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        let kind = self.kind;
        log_first_paint(kind, self.size, self.compact);
        let parent = self.surface.unwrap_or(ui.visuals().panel_fill);
        let disabled = !ui.is_enabled();
        let state = ChromeButtonState {
            disabled,
            down: false,
            selected: self.selected,
            flat: self.flat || self.compact,
        };
        let visuals = chrome_button_visuals(kind, &self.colors, state, parent, self.text.color);
        let active = chrome_button_visuals(
            kind,
            &self.colors,
            ChromeButtonState {
                down: true,
                ..state
            },
            parent,
            self.text.color,
        );
        let padding = self.padding();
        let font = self.label_font();
        let shown = if matches!(self.wrap, egui::TextWrapMode::Extend) && !self.compact {
            crate::ui::text::elide(
                ui,
                self.label,
                font.clone(),
                (ui.available_width() - padding.x * 2.0 - self.hint_width(ui)).max(0.0),
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
            let available = ui.available_width();
            // With no room the clamp below wins; skip measuring the label.
            if !self.compact && available > 0.0 {
                min_size.x = self.natural_width(ui);
            }
            min_size.x = min_size.x.min(available);
        }
        if !self.compact {
            min_size.y = min_size.y.max(self.size.height());
        }
        // Keep the frame geometry identical for every paint state.
        let stroke = if visuals.stroke.width == 0.0 {
            egui::Stroke::new(1.0_f32, Color32::TRANSPARENT)
        } else {
            visuals.stroke
        };
        // Growing atoms on both sides centre the label in any stretched box;
        // egui otherwise aligns it from the parent layout (left in a row).
        use egui::IntoAtoms as _;
        let atoms = match self.hint {
            Some(hint) => (
                egui::Atom::grow(),
                text,
                egui::Atom::custom(
                    egui::Id::new("kit_button_hint_gap"),
                    egui::vec2(style::SPACE_SM, 0.0),
                ),
                RichText::new(hint)
                    .size(style::TEXT_HINT)
                    .color(visuals.text.gamma_multiply(0.7)),
                egui::Atom::grow(),
            )
                .into_atoms(),
            None => (egui::Atom::grow(), text, egui::Atom::grow()).into_atoms(),
        };
        let button = egui::Button::new(atoms)
            .stroke(stroke)
            .corner_radius(self.class().radius())
            .fill(visuals.fill)
            .min_size(min_size)
            .wrap_mode(self.wrap);
        let hover_fill = if repaints_when_selected(kind, self.selected) {
            visuals.fill.lerp_to_gamma(self.colors.text_primary, 0.06)
        } else {
            match kind {
                ButtonKind::Secondary | ButtonKind::Ghost => self.colors.bg_hover,
                ButtonKind::Primary | ButtonKind::Danger => strong_hover(visuals.fill),
            }
        };
        let response = ui
            .scope(|ui| {
                if !self.compact {
                    ui.spacing_mut().interact_size.y = self.size.height();
                }
                ui.spacing_mut().button_padding = padding;
                // The grow atoms must not add a gap to the natural width.
                ui.spacing_mut().icon_spacing = 0.0;
                tactile_button(
                    ui,
                    button,
                    hover_fill,
                    active.fill,
                    parent,
                    disabled.then_some(visuals),
                )
                .on_hover_cursor(egui::CursorIcon::PointingHand)
            })
            .inner;
        // Keyboard focus: an accent ring just outside the frame, so it reads on
        // every fill (an accent Primary included) and never moves the frame.
        // Where the clip would cut an outside ring (a button at the window or
        // panel edge), the ring goes inside the frame so it stays whole.
        if response.has_focus() {
            log_first_focus_ring();
            let kind = if ui
                .clip_rect()
                .contains_rect(response.rect.expand(FOCUS_RING_WIDTH))
            {
                egui::StrokeKind::Outside
            } else {
                egui::StrokeKind::Inside
            };
            ui.painter().rect_stroke(
                response.rect,
                self.class().radius(),
                egui::Stroke::new(FOCUS_RING_WIDTH, self.colors.accent),
                kind,
            );
        }
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), self.label)
        });
        response
    }
}

/// Width of the keyboard-focus ring painted outside a focused kit button.
const FOCUS_RING_WIDTH: f32 = 2.0;

fn log_first_focus_ring() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| log::info!("ui: kit button painted its keyboard-focus ring"));
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
            .flat()
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

/// Hover and pressed fills of the strong (filled) kinds. Hover moves away from
/// the fill's own lightness so it stays visible on light accents (dusk) and on
/// dark ones (dawn); pressed is always clearly darker than hover.
fn strong_hover(fill: Color32) -> Color32 {
    let toward = if crate::ui::theme::luminance(fill) > 0.15 {
        Color32::BLACK
    } else {
        Color32::WHITE
    };
    fill.lerp_to_gamma(toward, 0.12)
}

fn strong_pressed(fill: Color32) -> Color32 {
    fill.lerp_to_gamma(Color32::BLACK, 0.2)
}

/// Selection repaints the quiet kinds (active fill, accent outline). Danger
/// is already the strongest paint, so it keeps its fill.
fn repaints_when_selected(kind: ButtonKind, selected: bool) -> bool {
    selected
        && matches!(
            kind,
            ButtonKind::Primary | ButtonKind::Secondary | ButtonKind::Ghost
        )
}

/// `parent` is the surface the button sits on; transparent and disabled fills
/// resolve contrast over it. `preferred_text` overrides the kind's text colour
/// before the contrast correction.
///
/// Primary is the accent-filled strongest button. Selected quiet kinds use the
/// active fill with an accent outline, so a selected item never reads as
/// Primary. Disabled is a neutral dim chip that matches no enabled kind. Every
/// state paints inside the same 1px frame, so none changes the layout box.
fn chrome_button_visuals(
    kind: ButtonKind,
    colors: &Colors,
    state: ChromeButtonState,
    parent: Color32,
    preferred_text: Option<Color32>,
) -> ChromeButtonVisuals {
    let selected_quiet = repaints_when_selected(kind, state.selected);
    let base_fill = if selected_quiet {
        colors.bg_active
    } else {
        match kind {
            ButtonKind::Primary => colors.accent,
            ButtonKind::Danger => colors.danger,
            ButtonKind::Secondary | ButtonKind::Ghost => Color32::TRANSPARENT,
        }
    };
    let fill = if state.disabled && matches!(kind, ButtonKind::Ghost) {
        Color32::TRANSPARENT
    } else if state.disabled {
        colors.bg_active.gamma_multiply(DISABLED_FILL)
    } else if state.down {
        if selected_quiet {
            base_fill.lerp_to_gamma(colors.text_primary, 0.12)
        } else {
            match kind {
                ButtonKind::Primary | ButtonKind::Danger => strong_pressed(base_fill),
                ButtonKind::Secondary | ButtonKind::Ghost => colors.bg_active,
            }
        }
    } else {
        base_fill
    };
    let text = if state.disabled {
        colors.text_dim
    } else if let Some(preferred) = preferred_text {
        preferred
    } else if selected_quiet {
        colors.text_primary
    } else {
        match kind {
            ButtonKind::Primary | ButtonKind::Danger => colors.text_on(base_fill),
            ButtonKind::Secondary | ButtonKind::Ghost => colors.text_primary,
        }
    };
    let stroke = if state.disabled {
        egui::Stroke::NONE
    } else if selected_quiet {
        egui::Stroke::new(1.0_f32, colors.accent)
    } else if state.selected {
        egui::Stroke::new(1.0_f32, colors.text_on(base_fill))
    } else {
        match kind {
            ButtonKind::Secondary => {
                egui::Stroke::new(1.0_f32, colors.border.lerp_to_gamma(colors.text_dim, 0.45))
            }
            ButtonKind::Ghost if !state.flat => {
                egui::Stroke::new(1.0_f32, colors.border.lerp_to_gamma(colors.text_dim, 0.3))
            }
            _ => egui::Stroke::NONE,
        }
    };
    let surface = crate::ui::foreground::Surface::over(parent, fill);
    ChromeButtonVisuals {
        fill,
        text: surface.foreground(text, crate::ui::foreground::Role::SmallText),
        stroke,
    }
}

/// Opacity of the neutral disabled chip over the surface, and of its label.
/// Disabled labels are inactive text: `layout_report::INACTIVE_TEXT_MIN`
/// governs their legibility instead of the SmallText 4.5:1 minimum.
const DISABLED_FILL: f32 = 0.4;
const DISABLED_LABEL: f32 = 0.55;

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
    disabled_paint: Option<ChromeButtonVisuals>,
) -> egui::Response {
    let layer = ui.layer_id();
    let start = ui
        .ctx()
        .graphics(|g| g.get(layer).map_or(0, |p| p.all_entries().len()));
    let backdrop = ui.painter().add(egui::Shape::Noop);
    let response = ui.add(button);
    // Glyph raster bounds can round beyond a fractional hit rectangle. Keep
    // every emitted shape within the control's allocated box at all scales.
    ui.ctx().graphics_mut(|graphics| {
        if let Some(list) = graphics.get_mut(layer) {
            let end = list.all_entries().len();
            for index in start..end {
                list.mutate_shape(egui::layers::ShapeIdx(index), |entry| {
                    entry.clip_rect = entry.clip_rect.intersect(response.rect);
                });
            }
        }
    });
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
            if let Some(rest) = disabled_paint {
                let surface = crate::ui::foreground::Surface::over(parent, rest.fill);
                let rest_text =
                    surface.foreground(rest.text, crate::ui::foreground::Role::SmallText);
                for index in start..end {
                    list.mutate_shape(egui::layers::ShapeIdx(index), |entry| {
                        match &mut entry.shape {
                            egui::Shape::Rect(rect) => {
                                rect.fill = rest.fill;
                                rect.stroke.color = Color32::TRANSPARENT;
                            }
                            egui::Shape::Text(text) => {
                                text.override_text_color =
                                    Some(rest_text.gamma_multiply(DISABLED_LABEL));
                            }
                            _ => {}
                        }
                    });
                }
                return;
            }
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
                    ButtonKind::Danger,
                    ButtonKind::Ghost,
                ] {
                    for (disabled, selected) in [(false, false), (false, true), (true, false)] {
                        for down in [false, true] {
                            let visuals = chrome_button_visuals(
                                kind,
                                &colors,
                                ChromeButtonState {
                                    disabled,
                                    down,
                                    selected,
                                    ..Default::default()
                                },
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
                                "{name}: {kind:?} on {parent:?} disabled={disabled} selected={selected} down={down} contrast={ratio}"
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
            fill_of(true).contains(&colors.bg_active),
            "selected Ghost paints the active fill"
        );
        assert!(
            !fill_of(true).contains(&colors.accent),
            "selected must not paint the Primary accent fill"
        );
    }

    #[test]
    fn selection_repaints_quiet_kinds_and_keeps_strong_kinds() {
        let colors = crate::testing::HostHarness::new().app.colors;
        for kind in [
            ButtonKind::Primary,
            ButtonKind::Secondary,
            ButtonKind::Ghost,
            ButtonKind::Danger,
        ] {
            let (output, rect) = render(3, None, |ui| {
                ui.add(KitButton::new("Tab", &colors).kind(kind).selected(true))
            });
            let expected = match kind {
                ButtonKind::Danger => colors.danger,
                _ => colors.bg_active,
            };
            assert_eq!(
                fills_at(&output, rect.center()).last(),
                Some(&expected),
                "selected {kind:?}"
            );
        }
    }

    /// Ian's ruling (stint 1216 round 2): Primary is the strongest button in
    /// every theme, light included, and a selected item is distinguishable
    /// from it.
    #[test]
    fn primary_is_strongest_and_selected_is_distinguishable_in_every_theme() {
        let ratio = |a: Color32, b: Color32| {
            crate::ui::theme::contrast(
                crate::ui::theme::luminance(a),
                crate::ui::theme::luminance(b),
            )
        };
        for name in crate::ui::theme::preset_names() {
            let colors = Colors::from_config(&crate::ui::theme::preset_colors(name).unwrap());
            let visuals = |kind, selected| {
                chrome_button_visuals(
                    kind,
                    &colors,
                    ChromeButtonState {
                        selected,
                        ..Default::default()
                    },
                    colors.bg_sidebar,
                    None,
                )
            };
            let primary = visuals(ButtonKind::Primary, false);
            assert_eq!(
                primary.fill, colors.accent,
                "{name}: Primary is accent-filled"
            );
            assert!(
                ratio(primary.fill, colors.bg_sidebar) >= 2.5,
                "{name}: Primary must stand out from the panel"
            );
            for kind in [
                ButtonKind::Primary,
                ButtonKind::Secondary,
                ButtonKind::Ghost,
            ] {
                let selected = visuals(kind, true);
                assert!(
                    ratio(primary.fill, selected.fill) >= 1.5,
                    "{name}: selected {kind:?} fill must differ from Primary"
                );
                assert_eq!(
                    selected.stroke.color, colors.accent,
                    "{name}: selected {kind:?} carries the accent outline"
                );
            }
        }
    }

    #[test]
    fn ghost_outline_is_visible_at_rest_but_fainter_than_secondary() {
        let ratio = |a: Color32, b: Color32| {
            crate::ui::theme::contrast(
                crate::ui::theme::luminance(a),
                crate::ui::theme::luminance(b),
            )
        };
        for name in crate::ui::theme::preset_names() {
            let colors = Colors::from_config(&crate::ui::theme::preset_colors(name).unwrap());
            let stroke = |kind, flat| {
                chrome_button_visuals(
                    kind,
                    &colors,
                    ChromeButtonState {
                        flat,
                        ..Default::default()
                    },
                    colors.bg_sidebar,
                    None,
                )
                .stroke
            };
            let panel = colors.bg_sidebar;
            let ghost = stroke(ButtonKind::Ghost, false);
            let secondary = stroke(ButtonKind::Secondary, false);
            assert_eq!(ghost.width, 1.0, "{name}");
            // Visible: a floor over the surface it sits on.
            let ghost_ratio = ratio(panel.blend(ghost.color), panel);
            assert!(
                ghost_ratio >= 1.2,
                "{name}: Ghost outline {ghost_ratio:.2} is invisible against the panel"
            );
            // Faint: never stronger than Secondary's outline.
            let secondary_ratio = ratio(panel.blend(secondary.color), panel);
            assert!(
                ghost_ratio <= secondary_ratio + 1e-3,
                "{name}: Ghost {ghost_ratio:.2} stronger than Secondary {secondary_ratio:.2}"
            );
            assert_eq!(
                stroke(ButtonKind::Ghost, true),
                egui::Stroke::NONE,
                "{name}"
            );
        }
    }

    /// Label text is centred in the control in both axes, at every size class.
    #[test]
    fn label_is_centred_in_every_size_class() {
        let colors = crate::testing::HostHarness::new().app.colors;
        for size in [ButtonSize::Sm, ButtonSize::Md, ButtonSize::Lg] {
            for min_width in [0.0_f32, 160.0] {
                let (output, rect) = render(3, None, |ui| {
                    ui.add(
                        KitButton::new("Centre", &colors)
                            .size(size)
                            .min_width(min_width),
                    )
                });
                let text = output
                    .shapes
                    .iter()
                    .find_map(|s| match &s.shape {
                        egui::Shape::Text(t) if t.galley.job.text == "Centre" => Some(t),
                        _ => None,
                    })
                    .expect("painted label");
                let label = egui::Rect::from_min_size(text.pos, text.galley.size());
                assert!(
                    (label.center().x - rect.center().x).abs() <= 0.5,
                    "{size:?} min_width={min_width}: x {label:?} in {rect:?}"
                );
                assert!(
                    (label.center().y - rect.center().y).abs() <= 0.5,
                    "{size:?} min_width={min_width}: y {label:?} in {rect:?}"
                );
            }
        }
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
    fn stretched_button_uses_resulting_height_class_font() {
        let colors = crate::testing::HostHarness::new().app.colors;
        for (height, expected) in [
            (style::BUTTON_H_SM, style::TEXT_CAPTION),
            (style::BUTTON_H_MD, style::TEXT_CONTROL),
            (style::BUTTON_H_LG, style::TEXT_BODY),
        ] {
            let (output, rect) = render(3, None, |ui| {
                ui.add(
                    KitButton::new("Action", &colors)
                        .size(ButtonSize::Sm)
                        .min_size(egui::vec2(80.0, height)),
                )
            });
            assert_eq!(rect.height(), height);
            assert_eq!(label_shape(&output, "Action").0.size, expected);
        }
    }

    #[test]
    fn approved_button_fills_cover_all_kinds() {
        let colors = crate::testing::HostHarness::new().app.colors;
        for (kind, rest, hover, pressed) in [
            (
                ButtonKind::Primary,
                colors.accent,
                strong_hover(colors.accent),
                strong_pressed(colors.accent),
            ),
            (
                ButtonKind::Secondary,
                Color32::TRANSPARENT,
                colors.bg_hover,
                colors.bg_active,
            ),
            (
                ButtonKind::Ghost,
                Color32::TRANSPARENT,
                colors.bg_hover,
                colors.bg_active,
            ),
            (
                ButtonKind::Danger,
                colors.danger,
                strong_hover(colors.danger),
                strong_pressed(colors.danger),
            ),
        ] {
            let (normal, rect) = render(3, None, |ui| {
                ui.add(KitButton::new("Action", &colors).kind(kind))
            });
            let (hovered, _) = render(3, Some(rect.center()), |ui| {
                ui.add(KitButton::new("Action", &colors).kind(kind))
            });
            assert_eq!(
                fills_at(&normal, rect.center()).last(),
                Some(&rest),
                "{kind:?} rest"
            );
            assert_eq!(
                fills_at(&hovered, rect.center()).last(),
                Some(&hover),
                "{kind:?} hover"
            );
            let down = chrome_button_visuals(
                kind,
                &colors,
                ChromeButtonState {
                    down: true,
                    ..Default::default()
                },
                colors.bg_sidebar,
                None,
            );
            assert_eq!(down.fill, pressed, "{kind:?} pressed");
        }
    }

    /// Disabled is a neutral chip with a faded label: the fade is exact, not
    /// just a recolour, so it cannot drift back to an enabled-looking label.
    #[test]
    fn disabled_primary_fades_the_label_and_uses_the_neutral_chip() {
        let colors = crate::testing::HostHarness::new().app.colors;
        let (rest, rect) = render(3, None, |ui| ui.add(KitButton::new("Action", &colors)));
        let (disabled, disabled_rect) = render(3, None, |ui| {
            ui.add_enabled(false, KitButton::new("Action", &colors))
        });
        assert_eq!(rect, disabled_rect, "layout box");
        assert_eq!(label_shape(&rest, "Action").1.a(), 255);
        let faded = label_shape(&disabled, "Action").1.a();
        let expected = (255.0 * DISABLED_LABEL).round() as i32;
        assert!(
            (i32::from(faded) - expected).abs() <= 1,
            "disabled label alpha {faded}, expected {expected}"
        );
        assert_eq!(
            fills_at(&disabled, rect.center()).last().copied(),
            Some(colors.bg_active.gamma_multiply(DISABLED_FILL)),
            "disabled chip"
        );
    }

    /// Ian's ruling (stint 1216 round 3): disabled is clearly dimmed in every
    /// kind, Ghost included, and never close to an enabled strongest kind.
    /// Distances are contrast ratios so the check holds in any theme.
    #[test]
    fn disabled_is_clearly_dimmed_and_far_from_every_enabled_kind() {
        let host = crate::testing::HostHarness::new();
        let colors = host.app.colors;
        let panel = host.app.ctx.global_style().visuals.panel_fill;
        let ratio = |a: Color32, b: Color32| {
            crate::ui::theme::contrast(
                crate::ui::theme::luminance(a),
                crate::ui::theme::luminance(b),
            )
        };
        let kinds = [
            ButtonKind::Primary,
            ButtonKind::Secondary,
            ButtonKind::Ghost,
            ButtonKind::Danger,
        ];
        let paint = |kind, enabled: bool| {
            let (out, rect) = render(3, None, |ui| {
                ui.add_enabled(enabled, KitButton::new("Action", &colors).kind(kind))
            });
            let fill = panel.blend(
                fills_at(&out, rect.center())
                    .last()
                    .copied()
                    .unwrap_or(Color32::TRANSPARENT),
            );
            (fill, label_shape(&out, "Action").1, rect)
        };
        for kind in kinds {
            let (rest_fill, rest_label, rect) = paint(kind, true);
            let (fill, label, disabled_rect) = paint(kind, false);
            assert_eq!(rect, disabled_rect, "{kind:?}: layout box");
            let legibility = |fill: Color32, label: Color32| ratio(fill.blend(label), fill);
            let (enabled, dimmed) = (legibility(rest_fill, rest_label), legibility(fill, label));
            assert!(
                enabled >= dimmed * 1.25,
                "{kind:?}: disabled label ({dimmed:.2}) must be clearly dimmer than enabled ({enabled:.2})"
            );
            assert!(
                dimmed >= crate::ui::layout_report::INACTIVE_TEXT_MIN,
                "{kind:?}: disabled label {dimmed:.2} under the inactive floor"
            );
        }
        let disabled_chip = paint(ButtonKind::Secondary, false).0;
        for strong in [ButtonKind::Primary, ButtonKind::Danger] {
            let enabled = paint(strong, true).0;
            assert!(
                ratio(disabled_chip, enabled) >= 1.5,
                "disabled chip {disabled_chip:?} is too close to enabled {strong:?} {enabled:?}"
            );
        }
    }

    /// Filled kinds change colour on hover, visibly, in every theme: dusk's
    /// light accent used to move only 8% toward white.
    #[test]
    fn filled_hover_and_pressed_are_visibly_distinct_in_every_theme() {
        let ratio = |a: Color32, b: Color32| {
            crate::ui::theme::contrast(
                crate::ui::theme::luminance(a),
                crate::ui::theme::luminance(b),
            )
        };
        for name in crate::ui::theme::preset_names() {
            let colors = Colors::from_config(&crate::ui::theme::preset_colors(name).unwrap());
            for fill in [colors.accent, colors.danger] {
                assert!(
                    ratio(fill, strong_hover(fill)) >= 1.08,
                    "{name}: hover {:?} too close to {fill:?}",
                    strong_hover(fill)
                );
                assert!(
                    ratio(strong_hover(fill), strong_pressed(fill)) >= 1.1,
                    "{name}: pressed too close to hover on {fill:?}"
                );
            }
        }
        let colors = crate::testing::HostHarness::new().app.colors;
        let (rest, rect) = render(3, None, |ui| ui.add(KitButton::new("Action", &colors)));
        let (hover, hover_rect) = render(3, Some(rect.center()), |ui| {
            ui.add(KitButton::new("Action", &colors))
        });
        assert_eq!(rect.size(), hover_rect.size());
        assert_eq!(
            fills_at(&hover, hover_rect.center()).last(),
            Some(&strong_hover(colors.accent))
        );
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
            let state = ChromeButtonState::default();
            let fill =
                |kind| chrome_button_visuals(kind, &colors, state, colors.bg_active, None).fill;
            assert_ne!(
                fill(ButtonKind::Secondary),
                colors.bg_active,
                "{name}: Secondary vs Card"
            );
            assert_ne!(
                fill(ButtonKind::Secondary),
                fill(ButtonKind::Primary),
                "{name}"
            );
        }
    }
}

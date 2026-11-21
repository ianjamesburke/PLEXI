//! A status panel: a one-line caption, a message, an optional busy mark and
//! the kit action row, on one bordered surface. Every state keeps this one
//! layout, so swapping the message or the actions never moves the caption
//! and the actions always sit in the same right-aligned row below the text.
//! Text wraps on word boundaries and never splits a token; the row stacks
//! its buttons before any label would elide.

use egui::{Color32, FontId, Rect, Shape, Stroke, vec2};

use super::bounded::{self, Presentation, Slot};
use super::dialog::{self, DialogAction};
use super::foreground::{Role, Surface};
use super::layout_report::{Action, Capture};
use super::style;
use super::text;
use super::theme::Colors;

/// Rows a message takes before it elides; the full text is on hover.
const MESSAGE_ROWS: usize = 3;

/// The border hue: what the panel asks of the user.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tone {
    Neutral,
    Accent,
    Danger,
}

pub(crate) struct StatusPanel<'a> {
    id: &'a str,
    caption: &'a str,
    message: &'a str,
    tone: Tone,
    busy: bool,
    actions: &'a [DialogAction<'a>],
}

impl<'a> StatusPanel<'a> {
    pub(crate) fn new(id: &'a str, caption: &'a str, message: &'a str) -> Self {
        Self {
            id,
            caption,
            message,
            tone: Tone::Neutral,
            busy: false,
            actions: &[],
        }
    }

    pub(crate) fn tone(mut self, tone: Tone) -> Self {
        self.tone = tone;
        self
    }

    /// Work is in progress with no measurable progress: an indeterminate mark
    /// sits at the trailing end of the message row.
    pub(crate) fn busy(mut self, busy: bool) -> Self {
        self.busy = busy;
        self
    }

    pub(crate) fn actions(mut self, actions: &'a [DialogAction<'a>]) -> Self {
        self.actions = actions;
        self
    }

    /// `backdrop` is the opaque fill the panel sits on. Returns the id of the
    /// clicked action.
    pub(crate) fn show(
        self,
        ui: &mut egui::Ui,
        backdrop: Color32,
        colors: &Colors,
    ) -> Option<&'a str> {
        static FIRST: std::sync::Once = std::sync::Once::new();
        FIRST.call_once(|| log::info!("ui: status panel {} first painted", self.id));
        let fill = colors.bg_toolbar;
        let surface = Surface::over(backdrop, fill);
        let border = match self.tone {
            Tone::Neutral => colors.border,
            Tone::Accent => colors.accent,
            Tone::Danger => colors.danger,
        };
        egui::Frame::new()
            .fill(fill)
            .stroke(style::hairline_border(border))
            .corner_radius(style::RADIUS_MD)
            .inner_margin(egui::Margin::symmetric(
                style::SPACE_MD as i8,
                style::SPACE_SM as i8,
            ))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = style::SPACE_XS;
                let mut capture = Capture::begin(ui.painter(), "status_panel");
                let width = ui.available_width();

                let caption = text::wrapped_galley(
                    ui,
                    self.caption,
                    FontId::proportional(style::TEXT_HINT),
                    surface.foreground(colors.text_dim, Role::SmallText),
                    width,
                    1,
                );
                let caption_rect = ui.add(egui::Label::new(caption.clone())).rect;
                capture.text_slot("caption", &caption, caption_rect);

                let font = FontId::proportional(style::TEXT_CAPTION);
                let row_height = ui.fonts_mut(|f| f.row_height(&font));
                let row = Rect::from_min_size(ui.cursor().min, vec2(width, row_height));
                let busy = [Slot {
                    id: "busy",
                    width: row_height,
                    compact: None,
                    required: true,
                }];
                let slots: &[Slot] = if self.busy { &busy } else { &[] };
                let (lane, mark) = match bounded::allocate(row, 0.0, style::SPACE_SM, slots) {
                    Ok(layout) => (
                        layout.text,
                        layout
                            .slots
                            .first()
                            .filter(|s| s.presentation == Presentation::Full)
                            .map(|s| s.rect),
                    ),
                    // Narrower than the mark itself: the message says it.
                    Err(_) => (row, None),
                };
                let message = text::wrapped_galley(
                    ui,
                    self.message,
                    font,
                    surface.foreground(colors.text_primary, Role::SmallText),
                    lane.width(),
                    MESSAGE_ROWS,
                );
                let elided = message.elided;
                let response = ui.add(egui::Label::new(message.clone()));
                capture.text_slot("message", &message, response.rect);
                if elided {
                    response.on_hover_text(self.message);
                }
                if let Some(mark) = mark {
                    paint_busy(
                        ui,
                        mark,
                        surface.foreground(colors.accent, Role::ControlMark),
                        surface.foreground(colors.border, Role::ControlMark),
                    );
                }

                let mut selected = None;
                let mut actions: Vec<Action> = Vec::new();
                if !self.actions.is_empty() {
                    ui.add_space(style::SPACE_SM);
                    (selected, actions) =
                        dialog::action_row(ui, self.id, self.actions, colors, surface.fill);
                }
                capture.finish(ui.painter(), ui.min_rect(), surface.fill, actions, vec![]);
                selected
            })
            .inner
    }
}

/// An indeterminate busy mark: a fixed arc turning once a second over a faint
/// track. Its length never changes, so every frame (and every screenshot)
/// shows it.
fn paint_busy(ui: &egui::Ui, rect: Rect, arc: Color32, track: Color32) {
    const POINTS: usize = 16;
    let radius = rect.height().min(rect.width()) / 2.0 - style::SPACE_XS / 2.0;
    let stroke = style::SPACE_XS / 2.0;
    let start = (ui.input(|i| i.time) * std::f64::consts::TAU) as f32;
    let span = 100_f32.to_radians();
    let points = (0..=POINTS)
        .map(|i| {
            let angle = start + span * i as f32 / POINTS as f32;
            rect.center() + radius * vec2(angle.cos(), angle.sin())
        })
        .collect::<Vec<_>>();
    let painter = ui.painter();
    painter.circle_stroke(rect.center(), radius, Stroke::new(stroke, track));
    painter.add(Shape::line(points, Stroke::new(stroke, arc)));
    // About 30 fps is smooth for one small arc; never a zero-delay loop.
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_millis(33));
}

//! The card block shared by sibling cards, atlas nodes and attention-queue bars.
//!
//! One layout + paint path so a state reads the same on every surface: a
//! colored strip and an animated icon say *what is happening*, the title and
//! detail say *to whom*. Text is wrapped on word boundaries against the real
//! font, capped at a line budget with a single trailing ellipsis, and painted
//! under a clip rect so nothing escapes the box.
//!
//! [`wrap_with_ellipsis`] is pure (a `measure` closure stands in for the font)
//! so its edge cases are unit-tested without an egui context.

use crate::ui::style;
use crate::ui::text::ELLIPSIS;
use crate::ui::theme::{self, Colors};
use egui::{
    Align2, Color32, Context, CornerRadius, FontFamily, FontId, Painter, Pos2, Rect, Stroke, Vec2,
};
use std::time::Duration;
use unicode_segmentation::UnicodeSegmentation;

/// Largest grapheme prefix of `word` that fits `max_w`, and the remainder.
fn split_fit<'a>(word: &'a str, max_w: f32, measure: &dyn Fn(&str) -> f32) -> (&'a str, &'a str) {
    let mut end = 0;
    for (idx, g) in word.grapheme_indices(true) {
        let next = idx + g.len();
        if measure(&word[..next]) > max_w {
            break;
        }
        end = next;
    }
    // Prefer a soft break (`-`, `/`, `_`, `.`) inside the fitting prefix over
    // splitting mid-segment, unless that would leave a sliver.
    if end < word.len() {
        let soft = word[..end]
            .char_indices()
            .filter(|(_, c)| matches!(c, '-' | '/' | '_' | '.'))
            .map(|(i, c)| i + c.len_utf8())
            .last();
        if let Some(cut) = soft {
            if cut >= 3 {
                return word.split_at(cut);
            }
        }
    }
    word.split_at(end)
}

/// `line` with a trailing [`ELLIPSIS`], shortened by grapheme until it fits.
fn ellipsize_line(line: &str, max_w: f32, measure: &dyn Fn(&str) -> f32) -> Option<String> {
    let mut graphemes: Vec<&str> = line.graphemes(true).collect();
    loop {
        let candidate = format!("{}{ELLIPSIS}", graphemes.concat().trim_end());
        if measure(&candidate) <= max_w {
            return Some(candidate);
        }
        graphemes.pop()?;
    }
}

/// Wrap `text` to `max_w` on word boundaries, hard-breaking words wider than
/// the box by grapheme. `max_lines` caps the result; when text is cut, only the
/// last returned line carries the ellipsis. `\n` starts a new line. No returned
/// line is wider than `max_w`; when not even one glyph fits the result is empty.
pub(crate) fn wrap_with_ellipsis(
    text: &str,
    max_w: f32,
    max_lines: Option<usize>,
    measure: &dyn Fn(&str) -> f32,
) -> Vec<String> {
    if max_lines == Some(0) || max_w <= 0.0 {
        return Vec::new();
    }
    // One more than the cap is enough to know the text overflowed.
    let budget = max_lines.map(|n| n + 1).unwrap_or(usize::MAX);
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();

    'paragraphs: for paragraph in text.split('\n') {
        for word in paragraph.split_whitespace() {
            let mut rest = word;
            loop {
                let candidate = if cur.is_empty() {
                    rest.to_string()
                } else {
                    format!("{cur} {rest}")
                };
                if measure(&candidate) <= max_w {
                    cur = candidate;
                    break;
                }
                if !cur.is_empty() {
                    lines.push(std::mem::take(&mut cur));
                } else {
                    let (head, tail) = split_fit(rest, max_w, measure);
                    if head.is_empty() {
                        return Vec::new(); // narrower than one glyph
                    }
                    lines.push(head.to_string());
                    rest = tail;
                }
                if lines.len() >= budget {
                    break 'paragraphs;
                }
            }
        }
        if !cur.is_empty() {
            lines.push(std::mem::take(&mut cur));
            if lines.len() >= budget {
                break;
            }
        }
    }
    if !cur.is_empty() && lines.len() < budget {
        lines.push(cur);
    }

    if let Some(cap) = max_lines {
        if lines.len() > cap {
            lines.truncate(cap);
            if let Some(last) = lines.pop() {
                if let Some(line) = ellipsize_line(&last, max_w, measure) {
                    lines.push(line);
                }
            }
        }
    }
    lines
}

/// Measure text in `font` with the renderer's own fonts.
pub(crate) fn measure_fn<'a>(ctx: &'a Context, font: FontId) -> impl Fn(&str) -> f32 + 'a {
    move |s: &str| {
        ctx.fonts_mut(|f| {
            f.layout_no_wrap(s.to_string(), font.clone(), Color32::PLACEHOLDER)
                .size()
                .x
        })
    }
}

/// Wrap `text` against the renderer's fonts.
pub(crate) fn wrap_text(
    ctx: &Context,
    text: &str,
    font: FontId,
    max_w: f32,
    max_lines: Option<usize>,
) -> Vec<String> {
    wrap_with_ellipsis(text, max_w, max_lines, &measure_fn(ctx, font))
}

/// What the thing the card stands for is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CardState {
    /// A command is queued and will run on confirmation.
    CommandWaiting,
    /// An agent is actively working; `detail` carries its live activity.
    Working,
    /// Blocked on a human decision.
    NeedsYou,
    Done,
    Failed,
    /// A quiet shell or an acknowledged notice.
    Idle,
}

impl CardState {
    pub(crate) fn label(self) -> &'static str {
        match self {
            CardState::CommandWaiting => "waiting to run",
            CardState::Working => "working",
            CardState::NeedsYou => "needs you",
            CardState::Done => "done",
            CardState::Failed => "failed",
            CardState::Idle => "idle",
        }
    }

    fn color(self, colors: &Colors) -> Color32 {
        match self {
            CardState::CommandWaiting => colors.warning,
            CardState::Working => colors.accent,
            CardState::NeedsYou => colors.zoom_needs,
            CardState::Done => colors.success,
            CardState::Failed => colors.zoom_failed,
            CardState::Idle => colors.text_dim,
        }
    }

    fn fill(self, colors: &Colors, base: Color32) -> Color32 {
        match self {
            CardState::NeedsYou => colors.needs_fill,
            CardState::Working => base.lerp_to_gamma(colors.accent, 0.24),
            _ => base,
        }
    }

    fn animated(self) -> bool {
        matches!(
            self,
            CardState::Working | CardState::NeedsYou | CardState::CommandWaiting
        )
    }
}

/// The surface the card is drawn on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CardForm {
    Sibling,
    Atlas,
    Bar,
}

struct Metrics {
    pad_x: f32,
    pad_y: f32,
    title_font: FontId,
    detail_font: FontId,
    title_lines: usize,
    detail_lines: usize,
    min_h: f32,
}

impl CardForm {
    fn metrics(self) -> Metrics {
        match self {
            CardForm::Sibling => Metrics {
                pad_x: 14.0,
                pad_y: 12.0,
                title_font: theme::font_medium(style::TEXT_BODY),
                detail_font: FontId::new(style::TEXT_CAPTION, FontFamily::Proportional),
                title_lines: 2,
                detail_lines: 3,
                min_h: 0.0,
            },
            CardForm::Atlas => Metrics {
                pad_x: 12.0,
                pad_y: 10.0,
                title_font: theme::font_medium(style::TEXT_CAPTION),
                detail_font: FontId::new(style::TEXT_HINT, FontFamily::Proportional),
                title_lines: 2,
                detail_lines: 2,
                min_h: style::ATLAS_NODE_H,
            },
            CardForm::Bar => Metrics {
                pad_x: 10.0,
                pad_y: 8.0,
                title_font: theme::font_medium(style::TEXT_CAPTION),
                detail_font: FontId::new(style::TEXT_CAPTION, FontFamily::Proportional),
                title_lines: 1,
                detail_lines: 2,
                min_h: 0.0,
            },
        }
    }
}

/// Width of the state strip on the card's left edge.
const STRIP_W: f32 = 4.0;
/// Room reserved for the state icon between the strip and the title.
const ICON_W: f32 = 18.0;
const LINE_GAP: f32 = 2.0;
const SECTION_GAP: f32 = 4.0;

/// Content of a card block.
pub(crate) struct CardSpec<'a> {
    pub state: CardState,
    pub title: &'a str,
    /// Dim single line: context path, pane/child counts.
    pub subtitle: &'a str,
    /// Wrapped body: live activity, the ask, the failure.
    pub detail: &'a str,
    /// Right-aligned badge on the title row (`◆2`, `Enter ▸ Decide`, `✕`).
    pub badge: Option<(String, Color32)>,
}

/// Wrapped text and resolved geometry for a [`CardSpec`] at a width.
pub(crate) struct CardLayout {
    pub form: CardForm,
    pub height: f32,
    title: Vec<String>,
    subtitle: String,
    detail: Vec<String>,
    badge_w: f32,
}

/// Room for the play glyph drawn before a command-waiting "run" label.
const RUN_GLYPH_W: f32 = 10.0;

/// Badge text: the spec's own, or "run" for a command waiting to execute.
fn badge_text(spec: &CardSpec) -> Option<String> {
    match (&spec.badge, spec.state) {
        (Some((t, _)), _) => Some(t.clone()),
        (None, CardState::CommandWaiting) => Some("run".to_string()),
        _ => None,
    }
}

fn row_h(font: &FontId) -> f32 {
    font.size * 1.25
}

/// Wrap every text slot of `spec` for a card `width` wide and compute the
/// height that fits them (never below the form's minimum).
pub(crate) fn layout_card(
    ctx: &Context,
    form: CardForm,
    spec: &CardSpec,
    width: f32,
) -> CardLayout {
    let m = form.metrics();
    let badge_font = theme::font_medium(style::TEXT_CAPTION);
    let badge_w = badge_text(spec)
        .map(|t| {
            let glyph = if spec.badge.is_none() { RUN_GLYPH_W } else { 0.0 };
            measure_fn(ctx, badge_font)(&t) + 12.0 + glyph
        })
        .unwrap_or(0.0);
    let text_x = m.pad_x + STRIP_W + ICON_W;
    let inner_w = (width - text_x - m.pad_x).max(1.0);
    let title_w = (inner_w - badge_w).max(1.0);

    let title = wrap_text(
        ctx,
        spec.title,
        m.title_font.clone(),
        title_w,
        Some(m.title_lines),
    );
    let detail = wrap_text(
        ctx,
        spec.detail,
        m.detail_font.clone(),
        inner_w,
        Some(m.detail_lines),
    );

    let sub_font = FontId::new(style::TEXT_HINT, FontFamily::Proportional);
    let mut h = m.pad_y * 2.0 + row_h(&m.title_font) * title.len().max(1) as f32;
    // Bars keep the path on the title row; other forms give it its own line.
    if form != CardForm::Bar && !spec.subtitle.is_empty() {
        h += SECTION_GAP + row_h(&sub_font);
    }
    if !detail.is_empty() {
        h += SECTION_GAP + (row_h(&m.detail_font) + LINE_GAP) * detail.len() as f32 - LINE_GAP;
    }
    // A title-only card shrinks to its content; the form minimum is for cards
    // that carry a path or detail line.
    let title_only = spec.subtitle.is_empty() && detail.is_empty();
    CardLayout {
        form,
        height: if title_only { h } else { h.max(m.min_h) },
        title,
        subtitle: spec.subtitle.to_string(),
        detail,
        badge_w,
    }
}

/// Paint a card block into `rect` (whose width the layout was built for).
/// Returns the rect the title row occupies on the right for hit-testing a badge.
pub(crate) fn paint_card(
    painter: &Painter,
    rect: Rect,
    layout: &CardLayout,
    spec: &CardSpec,
    colors: &Colors,
    base_fill: Color32,
    selected: bool,
) -> Rect {
    let form = layout.form;
    let m = form.metrics();
    let state_color = spec.state.color(colors);
    let radius = match form {
        CardForm::Bar => style::RADIUS_BAR,
        _ => style::RADIUS_MOTORWAYS,
    };
    let stroke = if selected {
        Stroke::new(2.0_f32, colors.accent)
    } else if spec.state == CardState::NeedsYou {
        Stroke::new(1.5_f32, state_color)
    } else {
        Stroke::new(1.0_f32, colors.border)
    };
    if form != CardForm::Bar {
        painter.rect_filled(
            rect.translate(Vec2::new(2.0, 2.0)),
            radius,
            colors.shadow_flat,
        );
    }
    painter.rect_filled(rect, radius, spec.state.fill(colors, base_fill));
    painter.rect_stroke(rect, radius, stroke, egui::StrokeKind::Inside);

    // Everything below is clipped to the box: no glyph can escape it.
    let clip = rect.shrink(1.0);
    let p = painter.with_clip_rect(clip.intersect(painter.clip_rect()));

    // State strip (left corners follow the card's rounding).
    let r = radius.nw;
    p.rect_filled(
        Rect::from_min_size(rect.min, Vec2::new(STRIP_W, rect.height())),
        CornerRadius {
            nw: r,
            sw: r,
            ne: 0,
            se: 0,
        },
        if spec.state == CardState::Idle {
            Color32::TRANSPARENT
        } else {
            state_color
        },
    );

    let time = painter.ctx().input(|i| i.time);
    let title_h = row_h(&m.title_font);
    let icon_c = Pos2::new(
        rect.min.x + STRIP_W + m.pad_x * 0.5 + ICON_W * 0.5,
        rect.min.y + m.pad_y + title_h * 0.5,
    );
    paint_state_icon(&p, icon_c, spec.state, colors, time);
    if spec.state.animated() {
        painter.ctx().request_repaint_after(Duration::from_millis(50));
    }

    let text_x = rect.min.x + m.pad_x + STRIP_W + ICON_W;
    let mut y = rect.min.y + m.pad_y;
    for line in &layout.title {
        p.text(
            Pos2::new(text_x, y),
            Align2::LEFT_TOP,
            line,
            m.title_font.clone(),
            if spec.state == CardState::Idle {
                colors.text_dim.gamma_multiply(0.75)
            } else {
                colors.text_primary
            },
        );
        y += title_h;
    }
    y = y.max(rect.min.y + m.pad_y + title_h);

    let sub_font = FontId::new(style::TEXT_HINT, FontFamily::Proportional);
    let inner_w = (rect.width() - (text_x - rect.min.x) - m.pad_x).max(1.0);
    if !layout.subtitle.is_empty() {
        let measure = measure_fn(painter.ctx(), sub_font.clone());
        if form == CardForm::Bar {
            // Path sits on the title row after the (last) title line.
            let last = layout.title.last().map(String::as_str).unwrap_or("");
            let title_end =
                measure_fn(painter.ctx(), m.title_font.clone())(last) + 8.0;
            let avail = inner_w - layout.badge_w - title_end;
            if let Some(line) =
                wrap_with_ellipsis(&layout.subtitle, avail, Some(1), &measure).first()
            {
                p.text(
                    Pos2::new(text_x + title_end, y - title_h + (title_h - row_h(&sub_font)) * 0.5 + 1.0),
                    Align2::LEFT_TOP,
                    line,
                    sub_font.clone(),
                    colors.text_dim,
                );
            }
        } else {
            y += SECTION_GAP;
            if let Some(line) =
                wrap_with_ellipsis(&layout.subtitle, inner_w, Some(1), &measure).first()
            {
                p.text(
                    Pos2::new(text_x, y),
                    Align2::LEFT_TOP,
                    line,
                    sub_font.clone(),
                    colors.text_dim,
                );
            }
            y += row_h(&sub_font);
        }
    }

    if !layout.detail.is_empty() {
        y += SECTION_GAP;
        let color = if spec.state == CardState::Idle {
            colors.text_dim.gamma_multiply(0.7)
        } else {
            colors.text_dim
        };
        for line in &layout.detail {
            // Never draw a partial line: it must fit above the bottom padding.
            if y + row_h(&m.detail_font) > rect.max.y - m.pad_y * 0.5 {
                break;
            }
            p.text(
                Pos2::new(text_x, y),
                Align2::LEFT_TOP,
                line,
                m.detail_font.clone(),
                color,
            );
            y += row_h(&m.detail_font) + LINE_GAP;
        }
    }

    let badge_rect = Rect::from_min_size(
        Pos2::new(rect.max.x - m.pad_x - layout.badge_w, rect.min.y + m.pad_y),
        Vec2::new(layout.badge_w, title_h),
    );
    if let Some(text) = badge_text(spec) {
        let color = spec.badge.as_ref().map(|(_, c)| *c).unwrap_or(state_color);
        let cy = rect.min.y + m.pad_y + title_h * 0.5;
        let galley = p.text(
            Pos2::new(rect.max.x - m.pad_x, cy),
            Align2::RIGHT_CENTER,
            text,
            theme::font_medium(style::TEXT_CAPTION),
            color,
        );
        if spec.badge.is_none() {
            // Play triangle before "run".
            let x = galley.min.x - 4.0;
            p.add(egui::Shape::convex_polygon(
                vec![
                    Pos2::new(x - 6.0, cy - 4.0),
                    Pos2::new(x, cy),
                    Pos2::new(x - 6.0, cy + 4.0),
                ],
                color,
                Stroke::NONE,
            ));
        }
    }
    badge_rect
}

/// Icons are drawn as shapes, not glyphs, so they never depend on font coverage.
fn paint_state_icon(p: &Painter, c: Pos2, state: CardState, colors: &Colors, time: f64) {
    let col = state.color(colors);
    let pulse = ((time * std::f64::consts::TAU / 1.4).sin() as f32 * 0.5 + 0.5).clamp(0.0, 1.0);
    match state {
        CardState::Working => {
            p.circle_filled(c, 3.0 + 3.0 * pulse, col.gamma_multiply(0.35 * (1.0 - pulse)));
            p.circle_filled(c, 3.5, col);
        }
        CardState::NeedsYou => {
            p.circle_stroke(
                c,
                4.0 + 3.0 * pulse,
                Stroke::new(1.5_f32, col.gamma_multiply(1.0 - pulse * 0.8)),
            );
            let d = 3.5;
            p.add(egui::Shape::convex_polygon(
                vec![
                    c + Vec2::new(0.0, -d),
                    c + Vec2::new(d, 0.0),
                    c + Vec2::new(0.0, d),
                    c + Vec2::new(-d, 0.0),
                ],
                col,
                Stroke::NONE,
            ));
        }
        CardState::CommandWaiting => {
            // A blinking block cursor.
            let on = (time * 1.6).fract() < 0.6;
            let r = Rect::from_center_size(c, Vec2::new(6.0, 10.0));
            if on {
                p.rect_filled(r, 1.0, col);
            } else {
                p.rect_stroke(r, 1.0, Stroke::new(1.0_f32, col), egui::StrokeKind::Inside);
            }
        }
        CardState::Done => {
            let s = Stroke::new(2.0_f32, col);
            p.line_segment([c + Vec2::new(-4.0, 0.5), c + Vec2::new(-1.0, 3.5)], s);
            p.line_segment([c + Vec2::new(-1.0, 3.5), c + Vec2::new(4.5, -3.5)], s);
        }
        CardState::Failed => {
            let s = Stroke::new(2.0_f32, col);
            p.line_segment([c + Vec2::new(-3.5, -3.5), c + Vec2::new(3.5, 3.5)], s);
            p.line_segment([c + Vec2::new(-3.5, 3.5), c + Vec2::new(3.5, -3.5)], s);
        }
        CardState::Idle => {
            p.circle_stroke(c, 3.0, Stroke::new(1.5_f32, col));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every glyph is 10px wide; spaces too.
    fn m(s: &str) -> f32 {
        s.graphemes(true).count() as f32 * 10.0
    }

    #[test]
    fn fits_on_one_line() {
        assert_eq!(wrap_with_ellipsis("hello world", 200.0, Some(2), &m), ["hello world"]);
    }

    #[test]
    fn wraps_on_word_boundaries() {
        // 80px = 8 glyphs.
        let out = wrap_with_ellipsis("one two three four", 80.0, None, &m);
        assert_eq!(out, ["one two", "three", "four"]);
        assert!(out.iter().all(|l| m(l) <= 80.0));
    }

    #[test]
    fn ellipsis_only_on_last_line() {
        let out = wrap_with_ellipsis("one two three four five six", 80.0, Some(2), &m);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0], "one two");
        assert!(out[1].ends_with(ELLIPSIS));
        assert!(!out[0].contains(ELLIPSIS));
        assert!(out.iter().all(|l| m(l) <= 80.0));
    }

    #[test]
    fn exact_cap_without_overflow_has_no_ellipsis() {
        let out = wrap_with_ellipsis("one two three", 80.0, Some(2), &m);
        assert_eq!(out, ["one two", "three"]);
    }

    #[test]
    fn long_word_hard_breaks_by_grapheme() {
        let out = wrap_with_ellipsis("abcdefghijklmnopqrst", 80.0, None, &m);
        assert_eq!(out, ["abcdefgh", "ijklmnop", "qrst"]);
        // Combining sequences are never split.
        let out = wrap_with_ellipsis("e\u{301}e\u{301}e\u{301}e\u{301}", 20.0, None, &m);
        assert_eq!(out, ["e\u{301}e\u{301}", "e\u{301}e\u{301}"]);
    }

    #[test]
    fn long_word_with_cap_ends_in_ellipsis() {
        let out = wrap_with_ellipsis("abcdefghijklmnopqrst", 80.0, Some(2), &m);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0], "abcdefgh");
        assert!(out[1].ends_with(ELLIPSIS) && m(&out[1]) <= 80.0);
    }

    #[test]
    fn empty_text_and_zero_lines_yield_nothing() {
        assert!(wrap_with_ellipsis("", 100.0, Some(2), &m).is_empty());
        assert!(wrap_with_ellipsis("   \n  ", 100.0, Some(2), &m).is_empty());
        assert!(wrap_with_ellipsis("text", 100.0, Some(0), &m).is_empty());
    }

    #[test]
    fn width_smaller_than_one_glyph_yields_nothing() {
        assert!(wrap_with_ellipsis("text", 5.0, Some(2), &m).is_empty());
        assert!(wrap_with_ellipsis("text", 0.0, None, &m).is_empty());
    }

    #[test]
    fn one_glyph_wide_box_never_overflows() {
        let out = wrap_with_ellipsis("abc", 10.0, Some(2), &m);
        assert!(out.iter().all(|l| m(l) <= 10.0));
        assert_eq!(out.last().map(String::as_str), Some(ELLIPSIS.to_string().as_str()));
    }

    #[test]
    fn newlines_start_new_lines() {
        let out = wrap_with_ellipsis("ab\ncd", 200.0, None, &m);
        assert_eq!(out, ["ab", "cd"]);
    }
}

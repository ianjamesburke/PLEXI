//! Shared `SearchBar` widget and match-highlight layout helpers (`stint 0814`).
//!
//! Wraps [`TextField::singleline`] with [`SurfaceKey`] focus registration so
//! pane-embedded and overlay-embedded search inputs participate in the single
//! [`InputOwner`] focus authority and get full `egui::TextEdit` capabilities
//! (blinking caret, `Cmd+V` paste, `Alt+Backspace` word-delete, selection).

use crate::ui::bounded::{self, Presentation, Slot};
use crate::ui::focus::{claim_text_surface, register_default_text_surface, SurfaceKey};
use crate::ui::style;
use crate::ui::text_field::TextField;
use crate::ui::theme::Colors;
use egui::text::{LayoutJob, TextFormat};
use egui::{Align, Color32, FontId, Id, Layout, Pos2, Rect, Sense, Stroke, Ui};
use std::ops::Range;

/// Outcome of rendering a [`SearchBar`] for one frame.
pub struct SearchBarOutput {
    /// True if the query text was modified this frame (typing, paste, or clear button).
    pub changed: bool,
    /// True if the search input or magnifier icon was clicked.
    pub clicked: bool,
    /// True if the search input gained focus this frame.
    pub gained_focus: bool,
}

/// Shared search input bar with leading magnifier icon, [`SurfaceKey`] focus
/// registration, live match count / scanning status badge, and clear (`×`) button.
pub struct SearchBar<'a> {
    id: Id,
    hint: &'a str,
    surface: Option<SurfaceKey>,
    claim_focus: bool,
    status: Option<StatusKind>,
    scanning: bool,
    font_size: f32,
    pending_click: Option<crate::host::pane::PendingPaneClick>,
    shortcut: Option<&'a [&'a str]>,
}

impl<'a> SearchBar<'a> {
    pub fn new(id_source: impl std::hash::Hash, hint: &'a str) -> Self {
        Self::with_id(Id::new(id_source), hint)
    }

    /// Build with an input id the caller already owns, so the caller's focus
    /// bookkeeping and this widget share one id instead of two derivations.
    pub fn with_id(id: Id, hint: &'a str) -> Self {
        Self {
            id,
            hint,
            surface: None,
            claim_focus: false,
            status: None,
            scanning: false,
            font_size: style::TEXT_HINT,
            pending_click: None,
            shortcut: None,
        }
    }

    /// Bind this search bar to a [`SurfaceKey`] so `reconcile_egui_focus` keeps
    /// it focused as the surface's default text input while visible.
    pub fn surface(mut self, surface: SurfaceKey) -> Self {
        self.surface = Some(surface);
        self
    }

    /// Explicitly claim focus on this frame (e.g. when search mode first opens).
    pub fn claim_focus(mut self, claim: bool) -> Self {
        self.claim_focus = claim;
        self
    }

    /// Display the live match count and scanning indicator on the trailing edge.
    pub fn status(mut self, match_count: usize, scanning: bool) -> Self {
        self.status = Some(StatusKind::Count(match_count));
        self.scanning = scanning;
        self
    }

    /// Display the current match position as `current / total` (or
    /// `No results` when `total` is zero). `current` is 1-based.
    pub fn position(mut self, current: usize, total: usize) -> Self {
        self.status = Some(StatusKind::Position { current, total });
        self.scanning = false;
        self
    }

    /// Display shortcut hint chips on the trailing edge when query is empty.
    pub fn shortcut(mut self, shortcut: Option<&'a [&'a str]>) -> Self {
        self.shortcut = shortcut;
        self
    }

    /// Override the font size (defaults to [`style::TEXT_HINT`]).
    pub fn font_size(mut self, size: f32) -> Self {
        self.font_size = size;
        self
    }

    /// Forward a queued synthetic node click (`plexi pane click --node`).
    pub fn pending_click(
        mut self,
        pending_click: Option<crate::host::pane::PendingPaneClick>,
    ) -> Self {
        self.pending_click = pending_click;
        self
    }

    pub fn show(self, ui: &mut Ui, query: &mut String, colors: &Colors) -> SearchBarOutput {
        let mut cleared = false;
        let before = query.clone();

        let (status_label, status_compact) = status_labels(self.status, self.scanning);
        let has_clear = !query.is_empty();
        let clear_id = self.id.with("clear_btn");
        let gap = style::SPACE_SM;

        let font_id = FontId::proportional(self.font_size);
        let measure = |ui: &Ui, text: &str| {
            ui.fonts_mut(|f| {
                f.layout_no_wrap(text.to_owned(), font_id.clone(), colors.text_dim)
                    .size()
                    .x
            })
            .ceil()
        };
        let row_h = (self.font_size + 12.0).max(24.0);
        let (bounds, _) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), row_h), Sense::hover());

        // Trailing slots, right to left. Clear never shrinks; the status badge
        // degrades full -> compact -> hidden before the input yields.
        let mut slots: Vec<Slot> = Vec::new();
        if has_clear {
            slots.push(Slot {
                id: "clear",
                width: CLEAR_W,
                compact: None,
                required: true,
            });
        }
        if !status_label.is_empty() {
            let full = measure(ui, &status_label);
            slots.push(Slot {
                id: "status",
                width: full,
                compact: Some(measure(ui, &status_compact).min(full)),
                required: false,
            });
        } else if !has_clear {
            if let Some(keys) = self.shortcut {
                slots.push(Slot {
                    id: "shortcut",
                    width: crate::ui::shortcuts::key_combo_width(ui, keys),
                    compact: None,
                    required: false,
                });
            }
        }
        let icon_side = (self.font_size + 4.0).max(16.0);
        let text_minimum = icon_side + gap + INPUT_MIN_W;
        let row = bounded::allocate(bounds, text_minimum, gap, &slots).unwrap_or_else(|short| {
            log::info!(
                "search_bar: row narrower than required actions ({:.1} < {:.1}); trailing controls hidden",
                bounds.width(),
                short.minimum_width
            );
            bounded::allocate(bounds, 0.0, gap, &[]).expect("empty slot list always fits finite bounds")
        });

        let icon_visible = row.text.width() >= icon_side;
        let icon_rect = Rect::from_min_size(
            Pos2::new(row.text.left(), bounds.center().y - icon_side / 2.0),
            egui::vec2(icon_side, icon_side),
        );
        let mut icon_clicked = false;
        if icon_visible {
            let icon_resp = ui.interact(icon_rect, self.id.with("icon"), Sense::click());
            paint_magnifier_icon(ui.painter(), icon_rect, colors.accent);
            icon_clicked = icon_resp.clicked()
                || self
                    .pending_click
                    .is_some_and(|c| c.targets_id(icon_resp.id));
        }
        let input_left = if icon_visible {
            icon_rect.right() + gap
        } else {
            row.text.left()
        };
        let input_rect = Rect::from_min_max(
            Pos2::new(input_left.min(row.text.right()), row.text.top()),
            row.text.right_bottom(),
        );

        let hint_label = self.hint;
        let mut input_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(input_rect)
                .layout(Layout::left_to_right(Align::Center)),
        );
        let resp = TextField::singleline(self.id, self.hint)
            .log_name("search_bar")
            .show(&mut input_ui, query, colors);
        if query.is_empty() {
            resp.widget_info(move || {
                egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, hint_label)
            });
        }
        if let Some(surface_key) = self.surface {
            register_default_text_surface(ui.ctx(), surface_key, resp.id);
            if self.claim_focus || icon_clicked {
                claim_text_surface(ui.ctx(), surface_key, resp.id);
            }
        }

        for slot in row
            .slots
            .iter()
            .filter(|s| s.presentation != Presentation::Hidden)
        {
            match slot.id {
                "clear" => {
                    let btn = ui.put(
                        slot.rect,
                        crate::ui::button::KitButton::new("\u{00d7}", colors)
                            .kind(crate::ui::button::ButtonKind::Ghost)
                            .compact()
                            .text_size(self.font_size + 2.0)
                            .text_color(colors.text_dim),
                    );
                    let synthetic = self
                        .pending_click
                        .is_some_and(|c| c.targets_id(btn.id) || c.targets_id(clear_id));
                    if btn.clicked() || synthetic {
                        query.clear();
                        cleared = true;
                        if let Some(surface_key) = self.surface {
                            claim_text_surface(ui.ctx(), surface_key, self.id);
                        }
                    }
                }
                "status" => {
                    let text = if slot.presentation == Presentation::Compact {
                        &status_compact
                    } else {
                        &status_label
                    };
                    let color = if self.scanning {
                        colors.accent
                    } else {
                        colors.text_dim
                    };
                    let mut status_ui = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(slot.rect)
                            .layout(Layout::right_to_left(Align::Center)),
                    );
                    status_ui.add(
                        egui::Label::new(
                            egui::RichText::new(text).size(self.font_size).color(color),
                        )
                        .wrap_mode(egui::TextWrapMode::Extend),
                    );
                }
                "shortcut" => {
                    if let Some(keys) = self.shortcut {
                        crate::ui::shortcuts::key_combo_painted(
                            ui,
                            slot.rect.left(),
                            slot.rect.center().y,
                            keys,
                            colors,
                        );
                    }
                }
                other => unreachable!("search bar allocated unknown slot {other}"),
            }
        }

        let clicked = resp.clicked()
            || icon_clicked
            || self
                .pending_click
                .is_some_and(|c| c.targets_id(resp.id) || c.targets_id(self.id));
        let gained_focus = resp.gained_focus();
        let changed = resp.changed() || cleared || *query != before;
        SearchBarOutput {
            changed,
            clicked,
            gained_focus,
        }
    }
}

/// Width reserved for the clear (`×`) button.
const CLEAR_W: f32 = 26.0;

/// Narrowest the text input may get before the status badge yields.
const INPUT_MIN_W: f32 = 32.0;

#[derive(Clone, Copy)]
enum StatusKind {
    Count(usize),
    Position { current: usize, total: usize },
}

/// Full and compact status text. The compact form is what the badge degrades
/// to when the row cannot spare the full width.
fn status_labels(status: Option<StatusKind>, scanning: bool) -> (String, String) {
    let plural = |n: usize| if n == 1 { "" } else { "es" };
    match (status, scanning) {
        (Some(StatusKind::Position { current, total }), _) if total > 0 => {
            (format!("{current} / {total}"), format!("{current}/{total}"))
        }
        (Some(StatusKind::Position { .. }), _) => ("No results".into(), "0".into()),
        (Some(StatusKind::Count(n)), true) if n > 0 => (
            format!("Scanning\u{2026} \u{00b7} {n} match{}", plural(n)),
            n.to_string(),
        ),
        (Some(StatusKind::Count(_)) | None, true) => ("Scanning\u{2026}".into(), "\u{2026}".into()),
        (Some(StatusKind::Count(n)), false) => (format!("{n} match{}", plural(n)), n.to_string()),
        (None, false) => (String::new(), String::new()),
    }
}

fn paint_magnifier_icon(painter: &egui::Painter, rect: Rect, color: Color32) {
    let center = Pos2::new(
        rect.center().x - rect.width() * 0.08,
        rect.center().y - rect.height() * 0.08,
    );
    let radius = (rect.width().min(rect.height()) * 0.28).max(3.5);
    let stroke = Stroke::new(1.5_f32, color);
    painter.circle_stroke(center, radius, stroke);
    let dir = egui::vec2(0.707, 0.707);
    let handle_start = center + dir * radius;
    let handle_end = center + dir * (radius * 1.85);
    painter.line_segment([handle_start, handle_end], stroke);
}

/// Append `text` to `job`, highlighting any valid UTF-8 byte ranges in `ranges`
/// with `highlight_color`.
pub fn append_highlighted_segments(
    job: &mut LayoutJob,
    text: &str,
    ranges: &[Range<usize>],
    font_id: &FontId,
    base_color: Color32,
    highlight_color: Color32,
) {
    let base_fmt = TextFormat {
        font_id: font_id.clone(),
        color: base_color,
        ..Default::default()
    };
    let hi_fmt = TextFormat {
        font_id: font_id.clone(),
        color: highlight_color,
        ..Default::default()
    };

    let mut cursor = 0usize;
    for range in ranges {
        let start = range.start.clamp(cursor, text.len());
        let end = range.end.clamp(start, text.len());
        if !text.is_char_boundary(start) || !text.is_char_boundary(end) {
            continue;
        }
        if start > cursor {
            job.append(&text[cursor..start], 0.0, base_fmt.clone());
        }
        if end > start {
            job.append(&text[start..end], 0.0, hi_fmt.clone());
        }
        cursor = end;
    }
    if cursor < text.len() && text.is_char_boundary(cursor) {
        job.append(&text[cursor..], 0.0, base_fmt);
    }
}

/// Build a single-line [`LayoutJob`] for a File Explorer search result row:
/// - `parent_prefix` (e.g. `"src/ui/"`) in `colors.text_dim` (with any parent-path
///   match ranges highlighted in `colors.accent`),
/// - `file_name` (plus trailing `/` for directories) in `name_color` with
///   `name_ranges` highlighted in `colors.accent`,
/// - optional `:line — snippet` content preview when the hit matched file contents,
///   with `line_ranges` highlighted in `colors.accent`.
pub fn build_search_hit_job(
    hit: &crate::search::SearchHit,
    font_id: FontId,
    name_color: Color32,
    colors: &Colors,
) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;

    let parent = hit.parent_prefix();
    if !parent.is_empty() {
        let parent_len = parent.len();
        let parent_ranges: Vec<Range<usize>> = hit
            .rel_path_ranges
            .iter()
            .filter_map(|r| {
                if r.start < parent_len {
                    Some(r.start..r.end.min(parent_len))
                } else {
                    None
                }
            })
            .collect();
        append_highlighted_segments(
            &mut job,
            parent,
            &parent_ranges,
            &font_id,
            colors.text_dim,
            colors.accent,
        );
    }

    append_highlighted_segments(
        &mut job,
        &hit.file_name,
        &hit.name_ranges,
        &font_id,
        name_color,
        colors.accent,
    );

    if hit.is_dir {
        job.append(
            "/",
            0.0,
            TextFormat {
                font_id: font_id.clone(),
                color: name_color,
                ..Default::default()
            },
        );
    }

    if let Some(snippet) = hit.kind.content_snippet() {
        let meta_fmt = TextFormat {
            font_id: font_id.clone(),
            color: colors.text_dim,
            ..Default::default()
        };
        job.append(
            &format!("  :{} \u{2014} ", snippet.line_number),
            0.0,
            meta_fmt.clone(),
        );
        append_highlighted_segments(
            &mut job,
            &snippet.line_preview,
            &snippet.line_ranges,
            &font_id,
            colors.text_dim,
            colors.accent,
        );
        if snippet.additional_matches > 0 {
            job.append(
                &format!(" (+{})", snippet.additional_matches),
                0.0,
                meta_fmt,
            );
        }
    }

    job
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The clear glyph scales with the bar's font size (+2) and stays dim.
    #[test]
    fn clear_glyph_scales_with_font_size_and_stays_dim() {
        let host = crate::testing::HostHarness::new();
        let colors = host.app.colors;
        let glyph = |font_size: f32| {
            let mut output = None;
            for frame in 0..2 {
                let mut query = String::from("abc");
                output = Some(host.app.ctx.run_ui(
                    egui::RawInput {
                        time: Some(frame as f64),
                        ..Default::default()
                    },
                    |ui| {
                        egui::CentralPanel::default().show_inside(ui, |ui| {
                            SearchBar::new("clear_glyph_test", "Search")
                                .font_size(font_size)
                                .show(ui, &mut query, &colors);
                        });
                    },
                ));
            }
            output
                .unwrap()
                .shapes
                .into_iter()
                .find_map(|s| match s.shape {
                    egui::Shape::Text(t) if t.galley.job.text == "\u{00d7}" => {
                        let section = &t.galley.job.sections[0];
                        Some((
                            section.format.font_id.size,
                            t.override_text_color.unwrap_or(section.format.color),
                        ))
                    }
                    _ => None,
                })
                .expect("clear glyph")
        };
        let (small, color) = glyph(12.0);
        let (large, _) = glyph(20.0);
        assert_eq!(small, 14.0);
        assert_eq!(large, 22.0);
        assert_ne!(
            color, colors.text_primary,
            "glyph stays dim, not primary text"
        );
    }

    /// The query text follows the bar's font size, so a pane that scales its
    /// content with Cmd+/- scales what the user typed with it (stint 1110 PR 1).
    #[test]
    fn search_bar_query_text_follows_pane_font_size() {
        let host = crate::testing::HostHarness::new();
        let colors = host.app.colors;
        let query_font_size = |font_size: f32| {
            let mut output = None;
            for frame in 0..2 {
                let mut query = String::from("needle");
                output = Some(host.app.ctx.run_ui(
                    egui::RawInput {
                        time: Some(frame as f64),
                        ..Default::default()
                    },
                    |ui| {
                        egui::CentralPanel::default().show_inside(ui, |ui| {
                            SearchBar::new("query_font_test", "Search")
                                .font_size(font_size)
                                .show(ui, &mut query, &colors);
                        });
                    },
                ));
            }
            output
                .unwrap()
                .shapes
                .into_iter()
                .find_map(|s| match s.shape {
                    egui::Shape::Text(t) if t.galley.job.text == "needle" => {
                        Some(t.galley.job.sections[0].format.font_id.size)
                    }
                    _ => None,
                })
                .expect("query text")
        };
        assert_eq!(query_font_size(12.0), 12.0);
        assert_eq!(query_font_size(20.0), 20.0);
    }
}

#[cfg(test)]
mod status_tests {
    use super::*;

    #[test]
    fn count_status_keeps_the_explorer_wording() {
        assert_eq!(
            status_labels(Some(StatusKind::Count(1)), false).0,
            "1 match"
        );
        assert_eq!(
            status_labels(Some(StatusKind::Count(3)), false).0,
            "3 matches"
        );
        assert_eq!(
            status_labels(Some(StatusKind::Count(2)), true).0,
            "Scanning\u{2026} \u{00b7} 2 matches"
        );
        assert_eq!(status_labels(None, true).0, "Scanning\u{2026}");
        assert_eq!(status_labels(None, false), (String::new(), String::new()));
    }

    #[test]
    fn position_status_has_a_full_and_a_compact_form() {
        let position =
            |current, total| status_labels(Some(StatusKind::Position { current, total }), false);
        assert_eq!(position(2, 5), ("2 / 5".to_string(), "2/5".to_string()));
        assert_eq!(position(0, 0), ("No results".to_string(), "0".to_string()));
    }
}

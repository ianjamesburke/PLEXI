//! Shared `SearchBar` widget and match-highlight layout helpers (`stint 0814`).
//!
//! Wraps [`TextField::singleline`] with [`SurfaceKey`] focus registration so
//! pane-embedded and overlay-embedded search inputs participate in the single
//! [`InputOwner`] focus authority and get full `egui::TextEdit` capabilities
//! (blinking caret, `Cmd+V` paste, `Alt+Backspace` word-delete, selection).

use crate::ui::focus::{claim_text_surface, register_default_text_surface, SurfaceKey};
use crate::ui::style;
use crate::ui::text_field::TextField;
use crate::ui::theme::Colors;
use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontId, Id, Pos2, Rect, Sense, Stroke, Ui};
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
    match_count: Option<usize>,
    scanning: bool,
    font_size: f32,
    pending_click: Option<crate::host::pane::PendingPaneClick>,
    shortcut: Option<&'a [&'a str]>,
}

impl<'a> SearchBar<'a> {
    pub fn new(id_source: impl std::hash::Hash, hint: &'a str) -> Self {
        Self {
            id: Id::new(id_source),
            hint,
            surface: None,
            claim_focus: false,
            match_count: None,
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
        self.match_count = Some(match_count);
        self.scanning = scanning;
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

        let status_label = match (self.match_count, self.scanning) {
            (Some(count), true) if count > 0 => {
                format!(
                    "Scanning\u{2026} \u{00b7} {count} match{}",
                    if count == 1 { "" } else { "es" }
                )
            }
            (Some(_), true) => "Scanning\u{2026}".to_string(),
            (Some(count), false) => {
                format!("{count} match{}", if count == 1 { "" } else { "es" })
            }
            (None, true) => "Scanning\u{2026}".to_string(),
            (None, false) => String::new(),
        };

        let clear_id = self.id.with("clear_btn");
        let has_clear = !query.is_empty();

        let mut text_response: Option<egui::Response> = None;
        let mut icon_clicked = false;

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = style::SPACE_SM;

            // Leading magnifier icon
            let icon_side = (self.font_size + 4.0).max(16.0);
            let (icon_rect, icon_resp) =
                ui.allocate_exact_size(egui::vec2(icon_side, icon_side), Sense::click());
            paint_magnifier_icon(ui.painter(), icon_rect, colors.accent);
            if icon_resp.clicked()
                || self
                    .pending_click
                    .is_some_and(|c| c.targets_id(icon_resp.id))
            {
                icon_clicked = true;
            }

            // Compute trailing width reservation so the TextField doesn't push
            // the status badge, clear button, or shortcut chip off the right edge.
            let font_id = FontId::proportional(self.font_size);
            let status_w = if status_label.is_empty() {
                0.0
            } else {
                ui.fonts_mut(|f| {
                    f.layout_no_wrap(status_label.clone(), font_id.clone(), colors.text_dim)
                        .size()
                        .x
                }) + style::SPACE_MD
            };
            let clear_w = if has_clear { 26.0 } else { 0.0 };
            let shortcut_w = if !has_clear && status_label.is_empty() {
                self.shortcut
                    .map(|keys| crate::ui::shortcuts::key_combo_width(ui, keys))
                    .unwrap_or(0.0)
            } else {
                0.0
            };
            let input_w = (ui.available_width() - status_w - clear_w - shortcut_w).max(80.0);

            let hint_label = self.hint;
            let resp = ui
                .allocate_ui_with_layout(
                    egui::vec2(input_w, (self.font_size + 12.0).max(24.0)),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        TextField::singleline(self.id, self.hint)
                            .log_name("search_bar")
                            .show(ui, query, colors)
                    },
                )
                .inner;

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
            text_response = Some(resp);

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if has_clear {
                    let btn = ui.add(
                        egui::Button::new(
                            egui::RichText::new("\u{00d7}")
                                .size(self.font_size + 2.0)
                                .color(colors.text_dim),
                        )
                        .frame(false),
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
                } else if status_label.is_empty() {
                    if let Some(keys) = self.shortcut {
                        ui.with_layout(
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                crate::ui::shortcuts::key_combo(ui, keys, colors);
                            },
                        );
                    }
                }

                if !status_label.is_empty() {
                    let badge_color = if self.scanning {
                        colors.accent
                    } else {
                        colors.text_dim
                    };
                    ui.colored_label(
                        badge_color,
                        egui::RichText::new(&status_label).size(self.font_size),
                    );
                }
            });
        });

        let response = text_response.expect("SearchBar always renders TextField");
        let clicked = response.clicked()
            || icon_clicked
            || self
                .pending_click
                .is_some_and(|c| c.targets_id(response.id) || c.targets_id(self.id));
        let gained_focus = response.gained_focus();
        let changed = response.changed() || cleared || *query != before;
        SearchBarOutput {
            changed,
            clicked,
            gained_focus,
        }
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

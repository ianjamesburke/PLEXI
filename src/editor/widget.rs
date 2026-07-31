//! Minimal egui surface for the editor core — the only egui-dependent file.
//!
//! Adapted from Ferrite <https://github.com/OlaProeis/Ferrite>
//! @ 3ba085c561670342d72c560efbf6b0b92b5c0b46 (editor.rs event translation,
//! rendering/{cursor,text}.rs), MIT. Diverges from upstream: translates egui
//! input into [`EditorCommand`]s and paints from [`ViewState`] layout; owns no
//! editing logic. Focus arbitration is the caller's job (`src/ui/AGENTS.md`
//! forbids direct `request_focus`).

use egui::emath::GuiRounding;
use egui::text::{CCursor, LayoutJob, TextFormat};
use egui::{Color32, Event, ImeEvent, Key, Modifiers, Rect, Sense, Ui, Vec2};

use super::commands::{Document, EditorCommand, Movement};
use super::cursor::Cursor;
use super::highlight::{SpanProvider, TokenKind};
use super::layout::{DisplayLayout, LineLayout};
use super::mode::EditorMode;
use super::movement::line_text;
use super::preview::{is_bare_http_url, LinkTarget, MarkdownLayoutCache, MdStyle};
use super::view::ViewState;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use unicode_segmentation::UnicodeSegmentation;

/// Horizontal padding on each side of the gutter's line numbers.
const GUTTER_PAD: f32 = 6.0;

/// Caret on/off half-period, in seconds. The caret is solid for one interval,
/// hidden for the next, and so on — a ~500 ms phase matching the platform
/// text-cursor cadence.
const CARET_BLINK_INTERVAL: f64 = 0.5;

/// Whether the blinking caret is in its visible half-cycle, given the time
/// elapsed since the last caret activity (keystroke, edit, or caret move).
/// Activity resets the phase to zero, so the caret is always solid for the
/// first interval after any input and only then begins toggling. Pure so the
/// phase logic is unit-testable without pixels.
fn caret_blink_visible(elapsed_since_activity: f64, interval: f64) -> bool {
    if elapsed_since_activity < 0.0 {
        return true;
    }
    ((elapsed_since_activity / interval) as u64).is_multiple_of(2)
}

/// Wall-clock delay until the caret's next visibility flip, so the widget can
/// schedule exactly one repaint at the boundary instead of spinning. Always
/// positive (never a zero-delay repaint loop; see the egui repaint trap in the
/// root AGENTS.md).
fn caret_blink_next_toggle(elapsed_since_activity: f64, interval: f64) -> f64 {
    let elapsed = elapsed_since_activity.max(0.0);
    interval - (elapsed % interval)
}

fn galley_range_rects(galley: &egui::Galley, range: std::ops::Range<usize>) -> Vec<Rect> {
    if range.is_empty() {
        return Vec::new();
    }
    let mut row_start = 0;
    let mut rects = Vec::new();
    for row in &galley.rows {
        let row_end = row_start + row.char_count_excluding_newline();
        let from = range.start.max(row_start);
        let to = range.end.min(row_end);
        if from < to {
            let x0 = row.pos.x + row.x_offset(from - row_start);
            let x1 = row.pos.x + row.x_offset(to - row_start);
            rects.push(Rect::from_min_max(
                egui::pos2(x0, row.rect().top()),
                egui::pos2(x1, row.rect().bottom()),
            ));
        }
        row_start = row_end;
    }
    rects
}

#[derive(Clone)]
struct DisplayGalley {
    galley: std::sync::Arc<egui::Galley>,
    display_to_source: Vec<usize>,
}

#[derive(Clone)]
struct GeometryCache {
    revision: u64,
    width_bits: u32,
    pixels_per_point_bits: u32,
    soft_wrap: bool,
    font_id: egui::FontId,
    galleys: Vec<DisplayGalley>,
}

fn prepared_display_text(
    ui: &Ui,
    font_id: &egui::FontId,
    source: &str,
    wrap_width: f32,
) -> (String, Vec<usize>) {
    if !wrap_width.is_finite() {
        return (source.to_string(), (0..=source.chars().count()).collect());
    }
    let mut display = String::new();
    let mut display_to_source = vec![0];
    let mut source_column = 0;
    for segment in source.split_inclusive(char::is_whitespace) {
        let segment_width = ui.fonts_mut(|fonts| {
            fonts
                .layout_no_wrap(segment.to_string(), font_id.clone(), egui::Color32::WHITE)
                .size()
                .x
        });
        let add_grapheme_breaks = segment_width > wrap_width;
        for (index, grapheme) in segment.graphemes(true).enumerate() {
            if add_grapheme_breaks && index > 0 {
                display.push(' ');
                display_to_source.push(source_column);
            }
            for ch in grapheme.chars() {
                display.push(ch);
                source_column += 1;
                display_to_source.push(source_column);
            }
        }
    }
    (display, display_to_source)
}

fn prepare_layout_job(
    mut source_job: LayoutJob,
    display_text: &str,
    display_to_source: &[usize],
) -> LayoutJob {
    if source_job.text == display_text {
        return source_job;
    }
    let source_text = std::mem::take(&mut source_job.text);
    let source_char_bytes: Vec<usize> = source_text
        .char_indices()
        .map(|(byte, _)| byte)
        .chain(std::iter::once(source_text.len()))
        .collect();
    let mut job = LayoutJob {
        wrap: source_job.wrap,
        ..Default::default()
    };
    for (display_column, ch) in display_text.chars().enumerate() {
        let source_column =
            display_to_source[display_column].min(source_char_bytes.len().saturating_sub(1));
        let source_byte = source_char_bytes[source_column];
        let mut format = source_job
            .sections
            .iter()
            .find(|section| {
                section.byte_range.contains(&source_byte)
                    || (source_byte == source_text.len()
                        && section.byte_range.end == source_text.len())
            })
            .map_or_else(TextFormat::default, |section| section.format.clone());
        if display_to_source[display_column] == display_to_source[display_column + 1] {
            format.font_id.size = 0.01;
            format.extra_letter_spacing = 0.0;
        }
        job.append(&ch.to_string(), 0.0, format);
    }
    job
}

/// Theme colors for code-mode chrome and syntax token kinds. Callers build
/// this from host design tokens (`src/ui/theme.rs`); the editor core never
/// picks concrete colors itself.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CodeTheme {
    pub gutter_text: Color32,
    pub current_line_bg: Color32,
    pub keyword: Color32,
    pub string: Color32,
    pub comment: Color32,
    pub number: Color32,
    pub ty: Color32,
    pub function: Color32,
    pub punctuation: Color32,
}

impl CodeTheme {
    /// Fallback derived from egui visuals, for callers without host tokens
    /// (tests): plain chrome, no token coloring beyond text/weak.
    fn from_visuals(visuals: &egui::Visuals) -> Self {
        let text = visuals.text_color();
        let weak = visuals.weak_text_color();
        Self {
            gutter_text: weak,
            current_line_bg: visuals.faint_bg_color,
            keyword: text,
            string: text,
            comment: weak,
            number: text,
            ty: text,
            function: text,
            punctuation: weak,
        }
    }

    fn color_for(&self, kind: TokenKind, plain: Color32) -> Color32 {
        match kind {
            TokenKind::Plain => plain,
            TokenKind::Keyword => self.keyword,
            TokenKind::String => self.string,
            TokenKind::Comment => self.comment,
            TokenKind::Number => self.number,
            TokenKind::Type => self.ty,
            TokenKind::Function => self.function,
            TokenKind::Punctuation => self.punctuation,
        }
    }
}

/// Theme colors for Markdown Live Preview styling. Callers build this from
/// host design tokens; the editor core never picks concrete colors. Styling
/// is color/italic only — it must never change glyph metrics (see
/// `super::preview` for the source↔layout mapping contract).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MarkdownTheme {
    /// Structural syntax markers (`#`, `-`, `>`, `**`, fences): dimmed.
    pub marker: Color32,
    pub heading: Color32,
    pub strong: Color32,
    pub emphasis: Color32,
    pub code: Color32,
    pub quote: Color32,
    pub rule: Color32,
    /// Link source spans (Markdown, autolink, wiki): colored + underlined.
    pub link: Color32,
}

impl MarkdownTheme {
    /// Fallback derived from egui visuals, for callers without host tokens.
    fn from_visuals(visuals: &egui::Visuals) -> Self {
        let text = visuals.text_color();
        let weak = visuals.weak_text_color();
        Self {
            marker: weak,
            heading: visuals.strong_text_color(),
            strong: visuals.strong_text_color(),
            emphasis: text,
            code: text,
            quote: weak,
            rule: weak,
            link: visuals.hyperlink_color,
        }
    }

    /// Color, italic, and underline flags for one style class.
    fn format_for(&self, style: MdStyle, plain: Color32) -> (Color32, bool, bool) {
        match style {
            MdStyle::Marker => (self.marker, false, false),
            MdStyle::Heading(_) => (self.heading, false, false),
            MdStyle::Strong => (self.strong, false, false),
            MdStyle::Emphasis => (self.emphasis, true, false),
            MdStyle::Code => (self.code, false, false),
            MdStyle::Quote => (self.quote, true, false),
            MdStyle::Rule => (self.rule, false, false),
            MdStyle::Link => (self.link, false, true),
            MdStyle::Plain => (plain, false, false),
        }
    }
}

/// Maximum on-screen height of one inline image strip.
const IMAGE_MAX_HEIGHT: f32 = 320.0;
/// Height of the placeholder strip for loading/missing/remote images.
const IMAGE_PLACEHOLDER_HEIGHT: f32 = 40.0;
/// Vertical padding around an inline image strip.
const IMAGE_PAD: f32 = 4.0;

/// Render state of one inline image destination.
pub enum ImageState {
    /// Decoded and uploaded; `size` is the source pixel size.
    Ready {
        texture: egui::TextureHandle,
        size: [usize; 2],
    },
    /// Read or decode failed; the message paints in the placeholder.
    Failed(String),
    /// Remote (http/https) destination: never downloaded implicitly.
    Remote,
}

/// Texture cache for inline Live Preview images, keyed by raw destination.
/// Local files reload when their mtime changes; failures cache until the
/// file changes so a broken image never re-decodes every frame.
#[derive(Default)]
pub struct ImageCache {
    entries: HashMap<String, (Option<SystemTime>, ImageState)>,
}

impl ImageCache {
    /// The render state for `dest`, loading/decoding on first sight (and
    /// again when the file's mtime changes). `base` resolves relative paths.
    fn get(&mut self, ctx: &egui::Context, base: &Path, dest: &str) -> &ImageState {
        let (path, mtime) = if dest.starts_with("http://") || dest.starts_with("https://") {
            (None, None)
        } else {
            let path = if Path::new(dest).is_absolute() {
                PathBuf::from(dest)
            } else {
                base.join(dest)
            };
            let mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
            (Some(path), mtime)
        };
        // Key by the *resolved* path (not the raw destination): two documents
        // can both reference `assets/foo.png` meaning different files, and a
        // retargeted pane must never paint the previous document's texture.
        let key = match &path {
            Some(path) => path.to_string_lossy().into_owned(),
            None => dest.to_string(),
        };
        let stale = match self.entries.get(&key) {
            Some((cached_mtime, _)) => *cached_mtime != mtime,
            None => true,
        };
        if stale {
            let state = match &path {
                None => ImageState::Remote,
                Some(path) => match Self::load(ctx, path, dest) {
                    Ok(state) => state,
                    Err(reason) => {
                        log::info!(
                            "notes_editor: inline image render failed for {dest:?}: {reason}"
                        );
                        ImageState::Failed(reason)
                    }
                },
            };
            self.entries.insert(key.clone(), (mtime, state));
        }
        &self.entries[&key].1
    }

    fn load(ctx: &egui::Context, path: &Path, dest: &str) -> Result<ImageState, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
        let decoded = image::load_from_memory(&bytes)
            .map_err(|e| format!("decode {}: {e}", path.display()))?
            .to_rgba8();
        let size = [decoded.width() as usize, decoded.height() as usize];
        let color = egui::ColorImage::from_rgba_unmultiplied(size, decoded.as_raw());
        let texture = ctx.load_texture(
            format!("editor-inline-image:{dest}"),
            color,
            egui::TextureOptions::LINEAR,
        );
        Ok(ImageState::Ready { texture, size })
    }
}

/// Result of one [`EditorWidget::show`] pass.
pub struct EditorOutput {
    pub response: egui::Response,
    /// A link the user explicitly activated this frame (modifier-click).
    /// The widget only detects the gesture — acting on it is the caller's
    /// job; ordinary clicks placed the caret as usual.
    pub link_activation: Option<LinkTarget>,
}

/// Renders a [`Document`] and translates egui input into [`EditorCommand`]s.
///
/// The widget processes keyboard/IME input every frame it is shown; callers
/// decide whether to show it (host focus model, stint 0474).
pub struct EditorWidget<'a> {
    doc: &'a mut Document,
    view: &'a mut ViewState,
    /// Stable widget id, so callers can register it with the host focus
    /// reconciler (`crate::ui::focus`). Auto-generated when unset.
    id: Option<egui::Id>,
    /// When false, keyboard/IME/clipboard events are ignored (the caller's
    /// focus model says another surface owns input); pointer placement still
    /// works so clicking the editor can re-claim focus.
    active: bool,
    /// Monospace font size override; defaults to the style's monospace size.
    font_size: Option<f32>,
    /// Presentation/input mode: plain, Markdown structure commands, or code
    /// chrome (gutter, current-line highlight, syntax spans).
    mode: EditorMode,
    /// Syntax-highlight span source for code mode. `None` paints plain text
    /// (the fallback for unknown languages).
    span_provider: Option<&'a mut dyn SpanProvider>,
    /// Code-mode chrome/token colors; derived from egui visuals when unset.
    code_theme: Option<CodeTheme>,
    /// Cached Markdown block/inline layout for Live Preview. `None` renders
    /// Markdown mode as plain source.
    md_cache: Option<&'a mut MarkdownLayoutCache>,
    /// Live Preview colors; derived from egui visuals when unset.
    md_theme: Option<MarkdownTheme>,
    /// Inline-image texture cache plus the base dir resolving relative
    /// destinations. `None` disables inline image strips.
    images: Option<(&'a mut ImageCache, PathBuf)>,
    /// Char ranges to paint with a background (find matches), plus the index
    /// of the "current" range and the two fill colors (normal, current).
    highlights: Vec<(usize, usize)>,
    current_highlight: Option<usize>,
    highlight_bg: egui::Color32,
    current_highlight_bg: egui::Color32,
}

impl<'a> EditorWidget<'a> {
    pub fn new(doc: &'a mut Document, view: &'a mut ViewState) -> Self {
        Self {
            doc,
            view,
            id: None,
            active: true,
            font_size: None,
            mode: EditorMode::PlainText,
            span_provider: None,
            code_theme: None,
            md_cache: None,
            md_theme: None,
            images: None,
            highlights: Vec::new(),
            current_highlight: None,
            highlight_bg: egui::Color32::TRANSPARENT,
            current_highlight_bg: egui::Color32::TRANSPARENT,
        }
    }

    #[must_use]
    pub fn id(mut self, id: egui::Id) -> Self {
        self.id = Some(id);
        self
    }

    #[must_use]
    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    #[must_use]
    pub fn font_size(mut self, size: f32) -> Self {
        self.font_size = Some(size);
        self
    }

    #[must_use]
    pub fn mode(mut self, mode: EditorMode) -> Self {
        self.mode = mode;
        self
    }

    /// Syntax-highlight span source for code mode. Ignored outside
    /// [`EditorMode::Code`].
    #[must_use]
    pub fn span_provider(mut self, provider: &'a mut dyn SpanProvider) -> Self {
        self.span_provider = Some(provider);
        self
    }

    /// Code-mode chrome/token colors (host theme tokens).
    #[must_use]
    pub fn code_theme(mut self, theme: CodeTheme) -> Self {
        self.code_theme = Some(theme);
        self
    }

    /// Markdown block/inline layout cache for Live Preview. Ignored unless
    /// the mode is [`EditorMode::Markdown`] with `live_preview: true`.
    #[must_use]
    pub fn markdown_preview(mut self, cache: &'a mut MarkdownLayoutCache) -> Self {
        self.md_cache = Some(cache);
        self
    }

    /// Live Preview colors (host theme tokens).
    #[must_use]
    pub fn markdown_theme(mut self, theme: MarkdownTheme) -> Self {
        self.md_theme = Some(theme);
        self
    }

    /// Enables inline image strips in Live Preview: `cache` holds textures
    /// across frames, `base` resolves relative image destinations (the
    /// document's parent directory). Ignored outside Live Preview.
    #[must_use]
    pub fn images(mut self, cache: &'a mut ImageCache, base: PathBuf) -> Self {
        self.images = Some((cache, base));
        self
    }

    #[must_use]
    pub fn highlights(
        mut self,
        ranges: Vec<(usize, usize)>,
        current: Option<usize>,
        bg: egui::Color32,
        current_bg: egui::Color32,
    ) -> Self {
        self.highlights = ranges;
        self.current_highlight = current;
        self.highlight_bg = bg;
        self.current_highlight_bg = current_bg;
        self
    }

    pub fn show(mut self, ui: &mut Ui) -> EditorOutput {
        let font_id = match self.font_size {
            Some(size) => egui::FontId::monospace(size),
            None => egui::TextStyle::Monospace.resolve(ui.style()),
        };
        // Quantize the row metric to the physical pixel grid (stint 0529):
        // a fractional font row height makes `line * line_height` land off-grid
        // differently per line, so per-row rasterization yields uneven leading
        // (17,17,16,17… physical pixels). Rounding once here keeps every
        // `line_top` on-grid for any font size and display scale.
        let ppp = ui.ctx().pixels_per_point();
        let line_height = ui
            .fonts_mut(|f| f.row_height(&font_id))
            .round_to_pixels(ppp)
            .max(1.0);
        let (auto_id, rect) = ui.allocate_space(ui.available_size_before_wrap());
        let id = self.id.unwrap_or(auto_id);
        let response = ui.interact(rect, id, Sense::click_and_drag());

        let gutter_width = self.gutter_width(ui, &font_id, self.doc.buffer().line_count());
        self.view.line_height = line_height;
        self.view.viewport_height = rect.height();
        self.view.viewport_width = rect.width() - gutter_width;
        let soft_wrap = !self.mode.is_code();
        if soft_wrap {
            self.view.scroll_x = 0.0;
            let logged_id = id.with("soft_wrap_logged");
            let already_logged =
                ui.memory(|memory| memory.data.get_temp::<bool>(logged_id).unwrap_or(false));
            if !already_logged {
                log::info!(
                    "editor: soft wrap active mode={} width={:.1}",
                    self.mode.describe(),
                    self.view.viewport_width
                );
                ui.memory_mut(|memory| memory.data.insert_temp(logged_id, true));
            }
        }
        let geometry_cache_id = id.with("display_geometry");
        let mut line_galleys =
            self.cached_geometry_galleys(ui, &font_id, soft_wrap, geometry_cache_id);
        self.update_display_layout(&line_galleys);
        let page_rows = ((rect.height() / line_height).floor().max(1.0)) as usize;

        let mut commands: Vec<EditorCommand> = Vec::new();
        let revision_before_commands = self.doc.revision();
        let mut copy_requested = false;
        let mut cut_requested = false;

        if self.active {
            // Lock focus-traversal keys (Tab, arrows) to this widget so egui
            // never re-purposes them for widget navigation while editing.
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(
                    id,
                    egui::EventFilter {
                        tab: true,
                        horizontal_arrows: true,
                        vertical_arrows: true,
                        escape: false,
                    },
                );
            });
            let events = ui.input(|i| i.events.clone());
            for event in &events {
                match event {
                    Event::Text(text) => commands.push(EditorCommand::InsertText(text.clone())),
                    Event::Paste(text) => {
                        let replacement = if self.mode.is_markdown()
                            && self.doc.selection().is_range()
                            && is_bare_http_url(text)
                        {
                            let label = self
                                .doc
                                .selected_text()
                                .replace('\\', "\\\\")
                                .replace('[', "\\[")
                                .replace(']', "\\]");
                            format!("[{label}]({text})")
                        } else {
                            text.clone()
                        };
                        commands.push(EditorCommand::InsertText(replacement));
                    }
                    Event::Copy => copy_requested = true,
                    Event::Cut => cut_requested = true,
                    Event::Key {
                        key,
                        pressed: true,
                        modifiers,
                        ..
                    } => {
                        if *key == Key::Backspace && modifiers.command {
                            log::info!("editor: received Cmd+Backspace key event");
                        }
                        if let Some(cmd) = translate_key(*key, *modifiers, page_rows, &self.mode) {
                            commands.push(cmd);
                        }
                    }
                    Event::Ime(ime) => match ime {
                        ImeEvent::Preedit(text) => {
                            commands.push(EditorCommand::ImePreedit(text.clone()));
                        }
                        ImeEvent::Commit(text) => {
                            commands.push(EditorCommand::ImeCommit(text.clone()));
                        }
                        ImeEvent::Disabled => commands.push(EditorCommand::ImeCancel),
                        ImeEvent::Enabled => {}
                    },
                    _ => {}
                }
            }
        }

        if copy_requested || cut_requested {
            let selected = self.doc.selected_text();
            if !selected.is_empty() {
                ui.ctx().copy_text(selected);
                if cut_requested {
                    commands.push(EditorCommand::Backspace);
                }
            }
        }

        // Pointer: click to place, shift-click / drag to extend, double-click
        // word, triple-click line. An explicit modifier-click (Cmd on macOS)
        // over a link span activates the link instead of moving the caret;
        // every other click keeps ordinary caret semantics.
        let mut link_activation: Option<LinkTarget> = None;
        let mut pointer_visual_row: Option<usize> = None;
        if let Some(pos) = response.interact_pointer_pos() {
            let (cursor, hit_row) = self.hit_test(ui, rect, gutter_width, pos, &line_galleys);
            if response.clicked() && ui.input(|i| i.modifiers.command) && self.mode.is_markdown() {
                if let Some(cache) = self.md_cache.as_deref_mut() {
                    let layout = cache.layout_for(self.doc.buffer(), self.doc.revision());
                    let text = line_text(self.doc.buffer(), cursor.line);
                    let byte_in_line = text
                        .char_indices()
                        .nth(cursor.column)
                        .map_or(text.len(), |(i, _)| i);
                    let byte = layout.line_byte_start(cursor.line) + byte_in_line;
                    link_activation = layout.link_at_byte(byte).cloned();
                }
            }
            if link_activation.is_none() {
                if response.triple_clicked() {
                    commands.push(EditorCommand::SelectLineAt(cursor.line));
                } else if response.double_clicked() {
                    commands.push(EditorCommand::SelectWordAt(cursor));
                } else if response.drag_started() || response.clicked() {
                    pointer_visual_row = Some(hit_row);
                    if ui.input(|i| i.modifiers.shift) {
                        commands.push(EditorCommand::ExtendTo(cursor));
                    } else {
                        commands.push(EditorCommand::SetCursor(cursor));
                    }
                } else if response.dragged() {
                    pointer_visual_row = Some(hit_row);
                    commands.push(EditorCommand::ExtendTo(cursor));
                }
            }
        }

        let edited = !commands.is_empty();
        let visual_goal_id = id.with("visual_goal_x");
        let visual_row_id = id.with("visual_row");
        let mut visual_goal_x = ui.memory(|memory| memory.data.get_temp::<f32>(visual_goal_id));
        let mut visual_row = ui.memory(|memory| memory.data.get_temp::<usize>(visual_row_id));
        for command in commands {
            match command {
                EditorCommand::Move { movement, extend }
                    if soft_wrap
                        && matches!(
                            movement,
                            Movement::Up
                                | Movement::Down
                                | Movement::VisualLineStart
                                | Movement::VisualLineEnd
                                | Movement::PageUp(_)
                                | Movement::PageDown(_)
                        ) =>
                {
                    let head = self.doc.cursor();
                    let current_row =
                        visual_row.unwrap_or_else(|| self.view.layout.display_row_for_cursor(head));
                    let vertical = matches!(
                        movement,
                        Movement::Up | Movement::Down | Movement::PageUp(_) | Movement::PageDown(_)
                    );
                    let goal = if vertical {
                        *visual_goal_x.get_or_insert_with(|| {
                            self.cursor_display_x(head, &line_galleys, Some(current_row))
                        })
                    } else {
                        visual_goal_x = None;
                        0.0
                    };
                    let cursor = self.visual_movement_cursor(
                        head,
                        movement,
                        goal,
                        &line_galleys,
                        Some(current_row),
                    );
                    visual_row = Some(match movement {
                        Movement::Up => current_row.saturating_sub(1),
                        Movement::Down => (current_row + 1)
                            .min(self.view.layout.display_row_count().saturating_sub(1)),
                        Movement::PageUp(rows) => current_row.saturating_sub(rows.max(1)),
                        Movement::PageDown(rows) => (current_row + rows.max(1))
                            .min(self.view.layout.display_row_count().saturating_sub(1)),
                        _ => current_row,
                    });
                    self.doc.apply(if extend {
                        EditorCommand::ExtendTo(cursor)
                    } else {
                        EditorCommand::SetCursor(cursor)
                    });
                }
                other => {
                    if !matches!(
                        other,
                        EditorCommand::Move {
                            movement: Movement::Up | Movement::Down,
                            ..
                        }
                    ) {
                        visual_goal_x = None;
                        visual_row = None;
                    }
                    self.doc.apply(other);
                }
            }
        }
        if let Some(row) = pointer_visual_row {
            visual_row = Some(row);
        }
        ui.memory_mut(|memory| {
            if let Some(goal) = visual_goal_x {
                memory.data.insert_temp(visual_goal_id, goal);
            } else {
                memory.data.remove::<f32>(visual_goal_id);
            }
            if let Some(row) = visual_row {
                memory.data.insert_temp(visual_row_id, row);
            } else {
                memory.data.remove::<usize>(visual_row_id);
            }
        });

        // Edits can change row breaks. Rebuild before viewport/caret geometry
        // and painting; source remains authoritative throughout.
        if self.doc.revision() != revision_before_commands {
            line_galleys = self.geometry_galleys(ui, &font_id, soft_wrap);
            self.store_geometry_cache(
                ui,
                &font_id,
                soft_wrap,
                geometry_cache_id,
                line_galleys.clone(),
            );
            self.update_display_layout(&line_galleys);
        }

        // Live Preview: the blocks intersecting the selection reveal raw
        // source; everything else renders styled. Computed after commands so
        // it reflects this frame's final selection.
        let (sel_start, sel_end) = self.doc.selection().ordered();
        let md_active: Option<std::ops::Range<usize>> =
            if self.mode.is_live_preview() && self.active {
                self.md_cache.as_deref_mut().map(|cache| {
                    let layout = cache.layout_for(self.doc.buffer(), self.doc.revision());
                    layout.active_lines(sel_start.line, sel_end.line)
                })
            } else {
                None
            };
        let image_rows = self.update_image_extras(ui.ctx(), md_active.as_ref());

        // Scrolling: wheel when hovered, then keep the caret visible after
        // any command.
        let line_count = self.doc.buffer().line_count();
        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta);
            if scroll != egui::Vec2::ZERO {
                self.view.scroll_y -= scroll.y;
                if !soft_wrap {
                    self.view.scroll_x = (self.view.scroll_x - scroll.x).max(0.0);
                }
            }
        }
        self.view.clamp_scroll(line_count);
        if edited {
            let caret = self.doc.cursor();
            self.view.scroll_to_cursor(caret, line_count);
            if !soft_wrap {
                self.view
                    .scroll_to_x(self.cursor_display_x(caret, &line_galleys, None));
            }
        }

        // Blinking caret (only while this widget owns input). Any command this
        // frame (typing, edit, caret move, selection) counts as activity and
        // resets the blink phase so the caret is immediately solid; otherwise
        // it toggles every `CARET_BLINK_INTERVAL`. The phase timestamp lives in
        // egui memory keyed by the widget id, so it survives across the
        // per-frame reconstruction of this widget.
        let caret_visible = if self.active {
            let now = ui.input(|i| i.time);
            let phase_id = id.with("caret_blink_since");
            let last_activity = ui.memory_mut(|m| match m.data.get_temp::<f64>(phase_id) {
                Some(t) if !edited => t,
                _ => {
                    m.data.insert_temp(phase_id, now);
                    now
                }
            });
            let elapsed = now - last_activity;
            // Schedule the next repaint exactly at the upcoming flip. Add the
            // frame's predicted duration so egui's delayed-repaint advance
            // can't collapse the deadline into an immediate repaint loop.
            let predicted_dt = ui.input(|i| i.predicted_dt) as f64;
            let delay = caret_blink_next_toggle(elapsed, CARET_BLINK_INTERVAL) + predicted_dt;
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs_f64(delay));
            caret_blink_visible(elapsed, CARET_BLINK_INTERVAL)
        } else {
            // Inactive editors paint no caret at all; visibility is moot.
            true
        };

        self.paint(
            ui,
            id,
            &font_id,
            rect,
            gutter_width,
            md_active,
            &image_rows,
            &line_galleys,
            visual_row,
            caret_visible,
        );
        EditorOutput {
            response,
            link_activation,
        }
    }

    fn geometry_galleys(
        &self,
        ui: &Ui,
        font_id: &egui::FontId,
        soft_wrap: bool,
    ) -> Vec<DisplayGalley> {
        let wrap_width = if soft_wrap {
            self.view.viewport_width.max(1.0)
        } else {
            f32::INFINITY
        };
        (0..self.doc.buffer().line_count())
            .map(|line| {
                let text = line_text(self.doc.buffer(), line);
                let (display_text, display_to_source) =
                    prepared_display_text(ui, font_id, &text, wrap_width);
                let source_job =
                    LayoutJob::simple_singleline(text, font_id.clone(), egui::Color32::WHITE);
                let mut job = prepare_layout_job(source_job, &display_text, &display_to_source);
                job.wrap.max_width = wrap_width;
                // Prefer Unicode line-break opportunities. Egui still breaks
                // an individually overlong word to keep the width bound.
                job.wrap.break_anywhere = false;
                DisplayGalley {
                    galley: ui.fonts_mut(|fonts| fonts.layout_job(job)),
                    display_to_source,
                }
            })
            .collect()
    }

    fn cached_geometry_galleys(
        &self,
        ui: &Ui,
        font_id: &egui::FontId,
        soft_wrap: bool,
        cache_id: egui::Id,
    ) -> Vec<DisplayGalley> {
        let revision = self.doc.revision();
        let width_bits = self.view.viewport_width.to_bits();
        let pixels_per_point_bits = ui.ctx().pixels_per_point().to_bits();
        if let Some(cache) = ui.memory(|memory| memory.data.get_temp::<GeometryCache>(cache_id)) {
            if cache.revision == revision
                && cache.width_bits == width_bits
                && cache.pixels_per_point_bits == pixels_per_point_bits
                && cache.soft_wrap == soft_wrap
                && cache.font_id == *font_id
            {
                return cache.galleys;
            }
        }
        let galleys = self.geometry_galleys(ui, font_id, soft_wrap);
        self.store_geometry_cache(ui, font_id, soft_wrap, cache_id, galleys.clone());
        galleys
    }

    fn store_geometry_cache(
        &self,
        ui: &Ui,
        font_id: &egui::FontId,
        soft_wrap: bool,
        cache_id: egui::Id,
        galleys: Vec<DisplayGalley>,
    ) {
        let pixels_per_point_bits = ui.ctx().pixels_per_point().to_bits();
        ui.memory_mut(|memory| {
            memory.data.insert_temp(
                cache_id,
                GeometryCache {
                    revision: self.doc.revision(),
                    width_bits: self.view.viewport_width.to_bits(),
                    pixels_per_point_bits,
                    soft_wrap,
                    font_id: font_id.clone(),
                    galleys,
                },
            );
        });
    }

    fn update_display_layout(&mut self, galleys: &[DisplayGalley]) {
        let lines = galleys
            .iter()
            .enumerate()
            .map(|(source_line, galley)| {
                let row_counts: Vec<usize> = galley
                    .galley
                    .rows
                    .iter()
                    .map(|row| row.char_count_excluding_newline())
                    .collect();
                if galley
                    .display_to_source
                    .iter()
                    .enumerate()
                    .all(|(display, source)| display == *source)
                {
                    LineLayout::identity(source_line, galley.galley.text().to_string(), &row_counts)
                } else {
                    LineLayout::mapped(
                        source_line,
                        galley.galley.text().to_string(),
                        galley.display_to_source.clone(),
                        &row_counts,
                    )
                }
            })
            .collect();
        self.view.layout = DisplayLayout::new(lines);
    }

    fn cursor_display_x(
        &self,
        cursor: Cursor,
        galleys: &[DisplayGalley],
        preferred_row: Option<usize>,
    ) -> f32 {
        let Some(line_layout) = self.view.layout.line(cursor.line) else {
            return 0.0;
        };
        let Some(galley) = galleys.get(cursor.line) else {
            return 0.0;
        };
        let display_column = line_layout.display_column_for_source(cursor.column);
        if let Some(display_row) = preferred_row {
            let row_in_line =
                display_row.saturating_sub(self.view.layout.first_display_row(cursor.line));
            if let (Some(row), Some(mapped_row)) = (
                galley.galley.rows.get(row_in_line),
                line_layout.rows.get(row_in_line),
            ) {
                return row.x_offset(
                    display_column
                        .saturating_sub(mapped_row.display.start)
                        .min(row.char_count_excluding_newline()),
                );
            }
        }
        let prefer_next_row = line_layout
            .rows
            .iter()
            .any(|row| row.display.start == display_column && display_column != 0);
        galley
            .galley
            .pos_from_cursor(CCursor {
                index: display_column,
                prefer_next_row,
            })
            .left()
    }

    fn visual_movement_cursor(
        &self,
        cursor: Cursor,
        movement: Movement,
        goal_x: f32,
        galleys: &[DisplayGalley],
        preferred_row: Option<usize>,
    ) -> Cursor {
        let display_row =
            preferred_row.unwrap_or_else(|| self.view.layout.display_row_for_cursor(cursor));
        if matches!(
            movement,
            Movement::VisualLineStart | Movement::VisualLineEnd
        ) {
            return self
                .view
                .layout
                .cursor_at_display_row_boundary(display_row, movement == Movement::VisualLineEnd);
        }

        let target_row = match movement {
            Movement::Up => display_row.saturating_sub(1),
            Movement::Down => {
                (display_row + 1).min(self.view.layout.display_row_count().saturating_sub(1))
            }
            Movement::PageUp(rows) => display_row.saturating_sub(rows.max(1)),
            Movement::PageDown(rows) => (display_row + rows.max(1))
                .min(self.view.layout.display_row_count().saturating_sub(1)),
            _ => display_row,
        };
        let target_line = self.view.layout.source_line_at_display_row(target_row);
        let Some(line_layout) = self.view.layout.line(target_line) else {
            return cursor;
        };
        let Some(galley) = galleys.get(target_line) else {
            return cursor;
        };
        let row_in_line = target_row - self.view.layout.first_display_row(target_line);
        let Some(row) = galley.galley.rows.get(row_in_line) else {
            return cursor;
        };
        let Some(mapped_row) = line_layout.rows.get(row_in_line) else {
            return cursor;
        };
        let display_column = mapped_row.display.start + row.char_at(goal_x - row.pos.x);
        Cursor::new(
            target_line,
            line_layout.source_column_for_display(display_column),
        )
    }

    /// Reserves per-line extra height for inline image strips (Live Preview
    /// only) and returns the `(line, dest)` rows to paint. Lines inside the
    /// active (raw source) block render no strip.
    fn update_image_extras(
        &mut self,
        ctx: &egui::Context,
        md_active: Option<&std::ops::Range<usize>>,
    ) -> Vec<(usize, String)> {
        let enabled = self.mode.is_live_preview() && self.images.is_some();
        let Some(cache) = self.md_cache.as_deref_mut().filter(|_| enabled) else {
            self.view.line_extras.clear();
            return Vec::new();
        };
        let layout = cache.layout_for(self.doc.buffer(), self.doc.revision());
        let (image_cache, base) = self.images.as_mut().expect("checked above");
        let max_width = (self.view.viewport_width - 2.0 * IMAGE_PAD).max(16.0);
        let mut extras: Vec<(usize, f32)> = Vec::new();
        let mut rows: Vec<(usize, String)> = Vec::new();
        for span in &layout.images {
            let line = layout.line_of_byte(span.bytes.start);
            if md_active.is_some_and(|r| r.contains(&line)) {
                continue;
            }
            if rows.iter().any(|(l, _)| *l == line) {
                continue; // one strip per line
            }
            let height = match image_cache.get(ctx, base, &span.dest) {
                ImageState::Ready { size, .. } => {
                    let scale = (max_width / size[0] as f32).min(1.0);
                    (size[1] as f32 * scale).min(IMAGE_MAX_HEIGHT)
                }
                ImageState::Failed(_) | ImageState::Remote => IMAGE_PLACEHOLDER_HEIGHT,
            };
            // Quantized like `line_height` so `line_top` stays on the physical
            // pixel grid for every line below an attachment strip.
            extras.push((
                line,
                (height + 2.0 * IMAGE_PAD).round_to_pixels(ctx.pixels_per_point()),
            ));
            rows.push((line, span.dest.clone()));
        }
        extras.sort_by_key(|(l, _)| *l);
        self.view.line_extras = extras;
        rows
    }

    /// Width of the line-number gutter in code mode; zero otherwise.
    fn gutter_width(&self, ui: &Ui, font_id: &egui::FontId, line_count: usize) -> f32 {
        if !self.mode.is_code() {
            return 0.0;
        }
        let digits = (line_count.max(1).ilog10() as usize + 1).max(2);
        let char_width = ui.fonts_mut(|f| f.glyph_width(font_id, '0'));
        digits as f32 * char_width + 2.0 * GUTTER_PAD
    }

    /// Paint-time offsets snapped to the physical pixel grid (stint 0529):
    /// `(content_top, content_left, scroll_y, scroll_x)`. The scroll
    /// accumulators in [`ViewState`] stay fractional so momentum scrolling
    /// keeps its smooth feel; only the painted offset is rounded, so rows
    /// never straddle a pixel boundary mid-scroll. `paint` and `hit_test`
    /// both use these so what you click is what you see.
    fn snapped_offsets(&self, ui: &Ui, rect: Rect, gutter_width: f32) -> (f32, f32, f32, f32) {
        let ppp = ui.ctx().pixels_per_point();
        (
            rect.top().round_to_pixels(ppp),
            (rect.left() + gutter_width).round_to_pixels(ppp),
            self.view.scroll_y.round_to_pixels(ppp),
            self.view.scroll_x.round_to_pixels(ppp),
        )
    }

    /// Maps a pointer position to a document cursor via per-line galley
    /// hit-testing.
    fn hit_test(
        &self,
        ui: &Ui,
        rect: Rect,
        gutter_width: f32,
        pos: egui::Pos2,
        galleys: &[DisplayGalley],
    ) -> (Cursor, usize) {
        let line_count = self.doc.buffer().line_count();
        // Invert the same snapped offsets `paint` renders with so a click maps
        // to the row the user actually sees.
        let (content_top, content_left, scroll_y, scroll_x) =
            self.snapped_offsets(ui, rect, gutter_width);
        let y = pos.y - content_top + scroll_y;
        let line = self.view.line_at_y(y, line_count);
        let local_y = y - self.view.line_top(line);
        let Some(galley) = galleys.get(line) else {
            return (Cursor::new(line, 0), 0);
        };
        let display_cursor = galley
            .galley
            .cursor_from_pos(Vec2::new(pos.x - content_left + scroll_x, local_y));
        let source_column = self
            .view
            .layout
            .line(line)
            .map_or(display_cursor.index, |layout| {
                layout.source_column_for_display(display_cursor.index)
            });
        let row_in_line = galley.galley.layout_from_cursor(display_cursor).row;
        (
            Cursor::new(line, source_column),
            self.view.layout.first_display_row(line) + row_in_line,
        )
    }

    /// Lays out one line's galley: syntax-colored via the span provider in
    /// code mode, single-color otherwise (and as the unknown-language
    /// fallback).
    // Argument-struct refactor is a design change tracked in stint 0661;
    // this is the narrow exception, not a pattern to copy.
    #[allow(clippy::too_many_arguments)]
    fn line_galley(
        &mut self,
        ui: &Ui,
        font_id: &egui::FontId,
        line: usize,
        text: &str,
        text_color: Color32,
        theme: &CodeTheme,
        md_active: Option<&std::ops::Range<usize>>,
        wrap_width: f32,
        display: &DisplayGalley,
    ) -> std::sync::Arc<egui::Galley> {
        // Markdown Live Preview: inactive blocks render styled per-line
        // LayoutJobs; the active block (and everything in source mode) shows
        // raw source. Same font everywhere, so galley metrics — and with them
        // hit-testing and caret geometry — are identical to plain rendering.
        if self.mode.is_live_preview() && !text.is_empty() {
            let active = md_active.is_some_and(|r| r.contains(&line));
            if let (false, Some(cache)) = (active, self.md_cache.as_deref_mut()) {
                let md_theme = self
                    .md_theme
                    .unwrap_or_else(|| MarkdownTheme::from_visuals(ui.visuals()));
                let layout = cache.layout_for(self.doc.buffer(), self.doc.revision());
                let spans = layout.line_style_spans(line, text);
                if !spans.is_empty() {
                    let mut job = LayoutJob::default();
                    job.wrap.max_width = wrap_width;
                    job.wrap.break_anywhere = false;
                    for span in &spans {
                        let (color, italics, underline) =
                            md_theme.format_for(span.style, text_color);
                        let mut format = TextFormat::simple(font_id.clone(), color);
                        format.italics = italics;
                        if underline {
                            format.underline = egui::Stroke::new(1.0_f32, color);
                        }
                        job.append(&text[span.range.clone()], 0.0, format);
                    }
                    return ui.fonts_mut(|f| {
                        f.layout_job(prepare_layout_job(
                            job,
                            display.galley.text(),
                            &display.display_to_source,
                        ))
                    });
                }
            }
        }
        let spans = match (self.mode.is_code(), self.span_provider.as_deref_mut()) {
            (true, Some(provider)) => {
                provider.line_spans(self.doc.buffer(), line, self.doc.revision())
            }
            _ => &[],
        };
        if spans.is_empty() {
            let mut job =
                LayoutJob::simple_singleline(text.to_string(), font_id.clone(), text_color);
            job.wrap.max_width = wrap_width;
            job.wrap.break_anywhere = false;
            return ui.fonts_mut(|f| {
                f.layout_job(prepare_layout_job(
                    job,
                    display.galley.text(),
                    &display.display_to_source,
                ))
            });
        }
        let mut job = LayoutJob::default();
        job.wrap.max_width = wrap_width;
        job.wrap.break_anywhere = false;
        for span in spans {
            let (start, end) = (span.start.min(text.len()), span.end.min(text.len()));
            if start >= end {
                continue;
            }
            job.append(
                &text[start..end],
                0.0,
                TextFormat::simple(font_id.clone(), theme.color_for(span.kind, text_color)),
            );
        }
        ui.fonts_mut(|f| {
            f.layout_job(prepare_layout_job(
                job,
                display.galley.text(),
                &display.display_to_source,
            ))
        })
    }

    // Argument-struct refactor is a design change tracked in stint 0661;
    // this is the narrow exception, not a pattern to copy.
    #[allow(clippy::too_many_arguments)]
    fn paint(
        &mut self,
        ui: &Ui,
        widget_id: egui::Id,
        font_id: &egui::FontId,
        rect: Rect,
        gutter_width: f32,
        md_active: Option<std::ops::Range<usize>>,
        image_rows: &[(usize, String)],
        geometry_galleys: &[DisplayGalley],
        caret_visual_row: Option<usize>,
        caret_visible: bool,
    ) {
        let visuals = ui.visuals();
        let text_color = visuals.text_color();
        let code_theme = self
            .code_theme
            .unwrap_or_else(|| CodeTheme::from_visuals(visuals));
        // Text clips at the gutter edge so horizontal scroll never paints
        // under the line numbers.
        let text_rect =
            Rect::from_min_max(egui::pos2(rect.left() + gutter_width, rect.top()), rect.max);
        let painter = ui.painter_at(text_rect);
        let selection_color = visuals.selection.bg_fill.linear_multiply(0.5);
        let caret_color = visuals.selection.stroke.color;
        let ppp = ui.ctx().pixels_per_point();
        let (content_top, content_left, scroll_y, scroll_x) =
            self.snapped_offsets(ui, rect, gutter_width);
        let line_count = self.doc.buffer().line_count();
        let selection = self.doc.selection();
        let (sel_start, sel_end) = selection.ordered();
        let caret = self.doc.cursor();
        let mut caret_rect: Option<Rect> = None;
        let gutter_painter = ui.painter_at(rect);
        let show_code_chrome = self.mode.is_code();
        let wrap_width = if show_code_chrome {
            f32::INFINITY
        } else {
            self.view.viewport_width.max(1.0)
        };

        for line in self.view.visible_lines(line_count) {
            // On-grid by construction: snapped origin + quantized line metric
            // minus a snapped scroll offset.
            let top = content_top + self.view.line_top(line) - scroll_y;
            let text = line_text(self.doc.buffer(), line);
            let Some(display) = geometry_galleys.get(line) else {
                continue;
            };
            let galley = self.line_galley(
                ui,
                font_id,
                line,
                &text,
                text_color,
                &code_theme,
                md_active.as_ref(),
                wrap_width,
                display,
            );
            let origin = egui::pos2(content_left - scroll_x, top);

            if show_code_chrome {
                // Current-line highlight under everything else.
                if line == caret.line {
                    painter.rect_filled(
                        Rect::from_min_max(
                            egui::pos2(text_rect.left(), top),
                            egui::pos2(rect.right(), top + self.view.line_text_height(line)),
                        ),
                        0.0,
                        code_theme.current_line_bg,
                    );
                }
                // Right-aligned 1-based line number in the gutter, painted at
                // a snapped origin (the right-aligned x is fractional).
                let number = gutter_painter.layout_no_wrap(
                    (line + 1).to_string(),
                    font_id.clone(),
                    code_theme.gutter_text,
                );
                let number_pos = egui::pos2(content_left - GUTTER_PAD - number.size().x, top)
                    .round_to_pixels(ppp);
                gutter_painter.galley(number_pos, number, code_theme.gutter_text);
            }

            // Find-match highlight fills under the text and selection.
            if !self.highlights.is_empty() {
                let line_start = self.doc.buffer().line_to_char(line);
                let char_count = text.chars().count();
                let line_end_char = line_start + char_count;
                for (i, &(hs, he)) in self.highlights.iter().enumerate() {
                    if he <= line_start || hs >= line_end_char {
                        continue;
                    }
                    let from = hs.saturating_sub(line_start).min(char_count);
                    let to = (he - line_start).min(char_count);
                    let bg = if Some(i) == self.current_highlight {
                        self.current_highlight_bg
                    } else {
                        self.highlight_bg
                    };
                    let display_range = self.view.layout.line(line).map_or(from..to, |layout| {
                        layout.display_column_for_source(from)..layout.display_column_for_source(to)
                    });
                    for range_rect in galley_range_rects(&galley, display_range) {
                        painter.rect_filled(range_rect.translate(origin.to_vec2()), 0.0, bg);
                    }
                }
            }

            if selection.is_range() && selection.touches_line(line) {
                let char_count = text.chars().count();
                let from = if line == sel_start.line {
                    sel_start.column.min(char_count)
                } else {
                    0
                };
                // Interior lines extend one column past the end to show the
                // selected newline.
                let (to, extend_newline) = if line == sel_end.line {
                    (sel_end.column.min(char_count), false)
                } else {
                    (char_count, true)
                };
                let display_range = self.view.layout.line(line).map_or(from..to, |layout| {
                    layout.display_column_for_source(from)..layout.display_column_for_source(to)
                });
                let mut rects = galley_range_rects(&galley, display_range);
                if extend_newline {
                    if let Some(last) = rects.last_mut() {
                        last.max.x += self.view.line_height * 0.5;
                    } else if let Some(row) = galley.rows.last() {
                        rects.push(Rect::from_min_size(
                            egui::pos2(row.rect().right(), row.rect().top()),
                            Vec2::new(self.view.line_height * 0.5, row.rect().height()),
                        ));
                    }
                }
                for range_rect in rects {
                    painter.rect_filled(
                        range_rect.translate(origin.to_vec2()),
                        0.0,
                        selection_color,
                    );
                }
            }

            painter.galley(origin, galley.clone(), text_color);

            // Semantic mirror of the painted text: the editor draws galleys
            // directly (no egui text widgets), so without this the pane's
            // accesskit tree — and therefore `plexi pane state` and the scene
            // runner's `rendered_text_contains` evaluator — carries no
            // rendered text at all. One node per visible display row, with
            // display-only wrap characters removed through the source map.
            // No-op when accesskit is off.
            for (row_index, row) in galley.rows.iter().enumerate() {
                let rendered: String = self
                    .view
                    .layout
                    .line(line)
                    .and_then(|layout| layout.rows.get(row_index))
                    .map(|mapped_row| {
                        text.chars()
                            .skip(mapped_row.source.start)
                            .take(mapped_row.source.end - mapped_row.source.start)
                            .collect()
                    })
                    .unwrap_or_else(|| row.text());
                if rendered.is_empty() {
                    continue;
                }
                let node_id = egui::Id::new((widget_id, "editor_rendered_row", line, row_index));
                let node_rect = row.rect().translate(origin.to_vec2());
                ui.ctx().accesskit_node_builder(node_id, |node| {
                    node.set_role(egui::accesskit::Role::Paragraph);
                    node.set_value(rendered.as_str());
                    node.set_bounds(egui::accesskit::Rect {
                        x0: f64::from(node_rect.left()),
                        y0: f64::from(node_rect.top()),
                        x1: f64::from(node_rect.right()),
                        y1: f64::from(node_rect.bottom()),
                    });
                });
            }

            if self.active && line == caret.line {
                let preferred_row_in_line = caret_visual_row
                    .and_then(|display_row| {
                        let first = self.view.layout.first_display_row(line);
                        (display_row >= first
                            && display_row < first + self.view.layout.row_count(line))
                        .then_some(display_row - first)
                    })
                    .or_else(|| {
                        self.view
                            .layout
                            .line(line)
                            .map(|layout| layout.row_for_source_column(caret.column))
                    });
                let caret_pos = preferred_row_in_line
                    .and_then(|row_index| {
                        let layout = self.view.layout.line(line)?;
                        let mapped_row = layout.rows.get(row_index)?;
                        let row = galley.rows.get(row_index)?;
                        let display_column = (mapped_row.display.start..=mapped_row.display.end)
                            .find(|column| {
                                layout.source_column_for_display(*column) == caret.column
                            })?;
                        let x = row.pos.x + row.x_offset(display_column - mapped_row.display.start);
                        Some(Rect::from_min_max(
                            egui::pos2(x, row.rect().top()),
                            egui::pos2(x, row.rect().bottom()),
                        ))
                    })
                    .unwrap_or_else(|| {
                        let display_column = self
                            .view
                            .layout
                            .line(line)
                            .map_or(caret.column.min(text.chars().count()), |layout| {
                                layout.display_column_for_source(caret.column)
                            });
                        galley.pos_from_cursor(CCursor::new(display_column))
                    });
                // Caret and preedit sit on the pixel grid so the 1pt bar never
                // smears across two physical columns.
                let caret_x = (origin.x + caret_pos.left()).round_to_pixels(ppp);
                let caret_top = origin.y + caret_pos.top();

                // IME preedit paints as underlined overlay text at the caret.
                if let Some(preedit) = self.doc.ime().preedit().filter(|p| !p.is_empty()) {
                    let preedit_galley = ui.fonts_mut(|f| {
                        f.layout_no_wrap(preedit.to_string(), font_id.clone(), text_color)
                    });
                    let width = preedit_galley.size().x;
                    painter.galley(egui::pos2(caret_x, caret_top), preedit_galley, text_color);
                    painter.line_segment(
                        [
                            egui::pos2(caret_x, caret_top + self.view.line_height - 1.0),
                            egui::pos2(caret_x + width, caret_top + self.view.line_height - 1.0),
                        ],
                        egui::Stroke::new(1.0_f32, text_color),
                    );
                    caret_rect = Some(Rect::from_min_size(
                        egui::pos2(caret_x + width, caret_top),
                        Vec2::new(1.0, self.view.line_height),
                    ));
                } else {
                    caret_rect = Some(Rect::from_min_size(
                        egui::pos2(caret_x, caret_top),
                        Vec2::new(1.0, self.view.line_height),
                    ));
                }
            }
        }

        // Inline image strips below their source lines (Live Preview only):
        // bounded to content width, aspect preserved, placeholders for
        // remote/missing/undecodable destinations.
        if !image_rows.is_empty() {
            let visible = self.view.visible_lines(line_count);
            let weak = visuals.weak_text_color();
            if let Some((cache, base)) = self.images.as_mut() {
                for (line, dest) in image_rows {
                    if !visible.contains(line) {
                        continue;
                    }
                    let extra = self.view.line_extra(*line);
                    if extra <= 0.0 {
                        continue;
                    }
                    let strip_top = content_top + self.view.line_top(*line) - scroll_y
                        + self.view.line_text_height(*line)
                        + IMAGE_PAD;
                    let height = extra - 2.0 * IMAGE_PAD;
                    let left = content_left + IMAGE_PAD - scroll_x;
                    match cache.get(ui.ctx(), base, dest) {
                        ImageState::Ready { texture, size } => {
                            let scale = (height / size[1] as f32).min(1.0);
                            let width = size[0] as f32 * scale;
                            let image_rect = Rect::from_min_size(
                                egui::pos2(left, strip_top),
                                Vec2::new(width, height),
                            );
                            painter.image(
                                texture.id(),
                                image_rect,
                                Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                                Color32::WHITE,
                            );
                        }
                        state @ (ImageState::Failed(_) | ImageState::Remote) => {
                            let label = match state {
                                ImageState::Failed(reason) => {
                                    format!("image unavailable: {reason}")
                                }
                                _ => format!("remote image (not downloaded): {dest}"),
                            };
                            let strip_rect = Rect::from_min_size(
                                egui::pos2(left, strip_top),
                                Vec2::new(
                                    (self.view.viewport_width - 2.0 * IMAGE_PAD).max(16.0),
                                    height,
                                ),
                            );
                            painter.rect_stroke(
                                strip_rect,
                                2.0,
                                egui::Stroke::new(1.0_f32, weak),
                                egui::StrokeKind::Inside,
                            );
                            let label_galley = painter.layout_no_wrap(label, font_id.clone(), weak);
                            let label_pos = egui::pos2(
                                strip_rect.left() + 8.0,
                                strip_rect.center().y - label_galley.size().y / 2.0,
                            )
                            .round_to_pixels(ppp);
                            painter.galley(label_pos, label_galley, weak);
                        }
                    }
                }
            }
        }

        if let Some(caret_rect) = caret_rect {
            // The blink hides the fill on the off half-cycle; the IME anchor
            // below still tracks the caret so the candidate window stays put.
            if caret_visible {
                painter.rect_filled(caret_rect, 0.0, caret_color);
            }
            // Position the OS IME candidate window at the caret.
            ui.ctx().output_mut(|o| {
                o.ime = Some(egui::output::IMEOutput {
                    rect,
                    cursor_rect: caret_rect,
                });
            });
        }
    }
}

/// Maps a key press (with modifiers) to an editor command. Returns `None`
/// for keys the editor does not handle. `page_rows` is the viewport height in
/// lines, used by PageUp/PageDown. [`EditorMode::Markdown`] routes
/// Tab/Enter/Backspace to the Markdown-aware commands; plain and code modes
/// share the plain map (Tab/Shift-Tab already indent/outdent there).
fn translate_key(
    key: Key,
    modifiers: Modifiers,
    page_rows: usize,
    mode: &EditorMode,
) -> Option<EditorCommand> {
    let extend = modifiers.shift;
    let word = modifiers.alt;
    let line = modifiers.command;
    let mv = |movement: Movement| Some(EditorCommand::Move { movement, extend });
    if mode.is_markdown() {
        match key {
            Key::Tab if modifiers.shift => return Some(EditorCommand::MarkdownOutdent),
            Key::Tab if modifiers.is_none() => return Some(EditorCommand::MarkdownIndent),
            Key::Enter if modifiers.is_none() => return Some(EditorCommand::MarkdownNewline),
            Key::Backspace if modifiers.is_none() => return Some(EditorCommand::MarkdownBackspace),
            _ => {}
        }
    }
    match key {
        Key::Tab if modifiers.shift => Some(EditorCommand::Outdent),
        Key::Tab if modifiers.is_none() => Some(EditorCommand::Indent),
        Key::PageUp => mv(Movement::PageUp(page_rows)),
        Key::PageDown => mv(Movement::PageDown(page_rows)),
        Key::ArrowLeft if line => mv(Movement::LineStart),
        Key::ArrowLeft if word => mv(Movement::WordLeft),
        Key::ArrowLeft => mv(Movement::Left),
        Key::ArrowRight if line => mv(Movement::LineEnd),
        Key::ArrowRight if word => mv(Movement::WordRight),
        Key::ArrowRight => mv(Movement::Right),
        Key::ArrowUp if line => mv(Movement::DocStart),
        Key::ArrowUp => mv(Movement::Up),
        Key::ArrowDown if line => mv(Movement::DocEnd),
        Key::ArrowDown => mv(Movement::Down),
        Key::Home => mv(Movement::VisualLineStart),
        Key::End => mv(Movement::VisualLineEnd),
        Key::Backspace if modifiers.command && !modifiers.shift && !modifiers.alt => {
            Some(EditorCommand::KillToLineStart)
        }
        Key::Backspace => Some(EditorCommand::Backspace),
        Key::Delete => Some(EditorCommand::DeleteForward),
        Key::Enter => Some(EditorCommand::InsertNewline),
        Key::A if modifiers.command => Some(EditorCommand::SelectAll),
        Key::Z if modifiers.command && modifiers.shift => Some(EditorCommand::Redo),
        Key::Z if modifiers.command => Some(EditorCommand::Undo),
        Key::Y if modifiers.command => Some(EditorCommand::Redo),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::EditorSemanticState;

    struct TestState {
        doc: Document,
        view: ViewState,
        rect: Rect,
    }

    fn harness(text: &str) -> egui_kittest::Harness<'static, TestState> {
        let state = TestState {
            doc: Document::new(text),
            view: ViewState::default(),
            rect: Rect::NOTHING,
        };
        egui_kittest::Harness::new_ui_state(
            |ui, state: &mut TestState| {
                state.rect = EditorWidget::new(&mut state.doc, &mut state.view)
                    .show(ui)
                    .response
                    .rect;
            },
            state,
        )
    }

    fn semantic(h: &egui_kittest::Harness<'static, TestState>) -> EditorSemanticState {
        h.state().doc.semantic_state(h.state().view.scroll_y)
    }

    #[test]
    fn typing_keys_and_text_events_edit_document() {
        let mut h = harness("");
        h.event(Event::Text("hello".into()));
        h.step();
        h.key_press(Key::Enter);
        h.step();
        h.event(Event::Text("world".into()));
        h.step();

        let state = semantic(&h);
        assert_eq!(state.text, "hello\nworld");
        assert_eq!(state.cursor, Cursor::new(1, 5));

        // Backspace deletes; cmd+Z undoes the whole coalesced group.
        h.key_press(Key::Backspace);
        h.step();
        assert_eq!(semantic(&h).text, "hello\nworl");
        h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        h.step();
        assert_eq!(semantic(&h).text, "hello\nworld");
        assert!(semantic(&h).can_redo);
    }

    #[test]
    fn soft_wrap_breaks_overlong_tokens_only_at_grapheme_boundaries() {
        use unicode_segmentation::UnicodeSegmentation;

        let family = "👨\u{200D}👩\u{200D}👧";
        let text = family.repeat(12);
        let mut h = harness(&text);
        h.set_size(Vec2::new(120.0, 240.0));
        h.step();

        let line = h.state().view.layout.line(0).expect("line layout");
        assert!(line.rows.len() > 1, "long token should wrap");
        let grapheme_boundaries: Vec<usize> = std::iter::once(0)
            .chain(
                text.grapheme_indices(true)
                    .map(|(byte, grapheme)| text[..byte + grapheme.len()].chars().count()),
            )
            .collect();
        for row in &line.rows {
            assert!(
                grapheme_boundaries.contains(&row.source.start)
                    && grapheme_boundaries.contains(&row.source.end),
                "row {:?} split a grapheme cluster",
                row.source
            );
        }
    }

    #[test]
    fn wrapped_home_end_and_up_down_use_visual_rows_with_selection_extension() {
        let text = "abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";
        let mut h = harness(text);
        h.set_size(Vec2::new(120.0, 240.0));
        h.step();
        assert!(h.state().view.layout.line(0).unwrap().rows.len() >= 3);

        let second = h.state().view.layout.line(0).unwrap().rows[1]
            .source
            .clone();
        h.state_mut()
            .doc
            .apply(EditorCommand::SetCursor(Cursor::new(0, second.start + 2)));
        h.key_press(Key::End);
        h.step();
        assert_eq!(h.state().doc.cursor(), Cursor::new(0, second.end));

        h.key_press(Key::Home);
        h.step();
        assert_eq!(h.state().doc.cursor(), Cursor::new(0, second.start));
        let painted_caret = h
            .output()
            .platform_output
            .ime
            .as_ref()
            .expect("active editor IME anchor")
            .cursor_rect;
        assert!(
            painted_caret.top() >= h.state().rect.top() + h.state().view.line_height,
            "visual-row start caret painted on previous row: {painted_caret:?}"
        );

        h.key_press(Key::ArrowDown);
        h.step();
        let third = h.state().view.layout.line(0).unwrap().rows[2]
            .source
            .clone();
        assert_eq!(h.state().doc.cursor(), Cursor::new(0, third.start));

        h.key_press_modifiers(Modifiers::SHIFT, Key::ArrowUp);
        h.step();
        assert_eq!(
            h.state().doc.selection().anchor,
            Cursor::new(0, third.start)
        );
        assert_eq!(h.state().doc.selection().head, Cursor::new(0, second.start));
    }

    #[test]
    fn wrapped_command_arrows_stay_logical_and_page_keys_count_display_rows() {
        let text = "abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ".repeat(3);
        let mut h = harness(&text);
        h.set_size(Vec2::new(120.0, 70.0));
        h.step();
        let second = h.state().view.layout.line(0).unwrap().rows[1]
            .source
            .clone();
        h.state_mut()
            .doc
            .apply(EditorCommand::SetCursor(Cursor::new(0, second.start + 2)));

        h.key_press_modifiers(Modifiers::COMMAND, Key::ArrowLeft);
        h.step();
        assert_eq!(h.state().doc.cursor(), Cursor::new(0, 0));

        h.key_press(Key::PageDown);
        h.step();
        let page_rows = (h.state().view.viewport_height / h.state().view.line_height)
            .floor()
            .max(1.0) as usize;
        let expected = h.state().view.layout.line(0).unwrap().rows[page_rows]
            .source
            .start;
        assert_eq!(h.state().doc.cursor(), Cursor::new(0, expected));

        h.key_press_modifiers(Modifiers::COMMAND, Key::ArrowRight);
        h.step();
        assert_eq!(h.state().doc.cursor(), Cursor::new(0, text.chars().count()));
    }

    #[test]
    fn wrapped_pointer_hit_on_later_display_row_maps_to_source_cursor() {
        let text = "abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";
        let mut h = harness(text);
        h.set_size(Vec2::new(120.0, 240.0));
        h.step();
        let third = h.state().view.layout.line(0).unwrap().rows[2]
            .source
            .clone();
        let line_height = h.state().view.line_height;

        let pos = egui::pos2(
            h.state().rect.left() + 4.0,
            h.state().rect.top() + line_height * 2.5,
        );
        h.drag_at(pos);
        h.drop_at(pos);
        h.step();
        let cursor = h.state().doc.cursor();
        assert_eq!(cursor.line, 0);
        assert!(
            third.contains(&cursor.column) || cursor.column == third.end,
            "later-row hit returned {cursor:?}, outside {third:?}"
        );
    }

    #[test]
    fn shift_arrows_select_and_semantic_state_agrees() {
        let mut h = harness("abc def");
        h.key_press_modifiers(Modifiers::COMMAND, Key::A);
        h.step();
        let state = semantic(&h);
        assert_eq!(state.selection.ordered().1, Cursor::new(0, 7));

        h.key_press(Key::ArrowRight); // collapse to end
        h.step();
        h.key_press_modifiers(Modifiers::SHIFT | Modifiers::ALT, Key::ArrowLeft);
        h.step();
        let state = semantic(&h);
        assert_eq!(state.cursor, Cursor::new(0, 4));
        assert!(state.selection.is_range());
    }

    #[test]
    fn ime_events_compose_and_commit() {
        let mut h = harness("");
        h.event(Event::Ime(ImeEvent::Enabled));
        h.event(Event::Ime(ImeEvent::Preedit("かん".into())));
        h.step();
        let state = semantic(&h);
        assert_eq!(state.ime_composition, Some("かん".to_string()));
        assert_eq!(state.text, "");

        h.event(Event::Ime(ImeEvent::Commit("漢字".into())));
        h.event(Event::Ime(ImeEvent::Disabled));
        h.step();
        let state = semantic(&h);
        assert_eq!(state.text, "漢字");
        assert_eq!(state.ime_composition, None);
        assert_eq!(state.cursor, Cursor::new(0, 2));
    }

    #[test]
    fn tab_indents_and_shift_tab_outdents() {
        let mut h = harness("line");
        h.key_press(Key::Tab);
        h.step();
        assert_eq!(semantic(&h).text, "    line");
        h.key_press_modifiers(Modifiers::SHIFT, Key::Tab);
        h.step();
        assert_eq!(semantic(&h).text, "line");
    }

    fn inactive_harness(text: &str) -> egui_kittest::Harness<'static, TestState> {
        let state = TestState {
            doc: Document::new(text),
            view: ViewState::default(),
            rect: Rect::NOTHING,
        };
        egui_kittest::Harness::new_ui_state(
            |ui, state: &mut TestState| {
                EditorWidget::new(&mut state.doc, &mut state.view)
                    .active(false)
                    .show(ui);
            },
            state,
        )
    }

    #[test]
    fn inactive_widget_ignores_keyboard_and_text() {
        let mut h = inactive_harness("keep");
        h.event(Event::Text("nope".into()));
        h.step();
        h.key_press(Key::Backspace);
        h.step();
        assert_eq!(semantic(&h).text, "keep");
    }

    struct ModeState {
        doc: Document,
        view: ViewState,
        mode: EditorMode,
        highlighter: Option<crate::editor::SyntaxHighlighter>,
    }

    fn mode_harness(text: &str, mode: EditorMode) -> egui_kittest::Harness<'static, ModeState> {
        let highlighter = mode
            .language()
            .and_then(crate::editor::SyntaxHighlighter::new);
        let state = ModeState {
            doc: Document::new(text),
            view: ViewState::default(),
            mode,
            highlighter,
        };
        egui_kittest::Harness::new_ui_state(
            |ui, state: &mut ModeState| {
                let mut widget =
                    EditorWidget::new(&mut state.doc, &mut state.view).mode(state.mode.clone());
                if let Some(h) = &mut state.highlighter {
                    widget = widget.span_provider(h);
                }
                widget.show(ui);
            },
            state,
        )
    }

    #[test]
    fn code_mode_tab_indents_and_shift_tab_outdents() {
        let mut h = mode_harness(
            "fn main() {}",
            EditorMode::Code {
                language: "rs".into(),
            },
        );
        // Tab with a collapsed caret inserts one indent step at the caret.
        h.key_press(Key::Tab);
        h.step();
        assert_eq!(h.state().doc.text(), "    fn main() {}");
        // Shift-Tab outdents the line.
        h.key_press_modifiers(Modifiers::SHIFT, Key::Tab);
        h.step();
        assert_eq!(h.state().doc.text(), "fn main() {}");
        // Undo through the shared history works identically in code mode.
        h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        h.step();
        assert_eq!(h.state().doc.text(), "    fn main() {}");
    }

    #[test]
    fn mode_change_never_alters_document_state() {
        // Build up text, selection, history, and IME preedit in code mode…
        let mut h = mode_harness(
            "",
            EditorMode::Code {
                language: "rs".into(),
            },
        );
        h.event(Event::Text("héllo 😀".into()));
        h.step();
        h.key_press(Key::Enter);
        h.step();
        h.event(Event::Text("wörld".into()));
        h.step();
        h.key_press_modifiers(Modifiers::SHIFT | Modifiers::ALT, Key::ArrowLeft);
        h.step();
        h.event(Event::Ime(ImeEvent::Preedit("かん".into())));
        h.step();
        let before = h.state().doc.semantic_state(0.0);

        // …then flip through every mode without any input events: nothing in
        // the document (text, selection, undo history, IME) may change.
        for mode in [
            EditorMode::PlainText,
            EditorMode::Markdown {
                live_preview: false,
            },
            EditorMode::Markdown { live_preview: true },
            EditorMode::Code {
                language: "py".into(),
            },
        ] {
            h.state_mut().mode = mode.clone();
            h.state_mut().highlighter = mode
                .language()
                .and_then(crate::editor::SyntaxHighlighter::new);
            h.step();
            let after = h.state().doc.semantic_state(0.0);
            assert_eq!(after, before, "mode {mode:?} altered document state");
        }

        // Undo still walks the same history after the mode flips.
        h.event(Event::Ime(ImeEvent::Disabled));
        h.step();
        h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        h.step();
        assert_eq!(h.state().doc.text(), "héllo 😀\n");
    }

    #[test]
    fn unknown_code_language_falls_back_to_plain_spans() {
        let mode = EditorMode::Code {
            language: "not-a-language".into(),
        };
        assert!(mode
            .language()
            .and_then(crate::editor::SyntaxHighlighter::new)
            .is_none());
        // The widget still renders and edits without a provider.
        let mut h = mode_harness("some text", mode);
        h.event(Event::Text("!".into()));
        h.step();
        assert_eq!(h.state().doc.text(), "!some text");
    }

    struct PreviewState {
        doc: Document,
        view: ViewState,
        cache: MarkdownLayoutCache,
        live: bool,
    }

    fn preview_harness(text: &str) -> egui_kittest::Harness<'static, PreviewState> {
        let state = PreviewState {
            doc: Document::new(text),
            view: ViewState::default(),
            cache: MarkdownLayoutCache::default(),
            live: true,
        };
        egui_kittest::Harness::new_ui_state(
            |ui, state: &mut PreviewState| {
                EditorWidget::new(&mut state.doc, &mut state.view)
                    .mode(EditorMode::Markdown {
                        live_preview: state.live,
                    })
                    .markdown_preview(&mut state.cache)
                    .show(ui);
            },
            state,
        )
    }

    #[test]
    fn live_preview_editing_selection_and_undo_stay_coherent() {
        let mut h = preview_harness("# Title\n\npara **bold** text\n\n- item");
        // Move across blocks, then type into the paragraph.
        h.key_press(Key::ArrowDown);
        h.key_press(Key::ArrowDown);
        h.step();
        h.event(Event::Text("X".into()));
        h.step();
        assert_eq!(
            h.state().doc.text(),
            "# Title\n\nXpara **bold** text\n\n- item"
        );
        // Undo through the shared history, unchanged by preview rendering.
        h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        h.step();
        assert_eq!(
            h.state().doc.text(),
            "# Title\n\npara **bold** text\n\n- item"
        );
        // Cross-block select-all + copy path still sees the full source.
        h.key_press_modifiers(Modifiers::COMMAND, Key::A);
        h.step();
        assert_eq!(
            h.state().doc.selected_text(),
            "# Title\n\npara **bold** text\n\n- item"
        );
        // Flipping to source mode and back never alters document state.
        let before = h.state().doc.semantic_state(0.0);
        h.state_mut().live = false;
        h.step();
        h.state_mut().live = true;
        h.step();
        assert_eq!(h.state().doc.semantic_state(0.0), before);
    }

    #[test]
    fn live_preview_caret_movement_across_styled_blocks_hits_real_positions() {
        let mut h = preview_harness("# Head\ntext\n- one\n- two");
        // Walk the caret through every line end-to-end; every position is a
        // real source position (movement never skips styled markers).
        h.key_press_modifiers(Modifiers::COMMAND, Key::ArrowDown); // DocEnd
        h.step();
        assert_eq!(h.state().doc.cursor(), Cursor::new(3, 5));
        for _ in 0..(6 + 1 + 4 + 1 + 5 + 1 + 5) {
            h.key_press(Key::ArrowLeft);
        }
        h.step();
        assert_eq!(h.state().doc.cursor(), Cursor::new(0, 0));
    }

    #[test]
    fn paste_event_inserts_text() {
        let mut h = harness("ab");
        h.key_press(Key::ArrowRight);
        h.step();
        h.event(Event::Paste("XY".into()));
        h.step();
        assert_eq!(semantic(&h).text, "aXYb");
    }

    #[test]
    fn command_backspace_reaches_editor_and_kills_to_line_start() {
        let mut h = harness("one\nsecond");
        h.key_press_modifiers(Modifiers::COMMAND, Key::ArrowDown);
        h.step();
        h.key_press_modifiers(Modifiers::COMMAND, Key::Backspace);
        h.step();
        assert_eq!(semantic(&h).text, "one\n");
        assert_eq!(
            translate_key(
                Key::Backspace,
                Modifiers::COMMAND,
                1,
                &EditorMode::PlainText
            ),
            Some(EditorCommand::KillToLineStart)
        );
    }

    #[test]
    fn markdown_url_paste_over_selection_wraps_once_and_other_pastes_do_not() {
        let mut h = preview_harness("Plexi");
        h.key_press_modifiers(Modifiers::COMMAND, Key::A);
        h.step();
        h.event(Event::Paste("https://plexiapp.com".into()));
        h.step();
        assert_eq!(h.state().doc.text(), "[Plexi](https://plexiapp.com)");
        h.state_mut().doc.apply(EditorCommand::Undo);
        h.event(Event::Paste("not a URL".into()));
        h.step();
        assert_eq!(h.state().doc.text(), "not a URL");

        let mut h = preview_harness("foo]bar\\baz");
        h.key_press_modifiers(Modifiers::COMMAND, Key::A);
        h.step();
        h.event(Event::Paste("https://plexiapp.com".into()));
        h.step();
        assert_eq!(
            h.state().doc.text(),
            "[foo\\]bar\\\\baz](https://plexiapp.com)"
        );
    }

    #[test]
    fn caret_blink_is_solid_for_the_first_interval_then_toggles() {
        let iv = CARET_BLINK_INTERVAL;
        // Just after activity: solid.
        assert!(caret_blink_visible(0.0, iv));
        assert!(caret_blink_visible(iv * 0.99, iv));
        // First off half-cycle.
        assert!(!caret_blink_visible(iv * 1.01, iv));
        assert!(!caret_blink_visible(iv * 1.99, iv));
        // Back on.
        assert!(caret_blink_visible(iv * 2.01, iv));
        // A negative (clock went backwards) elapsed is treated as solid.
        assert!(caret_blink_visible(-1.0, iv));
    }

    #[test]
    fn caret_blink_next_toggle_is_the_positive_remainder_to_the_boundary() {
        let iv = CARET_BLINK_INTERVAL;
        assert!((caret_blink_next_toggle(0.0, iv) - iv).abs() < 1e-9);
        assert!((caret_blink_next_toggle(iv * 0.25, iv) - iv * 0.75).abs() < 1e-9);
        assert!((caret_blink_next_toggle(iv * 1.5, iv) - iv * 0.5).abs() < 1e-9);
        // Never zero (a zero-delay repaint would busy-loop the host).
        assert!(caret_blink_next_toggle(iv, iv) > 0.0);
        assert!(caret_blink_next_toggle(iv * 2.0, iv) > 0.0);
    }

    /// Stint 0529 gate: with the row metric quantized the way `show` does it
    /// (real font metrics rounded to the physical pixel grid), every painted
    /// row top lands on the grid and consecutive rows are uniformly spaced —
    /// across font sizes 9–32 and at both ppp 1.0 and 2.0. Fractional metrics
    /// used to yield 17,17,16,17… physical-pixel leading.
    #[test]
    fn painted_row_tops_uniform_in_physical_pixels_across_sizes_and_ppp() {
        let ctx = egui::Context::default();
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            for ppp in [1.0_f32, 2.0] {
                for size in 9..=32 {
                    let font_id = egui::FontId::monospace(size as f32);
                    let raw = ui.fonts_mut(|f| f.row_height(&font_id));
                    let line_height = raw.round_to_pixels(ppp).max(1.0);
                    let view = ViewState {
                        line_height,
                        ..ViewState::default()
                    };
                    let expected_phys = (line_height * ppp).round();
                    assert!(
                        ((line_height * ppp) - expected_phys).abs() < 1e-3,
                        "quantized line_height {line_height} not integer-physical at ppp {ppp}"
                    );
                    let mut prev = view.line_top(0) * ppp;
                    for line in 1..200 {
                        let top = view.line_top(line) * ppp;
                        let spacing = top - prev;
                        assert!(
                            (spacing - expected_phys).abs() < 1e-2,
                            "row spacing {spacing} != {expected_phys} physical px \
                             (font {size}, ppp {ppp}, line {line})"
                        );
                        assert!(
                            (top - top.round()).abs() < 1e-2,
                            "row top {top} off the pixel grid (font {size}, ppp {ppp}, line {line})"
                        );
                        prev = top;
                    }
                }
            }
        });
    }
}

mod helpers;
mod icons;

use crate::app::app_trait::{App, AppCommand, AppRenderContext};
use crate::ui::hints::{HintBar, HintGroup};
use crate::ui::list::ListRow;
use crate::ui::overlay::ModalShell;
use crate::ui::style;
use crate::ui::theme::Colors;
use egui::{Color32, CornerRadius, Stroke, StrokeKind};
use image::imageops::FilterType;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use helpers::{
    format_modified, format_size, is_text_preview_candidate, read_text_preview, sort_entries,
    ColumnConfig, ColumnId, ColumnModel, DirStats, Entry, MediaKind, SortDirection, TextPreview,
};
use icons::paint_entry_icon;

const DETAILS_TABLE_MIN_WIDTH: f32 = 560.0;
const INSPECTOR_MIN_WIDTH: f32 = 920.0;
const DIR_PREVIEW_CAP: usize = 500;
const DETAILS_HEADER_H: f32 = 28.0;
const DETAILS_ROW_H: f32 = style::LIST_ROW_DENSE_H;
// Total footer reservation subtracted from the body scroll area: SPACE_XS
// gap + separator + the status row + item spacing. Must cover everything
// draw_status_bar allocates or the footer collides with the list rows.
const STATUS_BAR_H: f32 = 44.0;
// Height of the single status row inside that reservation.
const STATUS_BAR_ROW_H: f32 = 28.0;
const INSPECTOR_DEFAULT_WIDTH: f32 = 280.0;
const INSPECTOR_MIN_PANEL_WIDTH: f32 = 220.0;
const INSPECTOR_SPLITTER_W: f32 = 7.0;
const QUICK_LOOK_TEXT_LINES: usize = 24;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileBrowserLayout {
    CompactList,
    DetailsTable,
}

impl FileBrowserLayout {
    fn for_width(width: f32) -> Self {
        if width >= DETAILS_TABLE_MIN_WIDTH {
            Self::DetailsTable
        } else {
            Self::CompactList
        }
    }

    fn shows_details(self) -> bool {
        matches!(self, Self::DetailsTable)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Normal,
    Search,
    Empty,
}

#[derive(Debug)]
enum FileBrowserAction {
    SelectNext,
    SelectPrev,
    SelectFirst,
    SelectLast,
    PageDown,
    PageUp,
    Activate,
    NavigateUp,
    Backspace,
    Escape,
    EnterSearch,
    ToggleSort,
    ToggleInspector,
    ToggleQuickLook,
    Refresh,
    CdTerminalAndClose,
    SelectAll,
    NewFolder,
    Rename,
    Copy,
    Cut,
    Paste,
    Duplicate,
    MoveToTrash,
    Reveal,
    OpenWithDefault,
    AppendText(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileOperation {
    Copy,
    Cut,
}

#[derive(Debug, Clone)]
struct FileOperationClipboard {
    operation: FileOperation,
    paths: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
enum PendingFileOperation {
    MoveToTrash { paths: Vec<PathBuf> },
}

/// Elide a path label from the left so the leaf directory stays visible —
/// `…/projects/plexi/src` reads better than `/Users/ian/Documents/proj…`.
/// Width-measured against the actual font, not a char-count guess.
fn elide_path_leading(
    ui: &egui::Ui,
    path: &str,
    font_id: egui::FontId,
    max_width: f32,
) -> String {
    if max_width <= 0.0 {
        return String::new();
    }
    let width = |s: &str| {
        ui.fonts(|f| {
            f.layout_no_wrap(s.to_string(), font_id.clone(), Color32::WHITE)
                .size()
                .x
        })
    };
    if width(path) <= max_width {
        return path.to_string();
    }
    const ELLIPSIS: char = '\u{2026}';
    let chars: Vec<char> = path.chars().collect();
    for keep in (1..chars.len()).rev() {
        let candidate: String = std::iter::once(ELLIPSIS)
            .chain(chars[chars.len() - keep..].iter().copied())
            .collect();
        if width(&candidate) <= max_width {
            return candidate;
        }
    }
    ELLIPSIS.to_string()
}

fn key_pressed_no_repeat(input: &egui::InputState, key: egui::Key) -> bool {
    input.events.iter().any(
        |e| matches!(e, egui::Event::Key { key: k, pressed: true, repeat: false, .. } if *k == key),
    )
}

// in_search gates all letter keys so that j/k/h/l/s/r/slash fall through
// to AppendText instead of firing navigation/action commands while the user
// is typing a query. Arrow keys, Enter, Escape, and Backspace always work.
fn classify_key(input: &egui::InputState, in_search: bool) -> Option<FileBrowserAction> {
    if !in_search && input.modifiers.command && key_pressed_no_repeat(input, egui::Key::A) {
        return Some(FileBrowserAction::SelectAll);
    }
    if !in_search && input.modifiers.command && key_pressed_no_repeat(input, egui::Key::N) {
        return Some(FileBrowserAction::NewFolder);
    }
    if !in_search && input.modifiers.command && key_pressed_no_repeat(input, egui::Key::R) {
        return Some(FileBrowserAction::Rename);
    }
    if !in_search && input.modifiers.command && key_pressed_no_repeat(input, egui::Key::C) {
        return Some(FileBrowserAction::Copy);
    }
    if !in_search && input.modifiers.command && key_pressed_no_repeat(input, egui::Key::X) {
        return Some(FileBrowserAction::Cut);
    }
    if !in_search && input.modifiers.command && key_pressed_no_repeat(input, egui::Key::V) {
        return Some(FileBrowserAction::Paste);
    }
    if !in_search && input.modifiers.command && key_pressed_no_repeat(input, egui::Key::D) {
        return Some(FileBrowserAction::Duplicate);
    }
    if !in_search && input.modifiers.command && key_pressed_no_repeat(input, egui::Key::Backspace) {
        return Some(FileBrowserAction::MoveToTrash);
    }
    if !in_search && input.modifiers.command && key_pressed_no_repeat(input, egui::Key::O) {
        return Some(FileBrowserAction::OpenWithDefault);
    }
    if !in_search && input.modifiers.command && key_pressed_no_repeat(input, egui::Key::Enter) {
        return Some(FileBrowserAction::Reveal);
    }
    if key_pressed_no_repeat(input, egui::Key::Escape) {
        return Some(FileBrowserAction::Escape);
    }
    if key_pressed_no_repeat(input, egui::Key::Backspace) {
        return Some(FileBrowserAction::Backspace);
    }
    if input.key_pressed(egui::Key::ArrowDown)
        || (!in_search && input.key_pressed(egui::Key::J) && !input.modifiers.any())
    {
        return Some(FileBrowserAction::SelectNext);
    }
    if input.key_pressed(egui::Key::ArrowUp)
        || (!in_search && input.key_pressed(egui::Key::K) && !input.modifiers.any())
    {
        return Some(FileBrowserAction::SelectPrev);
    }
    if input.key_pressed(egui::Key::Home) {
        return Some(FileBrowserAction::SelectFirst);
    }
    if input.key_pressed(egui::Key::End) {
        return Some(FileBrowserAction::SelectLast);
    }
    if input.key_pressed(egui::Key::PageDown) {
        return Some(FileBrowserAction::PageDown);
    }
    if input.key_pressed(egui::Key::PageUp) {
        return Some(FileBrowserAction::PageUp);
    }
    if !input.modifiers.command
        && (key_pressed_no_repeat(input, egui::Key::Enter)
            || key_pressed_no_repeat(input, egui::Key::ArrowRight)
            || (!in_search && key_pressed_no_repeat(input, egui::Key::L)))
    {
        return Some(FileBrowserAction::Activate);
    }
    if !input.modifiers.command
        && (key_pressed_no_repeat(input, egui::Key::ArrowLeft)
            || (!in_search && key_pressed_no_repeat(input, egui::Key::H)))
    {
        return Some(FileBrowserAction::NavigateUp);
    }
    if !in_search && key_pressed_no_repeat(input, egui::Key::Slash) && !input.modifiers.command {
        return Some(FileBrowserAction::EnterSearch);
    }
    if !in_search && key_pressed_no_repeat(input, egui::Key::S) && !input.modifiers.any() {
        return Some(FileBrowserAction::ToggleSort);
    }
    if !in_search && key_pressed_no_repeat(input, egui::Key::I) && !input.modifiers.any() {
        return Some(FileBrowserAction::ToggleInspector);
    }
    if !in_search && key_pressed_no_repeat(input, egui::Key::Space) && !input.modifiers.any() {
        return Some(FileBrowserAction::ToggleQuickLook);
    }
    if !in_search && key_pressed_no_repeat(input, egui::Key::R) && !input.modifiers.any() {
        return Some(FileBrowserAction::Refresh);
    }
    if !in_search && key_pressed_no_repeat(input, egui::Key::T) && !input.modifiers.any() {
        return Some(FileBrowserAction::CdTerminalAndClose);
    }
    let mut text = String::new();
    for event in &input.events {
        if let egui::Event::Text(t) = event {
            text.push_str(t);
        }
    }
    if !text.is_empty() {
        return Some(FileBrowserAction::AppendText(text));
    }
    None
}

pub struct FileBrowserApp {
    pub cwd: PathBuf,
    entries: Vec<Entry>,
    selected: usize,
    multi_selected: BTreeSet<PathBuf>,
    selection_anchor: Option<usize>,
    columns: ColumnModel,
    error: Option<String>,
    // Image preview
    preview_texture: Option<egui::TextureHandle>,
    preview_texture_path: Option<PathBuf>,
    preview_size: Option<[usize; 2]>,
    preview_error: Option<String>,
    // Dir preview
    dir_preview_path: Option<PathBuf>,
    dir_preview_stats: Option<DirStats>,
    // Text preview
    text_preview_path: Option<PathBuf>,
    text_preview: Option<TextPreview>,
    text_preview_error: Option<String>,
    inspector_open: bool,
    inspector_width: f32,
    quick_look_open: bool,
    operation_clipboard: Option<FileOperationClipboard>,
    pending_operation: Option<PendingFileOperation>,
    rename_path: Option<PathBuf>,
    rename_buffer: String,
    pending_cmds: Vec<AppCommand>,
    /// When true, the next draw_list pass will scroll the selected row into view.
    pending_scroll: bool,
    /// Remembers which entry was selected when leaving a directory,
    /// so navigating back restores the selection.
    directory_selection_memory: std::collections::HashMap<PathBuf, String>,
    // Fuzzy search
    in_search: bool,
    search_query: String,
    search_indices: Vec<usize>, // indices into `entries` that match the query
    should_close: bool,
    /// In tests: collects paths passed to open_file instead of spawning the system opener.
    #[cfg(test)]
    pub(crate) opened_files: Vec<PathBuf>,
}

impl FileBrowserApp {
    pub fn new(cwd: PathBuf) -> Self {
        let mut app = Self {
            cwd,
            entries: Vec::new(),
            selected: 0,
            multi_selected: BTreeSet::new(),
            selection_anchor: None,
            columns: ColumnModel::default(),
            error: None,
            preview_texture: None,
            preview_texture_path: None,
            preview_size: None,
            preview_error: None,
            dir_preview_path: None,
            dir_preview_stats: None,
            text_preview_path: None,
            text_preview: None,
            text_preview_error: None,
            inspector_open: false,
            inspector_width: INSPECTOR_DEFAULT_WIDTH,
            quick_look_open: false,
            operation_clipboard: None,
            pending_operation: None,
            rename_path: None,
            rename_buffer: String::new(),
            pending_cmds: Vec::new(),
            pending_scroll: false,
            directory_selection_memory: std::collections::HashMap::new(),
            in_search: false,
            search_query: String::new(),
            search_indices: Vec::new(),
            should_close: false,
            #[cfg(test)]
            opened_files: Vec::new(),
        };
        app.refresh();
        app
    }

    fn refresh(&mut self) {
        self.entries.clear();
        self.error = None;
        match fs::read_dir(&self.cwd) {
            Ok(dir) => {
                let mut entries: Vec<Entry> = dir
                    .filter_map(|e| {
                        let e = e.ok()?;
                        let name = e.file_name().to_string_lossy().to_string();
                        if name.starts_with('.') {
                            return None;
                        }
                        let path = e.path();
                        let is_dir = e.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
                        let meta = e.metadata().ok();
                        let size_bytes =
                            meta.as_ref()
                                .and_then(|m| if is_dir { None } else { Some(m.len()) });
                        let modified = meta.as_ref().and_then(|m| m.modified().ok());
                        let created = meta.as_ref().and_then(|m| m.created().ok());
                        let ext = path
                            .extension()
                            .and_then(|e| e.to_str())
                            .map(|e| e.to_ascii_lowercase());
                        let permissions = meta
                            .as_ref()
                            .map(|m| {
                                if m.permissions().readonly() {
                                    "ro"
                                } else {
                                    "rw"
                                }
                            })
                            .unwrap_or("?");
                        let is_image = ext
                            .as_deref()
                            .map(|e| {
                                matches!(
                                    e,
                                    "png" | "jpg" | "jpeg" | "gif" | "bmp" | "tiff" | "webp"
                                )
                            })
                            .unwrap_or(false);
                        Some(Entry {
                            name,
                            path,
                            is_dir,
                            is_image,
                            size_bytes,
                            modified,
                            created,
                            extension: ext,
                            permissions: permissions.to_string(),
                            tags: Vec::new(),
                        })
                    })
                    .collect();

                sort_entries(&mut entries, self.columns.sort, self.columns.folders_on_top);
                self.entries = entries;
                self.selected = self.selected.min(self.entries.len().saturating_sub(1));
                let live_paths = self
                    .entries
                    .iter()
                    .map(|entry| entry.path.clone())
                    .collect::<BTreeSet<_>>();
                self.multi_selected.retain(|path| live_paths.contains(path));
            }
            Err(e) => {
                self.error = Some(format!("Cannot read directory: {e}"));
            }
        }
    }

    fn refresh_preserving_filter(&mut self) {
        self.refresh();
        if self.in_search {
            self.refilter();
        }
    }

    fn navigate_into(&mut self, path: PathBuf) {
        if let Some(entry) = self.selected_entry() {
            self.directory_selection_memory
                .insert(self.cwd.clone(), entry.name.clone());
        }
        self.cwd = path;
        self.selected = 0;
        self.refresh();
        self.pending_scroll = true;
        self.clear_multi_selection();
    }

    /// Open a file the user activated (Enter, double-click, search-Enter).
    /// GUI↔Terminal media bridge (#79): recognised video/audio extensions
    /// route to the in-app players (`video-player` / `audio-player`) so
    /// the user stays inside the canvas. Everything else falls through
    /// to the system default opener (`open` on macOS, `xdg-open` on
    /// Linux). Failures to spawn the system opener are logged and
    /// silently swallowed — they're not user-recoverable from the file
    /// browser.
    fn open_file(&mut self, path: &Path) {
        let kind = MediaKind::for_path(path);
        if let Some(app_id) = kind.player_app_id() {
            log::info!(
                "file_browser: routing {kind:?} file '{}' to in-app player '{app_id}'",
                path.display()
            );
            self.pending_cmds.push(AppCommand::SpawnApp {
                type_id: app_id.to_string(),
                layout: None,
                args: vec![path.to_string_lossy().to_string()],
            });
            return;
        }
        #[cfg(test)]
        {
            self.opened_files.push(path.to_path_buf());
            return;
        }
        #[cfg(not(test))]
        match std::process::Command::new(Self::system_opener())
            .arg(path)
            .status()
        {
            Ok(status) if status.success() => {
                log::info!("file_browser: opened '{}'", path.display())
            }
            Ok(status) => log::error!(
                "file_browser: system-open failed for '{}': {status}",
                path.display()
            ),
            Err(e) => log::error!(
                "file_browser: system-open failed to spawn for '{}': {e}",
                path.display()
            ),
        }
    }

    /// Platform-appropriate fallback opener. macOS / Linux only — Windows
    /// callers fall through to a no-op since the media-bridge surface is
    /// unix-first for v3.4 (mirrors `canvas_bindings::shell_open`).
    #[cfg(not(test))]
    fn system_opener() -> &'static str {
        #[cfg(target_os = "macos")]
        {
            "open"
        }
        #[cfg(target_os = "linux")]
        {
            "xdg-open"
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            // The Command will fail to spawn; the error log makes the
            // platform gap visible without panicking.
            "open"
        }
    }

    /// Explicit cwd handoff (#2145): the ONLY path that writes a `cd` into
    /// the linked terminal. Browsing must never inject keystrokes into the
    /// terminal behind the explorer — a coding agent may be running there.
    fn cd_terminal_and_close(&mut self) {
        log::info!(
            "file_browser: explicit cd handoff to linked terminal: '{}'",
            self.cwd.display()
        );
        self.pending_cmds.push(AppCommand::CdRequest {
            cwd: self.cwd.to_string_lossy().to_string(),
            sender_pane_id: 0, // dispatch.rs stamps the real pane_id
        });
        self.should_close = true;
    }

    fn navigate_up(&mut self) {
        if let Some(parent) = self.cwd.parent().map(|p| p.to_path_buf()) {
            let leaving_name = self
                .cwd
                .file_name()
                .map(|n| n.to_string_lossy().to_string());
            self.cwd = parent;
            self.selected = 0;
            self.refresh();
            let restore_name = self
                .directory_selection_memory
                .remove(&self.cwd)
                .or(leaving_name);
            if let Some(name) = restore_name {
                if let Some(idx) = self.entries.iter().position(|e| e.name == name) {
                    self.selected = idx;
                }
            }
            self.pending_scroll = true;
        }
    }

    fn selected_entry(&self) -> Option<&Entry> {
        if self.in_search {
            self.search_indices
                .get(self.selected)
                .and_then(|&i| self.entries.get(i))
        } else {
            self.entries.get(self.selected)
        }
    }

    fn selected_visible_entry_indices(&self) -> Vec<usize> {
        let mut indices = self
            .visible_entry_indices()
            .into_iter()
            .filter(|idx| {
                self.entries
                    .get(*idx)
                    .map(|entry| self.multi_selected.contains(&entry.path))
                    .unwrap_or(false)
            })
            .collect::<Vec<_>>();
        if indices.is_empty() {
            if let Some(actual_idx) = self.visible_entry_indices().get(self.selected).copied() {
                indices.push(actual_idx);
            }
        }
        indices
    }

    fn selected_paths(&self) -> Vec<PathBuf> {
        self.selected_visible_entry_indices()
            .into_iter()
            .filter_map(|idx| self.entries.get(idx).map(|entry| entry.path.clone()))
            .collect()
    }

    fn selected_count(&self) -> usize {
        if self.multi_selected.is_empty() {
            usize::from(self.selected_entry().is_some())
        } else {
            self.multi_selected.len()
        }
    }

    fn is_entry_selected(&self, visible_idx: usize, entry: &Entry) -> bool {
        if self.multi_selected.is_empty() {
            self.selected == visible_idx
        } else {
            self.multi_selected.contains(&entry.path)
        }
    }

    fn clear_multi_selection(&mut self) {
        self.multi_selected.clear();
        self.selection_anchor = None;
    }

    fn set_single_selection(&mut self, visible_idx: usize) {
        self.selected = visible_idx;
        self.clear_multi_selection();
        self.selection_anchor = Some(visible_idx);
    }

    fn toggle_selection(&mut self, visible_idx: usize) {
        let Some(actual_idx) = self.visible_entry_indices().get(visible_idx).copied() else {
            return;
        };
        let Some(path) = self.entries.get(actual_idx).map(|entry| entry.path.clone()) else {
            return;
        };
        if !self.multi_selected.remove(&path) {
            self.multi_selected.insert(path);
        }
        self.selected = visible_idx;
        self.selection_anchor = Some(visible_idx);
        log::info!(
            "file_browser: selection changed count={}",
            self.selected_count()
        );
    }

    fn extend_selection_to(&mut self, visible_idx: usize) {
        let anchor = self.selection_anchor.unwrap_or(self.selected);
        self.multi_selected.clear();
        let (start, end) = if anchor <= visible_idx {
            (anchor, visible_idx)
        } else {
            (visible_idx, anchor)
        };
        let visible = self.visible_entry_indices();
        for idx in start..=end {
            if let Some(actual_idx) = visible.get(idx).copied() {
                if let Some(entry) = self.entries.get(actual_idx) {
                    self.multi_selected.insert(entry.path.clone());
                }
            }
        }
        self.selected = visible_idx;
        log::info!(
            "file_browser: range selection changed count={}",
            self.selected_count()
        );
    }

    fn select_all_visible(&mut self) {
        self.multi_selected = self
            .visible_entry_indices()
            .into_iter()
            .filter_map(|idx| self.entries.get(idx).map(|entry| entry.path.clone()))
            .collect();
        self.selection_anchor = Some(0);
        log::info!(
            "file_browser: selected all visible entries count={}",
            self.selected_count()
        );
    }

    fn refilter(&mut self) {
        let q = self.search_query.to_lowercase();
        self.search_indices = (0..self.entries.len())
            .filter(|&i| {
                if q.is_empty() {
                    return true;
                }
                let name = self.entries[i].name.to_lowercase();
                let mut qi = q.chars().peekable();
                for c in name.chars() {
                    if qi.peek() == Some(&c) {
                        qi.next();
                    }
                }
                qi.peek().is_none()
            })
            .collect();
        self.selected = 0;
        self.pending_scroll = true;
    }

    fn exit_search(&mut self) {
        self.in_search = false;
        self.search_query.clear();
        self.search_indices.clear();
        self.selected = 0;
        self.pending_scroll = true;
    }

    /// Called by the host when the linked terminal's CWD changes.
    pub fn sync_cwd(&mut self, new_cwd: PathBuf) {
        if new_cwd == self.cwd {
            return;
        }
        if let Some(entry) = self.selected_entry() {
            self.directory_selection_memory
                .insert(self.cwd.clone(), entry.name.clone());
        }
        let restore_name = self.directory_selection_memory.get(&new_cwd).cloned();
        self.cwd = new_cwd;
        self.selected = 0;
        self.refresh();
        if let Some(name) = restore_name {
            if let Some(idx) = self.entries.iter().position(|e| e.name == name) {
                self.selected = idx;
            }
        }
        self.pending_scroll = true;
    }

    // ─── Preview ensure methods ──────────────────────────────────────────────

    fn ensure_image_preview(&mut self, ctx: &egui::Context, path: &Path) {
        if self.preview_texture_path.as_deref() == Some(path) {
            return;
        }
        self.preview_texture = None;
        self.preview_texture_path = Some(path.to_path_buf());
        self.preview_size = None;
        self.preview_error = None;

        let decoded = match image::open(path) {
            Ok(img) => img,
            Err(e) => {
                self.preview_error = Some(format!("Unable to decode image: {e}"));
                return;
            }
        };
        let decoded = if decoded.width().max(decoded.height()) > 2048 {
            decoded.resize(2048, 2048, FilterType::Triangle)
        } else {
            decoded
        };
        let rgba = decoded.to_rgba8();
        let size = [rgba.width() as usize, rgba.height() as usize];
        let color_image = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
        let texture = ctx.load_texture(
            format!("fb-preview:{}", path.display()),
            color_image,
            egui::TextureOptions::LINEAR,
        );
        self.preview_size = Some(size);
        self.preview_texture = Some(texture);
    }

    fn ensure_dir_preview(&mut self, path: &Path) {
        if self.dir_preview_path.as_deref() == Some(path) {
            return;
        }
        self.dir_preview_path = Some(path.to_path_buf());
        let mut file_count = 0usize;
        let mut dir_count = 0usize;
        let mut total_bytes = 0u64;
        let mut truncated = false;
        if let Ok(entries) = fs::read_dir(path) {
            for e in entries.flatten() {
                if file_count + dir_count >= DIR_PREVIEW_CAP {
                    truncated = true;
                    break;
                }
                let is_dir = e.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
                if is_dir {
                    dir_count += 1;
                } else {
                    file_count += 1;
                    if let Ok(m) = e.metadata() {
                        total_bytes += m.len();
                    }
                }
            }
        }
        self.dir_preview_stats = Some(DirStats {
            file_count,
            dir_count,
            total_bytes,
            truncated,
        });
    }

    fn ensure_text_preview(&mut self, path: &Path) {
        if self.text_preview_path.as_deref() == Some(path) {
            return;
        }
        self.text_preview_path = Some(path.to_path_buf());
        self.text_preview = None;
        self.text_preview_error = None;

        match read_text_preview(path) {
            Ok(preview) => self.text_preview = Some(preview),
            Err(err) => {
                log::warn!(
                    "file_browser: text preview failed for '{}': {err}",
                    path.display()
                );
                self.text_preview_error = Some(err);
            }
        }
    }

    fn toggle_inspector(&mut self) {
        self.inspector_open = !self.inspector_open;
        log::info!(
            "file_browser: inspector {}",
            if self.inspector_open {
                "opened"
            } else {
                "closed"
            }
        );
    }

    fn should_show_inspector(&self, width: f32) -> bool {
        self.inspector_open && width >= INSPECTOR_MIN_WIDTH
    }

    fn toggle_quick_look(&mut self) {
        if self.quick_look_open {
            self.quick_look_open = false;
            log::info!("file_browser: quick look dismissed");
            return;
        }
        if let Some(path) = self.selected_entry().map(|entry| entry.path.clone()) {
            self.quick_look_open = true;
            log::info!("file_browser: quick look opened for '{}'", path.display());
        }
    }

    fn dismiss_quick_look(&mut self) {
        if self.quick_look_open {
            self.quick_look_open = false;
            log::info!("file_browser: quick look dismissed");
        }
    }

    fn unique_child_path(parent: &Path, stem: &str, extension: Option<&str>) -> PathBuf {
        let mut index = 0usize;
        loop {
            let name = if index == 0 {
                match extension {
                    Some(ext) if !ext.is_empty() => format!("{stem}.{ext}"),
                    _ => stem.to_string(),
                }
            } else {
                match extension {
                    Some(ext) if !ext.is_empty() => format!("{stem} {index}.{ext}"),
                    _ => format!("{stem} {index}"),
                }
            };
            let path = parent.join(name);
            if !path.exists() {
                return path;
            }
            index += 1;
        }
    }

    fn create_new_folder(&mut self) -> Result<PathBuf, String> {
        let path = Self::unique_child_path(&self.cwd, "Untitled Folder", None);
        fs::create_dir(&path).map_err(|err| {
            let msg = format!("Unable to create folder '{}': {err}", path.display());
            log::warn!("file_browser: {msg}");
            msg
        })?;
        log::info!("file_browser: created folder '{}'", path.display());
        self.refresh_preserving_filter();
        self.select_path(&path);
        Ok(path)
    }

    fn rename_selected(&mut self, new_name: &str) -> Result<PathBuf, String> {
        let Some(source) = self
            .rename_path
            .clone()
            .or_else(|| self.selected_paths().first().cloned())
        else {
            return Err("No selected file to rename".to_string());
        };
        let trimmed = new_name.trim();
        if trimmed.is_empty() || trimmed.contains('/') {
            return Err("Rename requires a non-empty file name".to_string());
        }
        let target = self.cwd.join(trimmed);
        fs::rename(&source, &target).map_err(|err| {
            let msg = format!(
                "Unable to rename '{}' to '{}': {err}",
                source.display(),
                target.display()
            );
            log::warn!("file_browser: {msg}");
            msg
        })?;
        log::info!(
            "file_browser: renamed '{}' to '{}'",
            source.display(),
            target.display()
        );
        self.refresh_preserving_filter();
        self.select_path(&target);
        Ok(target)
    }

    fn open_rename_modal(&mut self) {
        let Some(entry) = self.selected_entry().cloned() else {
            return;
        };
        self.rename_path = Some(entry.path);
        self.rename_buffer = entry.name;
        log::info!("file_browser: rename prompt opened");
    }

    fn confirm_rename_modal(&mut self) {
        let new_name = self.rename_buffer.clone();
        match self.rename_selected(&new_name) {
            Ok(_) => {
                self.rename_path = None;
                self.rename_buffer.clear();
            }
            Err(err) => self.error = Some(err),
        }
    }

    fn cancel_rename_modal(&mut self) {
        if self.rename_path.is_some() {
            log::info!("file_browser: rename prompt cancelled");
        }
        self.rename_path = None;
        self.rename_buffer.clear();
    }

    fn copy_selected(&mut self, operation: FileOperation) {
        let paths = self.selected_paths();
        if paths.is_empty() {
            return;
        }
        self.operation_clipboard = Some(FileOperationClipboard {
            operation,
            paths: paths.clone(),
        });
        log::info!(
            "file_browser: staged {} path(s) for {:?}",
            paths.len(),
            operation
        );
    }

    fn copy_path_recursive(source: &Path, target: &Path) -> Result<(), String> {
        if source.is_dir() {
            fs::create_dir(target).map_err(|err| {
                format!(
                    "Unable to create copied folder '{}': {err}",
                    target.display()
                )
            })?;
            let entries = fs::read_dir(source)
                .map_err(|err| format!("Unable to read folder '{}': {err}", source.display()))?;
            for entry in entries {
                let entry = entry.map_err(|err| format!("Unable to read folder entry: {err}"))?;
                Self::copy_path_recursive(&entry.path(), &target.join(entry.file_name()))?;
            }
        } else {
            fs::copy(source, target).map_err(|err| {
                format!(
                    "Unable to copy '{}' to '{}': {err}",
                    source.display(),
                    target.display()
                )
            })?;
        }
        Ok(())
    }

    fn paste_into_current_dir(&mut self) -> Result<Vec<PathBuf>, String> {
        let Some(clipboard) = self.operation_clipboard.clone() else {
            return Ok(Vec::new());
        };
        let mut created = Vec::new();
        for source in &clipboard.paths {
            let Some(file_name) = source.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            let target = Self::unique_child_path(&self.cwd, file_name, None);
            match clipboard.operation {
                FileOperation::Copy => Self::copy_path_recursive(source, &target)?,
                FileOperation::Cut => fs::rename(source, &target).map_err(|err| {
                    format!(
                        "Unable to move '{}' to '{}': {err}",
                        source.display(),
                        target.display()
                    )
                })?,
            }
            created.push(target);
        }
        if clipboard.operation == FileOperation::Cut {
            self.operation_clipboard = None;
        }
        log::info!(
            "file_browser: pasted {} path(s) into '{}'",
            created.len(),
            self.cwd.display()
        );
        self.refresh_preserving_filter();
        if let Some(path) = created.first() {
            self.select_path(path);
        }
        Ok(created)
    }

    fn duplicate_selected(&mut self) -> Result<Vec<PathBuf>, String> {
        let mut created = Vec::new();
        for source in self.selected_paths() {
            let stem = source
                .file_stem()
                .and_then(|stem| stem.to_str())
                .or_else(|| source.file_name().and_then(|name| name.to_str()))
                .unwrap_or("Untitled");
            let ext = source.extension().and_then(|ext| ext.to_str());
            let target = Self::unique_child_path(&self.cwd, &format!("{stem} copy"), ext);
            Self::copy_path_recursive(&source, &target)?;
            created.push(target);
        }
        log::info!("file_browser: duplicated {} path(s)", created.len());
        self.refresh_preserving_filter();
        self.multi_selected = created.iter().cloned().collect();
        if let Some(first) = created.first() {
            self.select_path(first);
            self.multi_selected = created.iter().cloned().collect();
        }
        Ok(created)
    }

    fn request_move_selected_to_trash(&mut self) {
        let paths = self.selected_paths();
        if paths.is_empty() {
            return;
        }
        log::info!(
            "file_browser: requested move to trash confirmation count={}",
            paths.len()
        );
        self.pending_operation = Some(PendingFileOperation::MoveToTrash { paths });
    }

    fn confirm_pending_operation(&mut self) -> Result<(), String> {
        let Some(operation) = self.pending_operation.take() else {
            return Ok(());
        };
        match operation {
            PendingFileOperation::MoveToTrash { paths } => {
                let trash_dir = self.cwd.join(".Trash");
                fs::create_dir_all(&trash_dir).map_err(|err| {
                    format!(
                        "Unable to create trash folder '{}': {err}",
                        trash_dir.display()
                    )
                })?;
                for source in paths {
                    let Some(file_name) = source.file_name().and_then(|name| name.to_str()) else {
                        continue;
                    };
                    let target = Self::unique_child_path(&trash_dir, file_name, None);
                    fs::rename(&source, &target).map_err(|err| {
                        format!(
                            "Unable to move '{}' to trash '{}': {err}",
                            source.display(),
                            target.display()
                        )
                    })?;
                    log::info!(
                        "file_browser: moved '{}' to trash '{}'",
                        source.display(),
                        target.display()
                    );
                }
            }
        }
        self.refresh_preserving_filter();
        self.clear_multi_selection();
        Ok(())
    }

    fn cancel_pending_operation(&mut self) {
        if self.pending_operation.is_some() {
            log::info!("file_browser: cancelled pending file operation");
        }
        self.pending_operation = None;
    }

    fn reveal_selected(&mut self) {
        for path in self.selected_paths() {
            log::info!("file_browser: reveal '{}'", path.display());
            self.open_file(&path);
        }
    }

    fn open_selected_with_default(&mut self) {
        for path in self.selected_paths() {
            self.open_file(&path);
        }
    }

    fn select_path(&mut self, path: &Path) {
        if let Some(idx) = self
            .visible_entry_indices()
            .into_iter()
            .position(|actual_idx| {
                self.entries
                    .get(actual_idx)
                    .map(|entry| entry.path == path)
                    .unwrap_or(false)
            })
        {
            self.selected = idx;
            self.selection_anchor = Some(idx);
            self.pending_scroll = true;
        }
    }

    // ─── Drawing ─────────────────────────────────────────────────────────────

    fn visible_entry_count(&self) -> usize {
        if self.in_search {
            self.search_indices.len()
        } else {
            self.entries.len()
        }
    }

    fn visible_entry_indices(&self) -> Vec<usize> {
        if self.in_search {
            self.search_indices.clone()
        } else {
            (0..self.entries.len()).collect()
        }
    }

    fn entry_title(entry: &Entry) -> String {
        if entry.is_dir {
            format!("{}/", entry.name)
        } else {
            entry.name.clone()
        }
    }

    fn entry_kind(entry: &Entry) -> &'static str {
        if entry.is_dir {
            "Folder"
        } else if entry.is_image {
            "Image"
        } else {
            "File"
        }
    }

    fn entry_cell_text(entry: &Entry, column: ColumnId) -> String {
        match column {
            ColumnId::Name => Self::entry_title(entry),
            ColumnId::Kind => Self::entry_kind(entry).to_string(),
            ColumnId::Size => format_size(entry.size_bytes),
            ColumnId::Modified => format_modified(entry.modified),
            ColumnId::Created => format_modified(entry.created),
            ColumnId::Extension => entry
                .extension
                .clone()
                .unwrap_or_else(|| "\u{2014}".to_string()),
            ColumnId::Permissions => entry.permissions.clone(),
            ColumnId::Tags => {
                if entry.tags.is_empty() {
                    "\u{2014}".to_string()
                } else {
                    entry.tags.join(", ")
                }
            }
        }
    }

    fn entry_chip(entry: &Entry) -> &'static str {
        if entry.is_dir {
            "dir"
        } else {
            "file"
        }
    }

    fn draw_compact_list(&mut self, ui: &mut egui::Ui, colors: &Colors) -> Option<(PathBuf, bool)> {
        let mut navigate_to: Option<(PathBuf, bool)> = None;
        let should_scroll = self.pending_scroll;
        self.pending_scroll = false;
        for (idx, actual_idx) in self.visible_entry_indices().into_iter().enumerate() {
            let entry = self.entries[actual_idx].clone();
            let is_selected = self.is_entry_selected(idx, &entry);
            // Compact rows are single-line per the file-explorer PRD —
            // metadata lives in the details table and the status bar, not
            // in a second line that doubles every row's height.
            let title = Self::entry_title(&entry);
            let response = ListRow::new(&title)
                .chip(Self::entry_chip(&entry))
                .dense()
                .selected(is_selected)
                .show(ui, colors);
            if is_selected {
                response.scroll_into_view(ui, should_scroll);
            }
            if response.row_clicked() {
                let modifiers = ui.input(|input| input.modifiers);
                if modifiers.shift {
                    self.extend_selection_to(idx);
                } else if modifiers.command {
                    self.toggle_selection(idx);
                } else {
                    self.set_single_selection(idx);
                }
            }
            if response.row_double_clicked() {
                self.set_single_selection(idx);
                navigate_to = Some((entry.path.clone(), entry.is_dir));
            }
        }
        navigate_to
    }

    fn draw_details_table(
        &mut self,
        ui: &mut egui::Ui,
        colors: &Colors,
    ) -> Option<(PathBuf, bool)> {
        let should_scroll = self.pending_scroll;
        self.pending_scroll = false;
        self.draw_details_header(ui, colors);

        let mut navigate_to = None;
        for (idx, actual_idx) in self.visible_entry_indices().into_iter().enumerate() {
            let entry = self.entries[actual_idx].clone();
            let (rect, response) = ui.allocate_exact_size(
                egui::vec2(ui.available_width(), DETAILS_ROW_H),
                egui::Sense::click(),
            );
            let selected = self.is_entry_selected(idx, &entry);
            if self.selected == idx {
                crate::ui::list::scroll_row_into_view(ui, &response, should_scroll);
            }
            let fill = if selected {
                colors.bg_active
            } else if response.hovered() {
                colors.bg_hover
            } else {
                Color32::TRANSPARENT
            };
            ui.painter().rect_filled(rect, style::RADIUS_MD, fill);

            self.paint_details_cells(ui, colors, rect, &entry, selected);

            if response.clicked() {
                let modifiers = ui.input(|input| input.modifiers);
                if modifiers.shift {
                    self.extend_selection_to(idx);
                } else if modifiers.command {
                    self.toggle_selection(idx);
                } else {
                    self.set_single_selection(idx);
                }
            }
            if response.double_clicked() {
                self.set_single_selection(idx);
                navigate_to = Some((entry.path.clone(), entry.is_dir));
            }
        }
        navigate_to
    }

    fn draw_details_header(&mut self, ui: &mut egui::Ui, colors: &Colors) {
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), DETAILS_HEADER_H),
            egui::Sense::hover(),
        );
        ui.painter()
            .rect_filled(rect, style::RADIUS_MD, colors.bg_sidebar);
        ui.painter().rect_stroke(
            rect,
            style::RADIUS_MD,
            Stroke::new(1.0, colors.border),
            StrokeKind::Inside,
        );
        for (column, cell) in self.details_columns(rect) {
            let handle_rect = egui::Rect::from_min_max(
                egui::pos2(cell.right() - 5.0, cell.top()),
                egui::pos2(cell.right() + 5.0, cell.bottom()),
            );
            let header_response = ui.interact(
                cell,
                ui.id().with(("file-browser-header", column.id.key())),
                egui::Sense::click_and_drag(),
            );
            let resize_response = ui.interact(
                handle_rect,
                ui.id().with(("file-browser-resize", column.id.key())),
                egui::Sense::drag(),
            );
            if header_response.clicked() {
                self.columns.toggle_sort(column.id);
                self.refresh_preserving_filter();
                log::info!(
                    "file_browser: sort changed to {} {}",
                    self.columns.sort.column.key(),
                    self.columns.sort.direction.key()
                );
            }
            if header_response.drag_stopped() {
                let delta = header_response.drag_delta().x;
                let offset = if delta < -cell.width() * 0.35 {
                    Some(-1)
                } else if delta > cell.width() * 0.35 {
                    Some(1)
                } else {
                    None
                };
                if let Some(offset) = offset {
                    self.columns.move_column(column.id, offset);
                    log::info!(
                        "file_browser: moved column {} by {}",
                        column.id.key(),
                        offset
                    );
                }
            }
            if resize_response.dragged() {
                let frame_delta = ui.input(|input| input.pointer.delta().x);
                self.columns
                    .resize_column(column.id, column.width + frame_delta);
                log::info!(
                    "file_browser: resized column {} to {:.1}",
                    column.id.key(),
                    self.columns
                        .columns
                        .iter()
                        .find(|candidate| candidate.id == column.id)
                        .map(|candidate| candidate.width)
                        .unwrap_or(column.width)
                );
            }
            let sort_suffix = if self.columns.sort.column == column.id {
                match self.columns.sort.direction {
                    SortDirection::Asc => " \u{2191}",
                    SortDirection::Desc => " \u{2193}",
                }
            } else {
                ""
            };
            ui.painter().text(
                egui::pos2(cell.left() + style::SPACE_SM, cell.center().y),
                egui::Align2::LEFT_CENTER,
                format!("{}{}", column.id.label(), sort_suffix),
                egui::FontId::proportional(style::TEXT_HINT),
                colors.text_dim,
            );
            ui.painter().line_segment(
                [
                    egui::pos2(cell.right(), cell.top() + 5.0),
                    egui::pos2(cell.right(), cell.bottom() - 5.0),
                ],
                Stroke::new(1.0, colors.border),
            );
        }
    }

    fn paint_details_cells(
        &self,
        ui: &mut egui::Ui,
        colors: &Colors,
        rect: egui::Rect,
        entry: &Entry,
        selected: bool,
    ) {
        let primary = if selected {
            colors.text_primary
        } else {
            colors.text_dim
        };
        let font = egui::FontId::proportional(style::TEXT_HINT);
        for (column, mut cell) in self.details_columns(rect) {
            if column.id == ColumnId::Name {
                let icon_rect = egui::Rect::from_min_size(
                    egui::pos2(cell.left() + style::SPACE_SM, cell.center().y - 9.0),
                    egui::vec2(18.0, 18.0),
                );
                paint_entry_icon(ui.painter(), icon_rect, entry, colors);
                cell = cell.shrink2(egui::vec2(28.0, 0.0));
            }
            let text_pos = egui::pos2(cell.left() + style::SPACE_SM, cell.center().y);
            ui.painter().text(
                text_pos,
                egui::Align2::LEFT_CENTER,
                Self::entry_cell_text(entry, column.id),
                font.clone(),
                if column.id == ColumnId::Name {
                    primary
                } else {
                    colors.text_dim
                },
            );
        }
    }

    fn details_columns(&self, rect: egui::Rect) -> Vec<(ColumnConfig, egui::Rect)> {
        let mut x = rect.left();
        let visible = self.columns.visible_columns().copied().collect::<Vec<_>>();
        let total_config_width = visible
            .iter()
            .map(|column| column.width.max(column.id.min_width()))
            .sum::<f32>()
            .max(1.0);
        let scale = if total_config_width > rect.width() {
            rect.width() / total_config_width
        } else {
            1.0
        };
        let mut columns = Vec::with_capacity(visible.len());
        for (idx, column) in visible.iter().enumerate() {
            let right = if idx + 1 == visible.len() {
                rect.right()
            } else {
                (x + column.width * scale).min(rect.right())
            };
            columns.push((
                *column,
                egui::Rect::from_min_max(
                    egui::pos2(x, rect.top()),
                    egui::pos2(right, rect.bottom()),
                ),
            ));
            x = right;
        }
        columns
    }

    fn draw_sidebar_preview(&mut self, ui: &mut egui::Ui, colors: &Colors) {
        let entry = match self.selected_entry().cloned() {
            Some(e) => e,
            None => return,
        };

        if entry.is_image {
            self.draw_image_sidebar(ui, colors, &entry);
        } else if entry.is_dir {
            self.draw_dir_sidebar(ui, colors, &entry);
        } else if is_text_preview_candidate(&entry.path) {
            self.draw_text_sidebar(ui, colors, &entry);
        } else {
            self.draw_generic_sidebar(ui, colors, &entry);
        }
    }

    fn draw_image_sidebar(&mut self, ui: &mut egui::Ui, colors: &Colors, entry: &Entry) {
        let path = entry.path.clone();
        self.ensure_image_preview(ui.ctx(), &path);

        egui::Frame::new()
            .fill(colors.bg_sidebar)
            .stroke(Stroke::new(1.0, colors.border))
            .corner_radius(CornerRadius::same(6))
            .inner_margin(egui::Margin::same(8))
            .show(ui, |ui| {
                ui.label(
                    egui::RichText::new(format!("Image \u{00b7} {}", entry.name))
                        .size(10.5)
                        .color(colors.text_primary)
                        .strong(),
                );
                if let Some([w, h]) = self.preview_size {
                    ui.label(
                        egui::RichText::new(format!("{w}\u{00d7}{h}"))
                            .size(10.0)
                            .color(colors.text_dim),
                    );
                }
                ui.add_space(6.0);
                let preview_max = egui::vec2(ui.available_width(), 220.0);
                let (slot_rect, _) = ui.allocate_exact_size(preview_max, egui::Sense::hover());
                ui.painter().rect_filled(
                    slot_rect,
                    CornerRadius::same(4),
                    colors.bg_darkest.gamma_multiply(0.95),
                );
                ui.painter().rect_stroke(
                    slot_rect,
                    CornerRadius::same(4),
                    Stroke::new(1.0, colors.border),
                    StrokeKind::Inside,
                );
                if let Some(texture) = &self.preview_texture {
                    let tex_size = egui::vec2(texture.size()[0] as f32, texture.size()[1] as f32);
                    let scale = (slot_rect.width() / tex_size.x)
                        .min(slot_rect.height() / tex_size.y)
                        .min(1.0);
                    let draw_size = tex_size * scale.max(0.01);
                    let image_rect = egui::Rect::from_center_size(slot_rect.center(), draw_size);
                    ui.painter().image(
                        texture.id(),
                        image_rect,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                } else if let Some(err) = &self.preview_error {
                    ui.painter().text(
                        slot_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        err,
                        egui::FontId::proportional(10.0),
                        Color32::from_rgb(0xff, 0xaf, 0xaf),
                    );
                } else {
                    ui.painter().text(
                        slot_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "Loading\u{2026}",
                        egui::FontId::proportional(10.0),
                        colors.text_dim,
                    );
                }
                ui.add_space(6.0);
                if let Some(size) = entry.size_bytes {
                    ui.label(
                        egui::RichText::new(format_size(Some(size)))
                            .size(9.5)
                            .color(colors.text_dim),
                    );
                }
            });
    }

    fn draw_dir_sidebar(&mut self, ui: &mut egui::Ui, colors: &Colors, entry: &Entry) {
        let path = entry.path.clone();
        self.ensure_dir_preview(&path);
        let stats = self.dir_preview_stats;

        egui::Frame::new()
            .fill(colors.bg_sidebar)
            .stroke(Stroke::new(1.0, colors.border))
            .corner_radius(CornerRadius::same(6))
            .inner_margin(egui::Margin::same(8))
            .show(ui, |ui| {
                ui.label(
                    egui::RichText::new(format!("Folder \u{00b7} {}", entry.name))
                        .size(10.5)
                        .color(colors.text_primary)
                        .strong(),
                );
                ui.separator();
                ui.label(
                    egui::RichText::new(format!("Path: {}", entry.path.display()))
                        .size(9.5)
                        .color(colors.text_dim),
                );
                let truncated = stats.map(|s| s.truncated).unwrap_or(false);
                let suffix = if truncated { "+" } else { "" };
                ui.label(
                    egui::RichText::new(format!(
                        "Contains: {}{suffix} folders, {}{suffix} files",
                        stats.map(|s| s.dir_count).unwrap_or(0),
                        stats.map(|s| s.file_count).unwrap_or(0),
                    ))
                    .size(9.5)
                    .color(colors.text_primary),
                );
                ui.label(
                    egui::RichText::new(format!(
                        "Immediate file size: {}",
                        format_size(stats.map(|s| s.total_bytes))
                    ))
                    .size(9.5)
                    .color(colors.text_primary),
                );
            });
    }

    fn draw_text_sidebar(&mut self, ui: &mut egui::Ui, colors: &Colors, entry: &Entry) {
        let path = entry.path.clone();
        self.ensure_text_preview(&path);

        egui::Frame::new()
            .fill(colors.bg_sidebar)
            .stroke(Stroke::new(1.0, colors.border))
            .corner_radius(CornerRadius::same(6))
            .inner_margin(egui::Margin::same(8))
            .show(ui, |ui| {
                ui.label(
                    egui::RichText::new(format!("Text \u{00b7} {}", entry.name))
                        .size(10.5)
                        .color(colors.text_primary)
                        .strong(),
                );
                if let Some(size) = entry.size_bytes {
                    ui.label(
                        egui::RichText::new(format_size(Some(size)))
                            .size(9.5)
                            .color(colors.text_dim),
                    );
                }
                ui.separator();
                self.draw_text_preview_body(ui, colors, 150.0, 10);
            });
    }

    fn draw_generic_sidebar(&mut self, ui: &mut egui::Ui, colors: &Colors, entry: &Entry) {
        egui::Frame::new()
            .fill(colors.bg_sidebar)
            .stroke(Stroke::new(1.0, colors.border))
            .corner_radius(CornerRadius::same(6))
            .inner_margin(egui::Margin::same(8))
            .show(ui, |ui| {
                ui.label(
                    egui::RichText::new(entry.name.clone())
                        .size(10.5)
                        .color(colors.text_primary)
                        .strong(),
                );
                ui.separator();
                if let Some(size) = entry.size_bytes {
                    ui.label(
                        egui::RichText::new(format_size(Some(size)))
                            .size(9.5)
                            .color(colors.text_dim),
                    );
                }
                if let Some(modified) = entry.modified {
                    ui.label(
                        egui::RichText::new(format!(
                            "Modified: {}",
                            format_modified(Some(modified))
                        ))
                        .size(9.5)
                        .color(colors.text_dim),
                    );
                }
            });
    }

    fn draw_text_preview_body(
        &self,
        ui: &mut egui::Ui,
        colors: &Colors,
        max_height: f32,
        max_lines: usize,
    ) {
        if let Some(preview) = &self.text_preview {
            let line_count = preview.body.lines().count();
            let mut text = preview
                .body
                .lines()
                .take(max_lines)
                .collect::<Vec<_>>()
                .join("\n");
            if preview.truncated || line_count > max_lines {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str("\u{2026}");
            }
            egui::ScrollArea::vertical()
                .max_height(max_height)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(text)
                                .monospace()
                                .size(style::TEXT_HINT)
                                .color(colors.text_primary),
                        )
                        .wrap(),
                    );
                });
        } else if let Some(err) = &self.text_preview_error {
            ui.label(
                egui::RichText::new(err)
                    .size(style::TEXT_HINT)
                    .color(colors.text_dim),
            );
        } else {
            ui.label(
                egui::RichText::new("Loading\u{2026}")
                    .size(style::TEXT_HINT)
                    .color(colors.text_dim),
            );
        }
    }

    fn draw_quick_look_modal(&mut self, ctx: &egui::Context, colors: &Colors) {
        if !self.quick_look_open {
            return;
        }
        let entry = match self.selected_entry().cloned() {
            Some(entry) => entry,
            None => {
                self.quick_look_open = false;
                return;
            }
        };
        let title = Self::entry_title(&entry);
        let response = ModalShell::centered("file_browser_quick_look")
            .title(&title)
            .width(style::MODAL_WIDTH_NOTIFY)
            .escape(true)
            .show(ctx, colors, |ui| {
                if entry.is_image {
                    self.draw_quick_look_image(ui, colors, &entry);
                } else if entry.is_dir {
                    self.draw_quick_look_folder(ui, colors, &entry);
                } else if is_text_preview_candidate(&entry.path) {
                    self.draw_quick_look_text(ui, colors, &entry);
                } else {
                    self.draw_quick_look_generic(ui, colors, &entry);
                }
                ui.add_space(style::SPACE_MD);
                let hints = [
                    HintGroup::new(&["Space"], "dismiss"),
                    HintGroup::new(&["Esc"], "dismiss"),
                    HintGroup::new(&["Enter"], "open selected item"),
                ];
                HintBar::new(&hints).show(ui, colors);
            });
        if response.dismissed {
            self.dismiss_quick_look();
        }
    }

    fn draw_pending_operation_modal(&mut self, ctx: &egui::Context, colors: &Colors) {
        let Some(operation) = self.pending_operation.clone() else {
            return;
        };
        let PendingFileOperation::MoveToTrash { paths } = operation;
        let count = paths.len();
        let response = ModalShell::centered("file_browser_confirm_file_operation")
            .title("Move to Trash")
            .escape(true)
            .show(ctx, colors, |ui| {
                ui.label(
                    egui::RichText::new(format!(
                        "Move {count} selected item{} to this folder's .Trash?",
                        if count == 1 { "" } else { "s" }
                    ))
                    .size(style::TEXT_BODY)
                    .color(colors.text_primary),
                );
                ui.add_space(style::SPACE_MD);
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        self.cancel_pending_operation();
                    }
                    if ui.button("Move to Trash").clicked() {
                        if let Err(err) = self.confirm_pending_operation() {
                            self.error = Some(err);
                        }
                    }
                });
                ui.add_space(style::SPACE_MD);
                let hints = [
                    HintGroup::new(&["Enter"], "confirm"),
                    HintGroup::new(&["Esc"], "cancel"),
                ];
                HintBar::new(&hints).show(ui, colors);
            });
        if response.dismissed {
            self.cancel_pending_operation();
        }
    }

    fn draw_rename_modal(&mut self, ctx: &egui::Context, colors: &Colors) {
        if self.rename_path.is_none() {
            return;
        }
        let response = ModalShell::centered("file_browser_rename")
            .title("Rename")
            .escape(true)
            .show(ctx, colors, |ui| {
                let text_response = ui
                    .scope(|ui| {
                        // egui's caret is hidden (transparent, non-blinking);
                        // draw_text_caret paints a glyph-height replacement on top.
                        ui.visuals_mut().text_cursor.blink = false;
                        ui.visuals_mut().text_cursor.stroke.color = egui::Color32::TRANSPARENT;
                        let font_id = egui::TextStyle::Body.resolve(ui.style());
                        let row_height = ui.fonts(|f| f.row_height(&font_id));
                        let output = egui::TextEdit::singleline(&mut self.rename_buffer)
                            .desired_width(f32::INFINITY)
                            .show(ui);
                        crate::ui::text_field::draw_text_caret(
                            ui,
                            &output,
                            font_id.size,
                            row_height,
                            egui::Stroke::new(1.5, colors.accent),
                        );
                        output.response
                    })
                    .inner;
                text_response.request_focus();
                ui.add_space(style::SPACE_MD);
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        self.cancel_rename_modal();
                    }
                    if ui.button("Rename").clicked() {
                        self.confirm_rename_modal();
                    }
                });
                ui.add_space(style::SPACE_MD);
                let hints = [
                    HintGroup::new(&["Enter"], "rename"),
                    HintGroup::new(&["Esc"], "cancel"),
                ];
                HintBar::new(&hints).show(ui, colors);
            });
        if response.dismissed {
            self.cancel_rename_modal();
        }
    }

    fn draw_quick_look_image(&mut self, ui: &mut egui::Ui, colors: &Colors, entry: &Entry) {
        self.ensure_image_preview(ui.ctx(), &entry.path);
        if let Some([w, h]) = self.preview_size {
            ui.label(
                egui::RichText::new(format!("Image \u{00b7} {w}\u{00d7}{h}"))
                    .size(style::TEXT_HINT)
                    .color(colors.text_dim),
            );
        }
        ui.add_space(style::SPACE_SM);
        let slot_size = egui::vec2(ui.available_width(), 420.0);
        let (slot_rect, _) = ui.allocate_exact_size(slot_size, egui::Sense::hover());
        ui.painter()
            .rect_filled(slot_rect, style::RADIUS_MD, colors.bg_darkest);
        ui.painter().rect_stroke(
            slot_rect,
            style::RADIUS_MD,
            Stroke::new(1.0, colors.border),
            StrokeKind::Inside,
        );
        if let Some(texture) = &self.preview_texture {
            let tex_size = egui::vec2(texture.size()[0] as f32, texture.size()[1] as f32);
            let scale = (slot_rect.width() / tex_size.x)
                .min(slot_rect.height() / tex_size.y)
                .min(1.0);
            let draw_size = tex_size * scale.max(0.01);
            let image_rect = egui::Rect::from_center_size(slot_rect.center(), draw_size);
            ui.painter().image(
                texture.id(),
                image_rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        } else if let Some(err) = &self.preview_error {
            ui.painter().text(
                slot_rect.center(),
                egui::Align2::CENTER_CENTER,
                err,
                egui::FontId::proportional(style::TEXT_HINT),
                colors.text_dim,
            );
        }
    }

    fn draw_quick_look_folder(&mut self, ui: &mut egui::Ui, colors: &Colors, entry: &Entry) {
        self.ensure_dir_preview(&entry.path);
        ui.label(
            egui::RichText::new(entry.path.display().to_string())
                .size(style::TEXT_HINT)
                .monospace()
                .color(colors.text_dim),
        );
        ui.add_space(style::SPACE_SM);
        if let Some(stats) = self.dir_preview_stats {
            let suffix = if stats.truncated { "+" } else { "" };
            ui.label(
                egui::RichText::new(format!(
                    "{}{suffix} folders, {}{suffix} files",
                    stats.dir_count, stats.file_count
                ))
                .size(style::TEXT_BODY)
                .color(colors.text_primary),
            );
            ui.label(
                egui::RichText::new(format!(
                    "Immediate file size: {}",
                    format_size(Some(stats.total_bytes))
                ))
                .size(style::TEXT_HINT)
                .color(colors.text_dim),
            );
        }
    }

    fn draw_quick_look_text(&mut self, ui: &mut egui::Ui, colors: &Colors, entry: &Entry) {
        self.ensure_text_preview(&entry.path);
        ui.label(
            egui::RichText::new(entry.path.display().to_string())
                .size(style::TEXT_HINT)
                .monospace()
                .color(colors.text_dim),
        );
        ui.add_space(style::SPACE_SM);
        self.draw_text_preview_body(ui, colors, 420.0, QUICK_LOOK_TEXT_LINES);
    }

    fn draw_quick_look_generic(&mut self, ui: &mut egui::Ui, colors: &Colors, entry: &Entry) {
        ui.label(
            egui::RichText::new(entry.path.display().to_string())
                .size(style::TEXT_HINT)
                .monospace()
                .color(colors.text_dim),
        );
        ui.add_space(style::SPACE_SM);
        if let Some(size) = entry.size_bytes {
            ui.label(
                egui::RichText::new(format!("Size: {}", format_size(Some(size))))
                    .size(style::TEXT_BODY)
                    .color(colors.text_primary),
            );
        }
        ui.label(
            egui::RichText::new(format!("Kind: {}", Self::entry_kind(entry)))
                .size(style::TEXT_BODY)
                .color(colors.text_primary),
        );
        if let Some(modified) = entry.modified {
            ui.label(
                egui::RichText::new(format!("Modified: {}", format_modified(Some(modified))))
                    .size(style::TEXT_HINT)
                    .color(colors.text_dim),
            );
        }
        if MediaKind::for_path(&entry.path).player_app_id().is_some() {
            ui.add_space(style::SPACE_SM);
            ui.label(
                egui::RichText::new("Press Enter to open with the existing Plexi media player.")
                    .size(style::TEXT_HINT)
                    .color(colors.text_dim),
            );
        }
    }

    fn draw_toolbar(&mut self, ui: &mut egui::Ui, colors: &Colors, layout: FileBrowserLayout) {
        // Path gets its own full-width row, elided from the left so the
        // leaf directory always stays visible. The old single-row layout
        // (path left, chips right) collided at narrow widths: the chips
        // wrapped under the path and the toolbar grew unpredictably.
        ui.horizontal(|ui| {
            let elided = elide_path_leading(
                ui,
                &self.cwd.display().to_string(),
                egui::FontId::monospace(style::TEXT_HINT),
                ui.available_width(),
            );
            ui.colored_label(
                colors.accent,
                egui::RichText::new(elided).size(style::TEXT_HINT).monospace(),
            );
        });
        ui.add_space(style::SPACE_XS);
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(match layout {
                    FileBrowserLayout::CompactList => "compact",
                    FileBrowserLayout::DetailsTable => "details",
                })
                .size(style::TEXT_HINT)
                .color(colors.text_dim),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // A single menu button instead of a strip of toggle chips.
                // The chip strip needed ~750px and, being right-to-left,
                // overflowed past the pane's LEFT edge when it didn't fit —
                // egui then extended every sibling row's left boundary, which
                // shoved the whole table off-screen at middle widths. A menu
                // is one fixed-width control: it fits at every pane size.
                ui.menu_button(
                    egui::RichText::new("View \u{2304}").size(style::TEXT_HINT),
                    |ui| {
                        let mut folders_on_top = self.columns.folders_on_top;
                        if ui.checkbox(&mut folders_on_top, "Folders first").changed() {
                            self.columns.folders_on_top = folders_on_top;
                            self.refresh_preserving_filter();
                            log::info!(
                                "file_browser: folders_on_top changed to {}",
                                self.columns.folders_on_top
                            );
                        }
                        let mut inspector = self.inspector_open;
                        if ui.checkbox(&mut inspector, "Inspector").changed() {
                            self.toggle_inspector();
                        }
                        // Column toggles only exist in the details table — in
                        // the compact list they would be seven dead entries.
                        if layout.shows_details() {
                            ui.separator();
                            for id in [
                                ColumnId::Kind,
                                ColumnId::Size,
                                ColumnId::Modified,
                                ColumnId::Created,
                                ColumnId::Extension,
                                ColumnId::Permissions,
                                ColumnId::Tags,
                            ] {
                                let mut visible = self
                                    .columns
                                    .columns
                                    .iter()
                                    .find(|column| column.id == id)
                                    .map(|column| column.visible)
                                    .unwrap_or(false);
                                if ui.checkbox(&mut visible, id.label()).changed() {
                                    self.columns.set_column_visible(id, visible);
                                    log::info!(
                                        "file_browser: column {} visibility changed to {}",
                                        id.key(),
                                        visible
                                    );
                                }
                            }
                        }
                    },
                );
            });
        });
    }

    fn draw_search_bar(&self, ui: &mut egui::Ui, colors: &Colors) {
        ui.horizontal(|ui| {
            ui.colored_label(colors.accent, "/");
            ui.colored_label(
                colors.text_primary,
                if self.search_query.is_empty() {
                    "type to filter\u{2026}"
                } else {
                    &self.search_query
                },
            );
            let count = self.search_indices.len();
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.colored_label(
                    colors.text_dim,
                    format!("{count} match{}", if count == 1 { "" } else { "es" }),
                );
            });
        });
    }

    fn draw_status_bar(&self, ui: &mut egui::Ui, colors: &Colors, show_inspector: bool) {
        ui.add_space(style::SPACE_XS);
        ui.separator();
        // Single non-wrapping row. The old `horizontal_wrapped` let the hint
        // bar wrap to a second line at narrow widths, overflowing the
        // STATUS_BAR_H reservation and colliding with the list rows above it.
        ui.horizontal(|ui| {
            ui.set_min_height(STATUS_BAR_ROW_H);
            let selected_label = self.selected_count().to_string();
            ui.label(
                egui::RichText::new(format!(
                    "{} items \u{00b7} selected: {}",
                    self.visible_entry_count(),
                    selected_label
                ))
                .size(style::TEXT_HINT)
                .color(colors.text_dim),
            );
            if show_inspector {
                ui.label(
                    egui::RichText::new("\u{00b7} inspector visible")
                        .size(style::TEXT_HINT)
                        .color(colors.text_dim),
                );
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // Hints are measured, not breakpointed: drop the least
                // important groups (front of the list) until the rest fit in
                // the width left over after the item-count label. Guessed
                // width tiers kept landing wrong, and a right-to-left layout
                // that overflows extends past the pane's LEFT edge — egui
                // then shifts every sibling row off-screen with it.
                let rem = ui.available_width();
                let all = [
                    HintGroup::new(&["/"], "search"),
                    HintGroup::new(&["s"], "sort"),
                    HintGroup::new(&["i"], "inspector"),
                    HintGroup::new(&["Space"], "quick look"),
                    HintGroup::new(&["Enter"], "open"),
                    HintGroup::new(&["t"], "cd terminal"),
                    HintGroup::new(&["Esc"], "close"),
                ];
                let fits = |groups: &[HintGroup]| {
                    let total = groups.iter().map(|g| g.width(ui)).sum::<f32>()
                        + style::SPACE_MD * groups.len().saturating_sub(1) as f32;
                    total + style::SPACE_XL <= rem
                };
                let mut start = 0;
                while start < all.len() - 1 && !fits(&all[start..]) {
                    start += 1;
                }
                HintBar::new(&all[start..]).show(ui, colors);
            });
        });
    }
}

impl App for FileBrowserApp {
    fn type_id(&self) -> &'static str {
        "file_browser"
    }

    fn display_name(&self) -> String {
        self.cwd
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "/".to_string())
    }

    fn ui(&mut self, ui: &mut egui::Ui, ctx: &AppRenderContext<'_>) {
        let colors = ctx.colors;

        egui::Frame::new()
            .fill(colors.terminal_bg)
            .inner_margin(egui::Margin::symmetric(12, 8))
            .show(ui, |ui| {
                let layout = FileBrowserLayout::for_width(ui.available_width());
                let show_inspector = self.should_show_inspector(ui.available_width());
                self.draw_toolbar(ui, colors, layout);

                if self.in_search {
                    self.draw_search_bar(ui, colors);
                }

                ui.add_space(style::SPACE_XS);

                if let Some(err) = &self.error.clone() {
                    ui.colored_label(colors.text_dim, err);
                    return;
                }

                if self.entries.is_empty() {
                    ui.colored_label(colors.text_dim, "Empty directory");
                    return;
                }

                let mut navigate_to: Option<(PathBuf, bool)> = None;
                let body_height = (ui.available_height() - STATUS_BAR_H).max(0.0);

                if show_inspector {
                    let total_width = ui.available_width();
                    let max_inspector_width =
                        (total_width - DETAILS_TABLE_MIN_WIDTH - INSPECTOR_SPLITTER_W)
                            .min(total_width * 0.45)
                            .max(INSPECTOR_MIN_PANEL_WIDTH);
                    self.inspector_width = self
                        .inspector_width
                        .clamp(INSPECTOR_MIN_PANEL_WIDTH, max_inspector_width);
                    let list_width = total_width - self.inspector_width - INSPECTOR_SPLITTER_W;

                    ui.horizontal(|ui| {
                        ui.set_height(body_height);
                        ui.allocate_ui_with_layout(
                            egui::vec2(list_width, body_height),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                egui::ScrollArea::vertical()
                                    // animated(false): required by scroll_row_into_view —
                                    // see src/ui/list.rs.
                                    .animated(false)
                                    .id_salt("fb_list")
                                    .auto_shrink([false, false])
                                    .max_height(body_height)
                                    .show(ui, |ui| {
                                        navigate_to = self.draw_details_table(ui, colors);
                                    });
                            },
                        );
                        let (splitter_rect, splitter_response) = ui.allocate_exact_size(
                            egui::vec2(INSPECTOR_SPLITTER_W, body_height),
                            egui::Sense::drag(),
                        );
                        if splitter_response.dragged() {
                            let delta = ui.input(|input| input.pointer.delta().x);
                            self.inspector_width = (self.inspector_width - delta)
                                .clamp(INSPECTOR_MIN_PANEL_WIDTH, max_inspector_width);
                        }
                        if splitter_response.drag_stopped() {
                            log::info!(
                                "file_browser: inspector resized to {:.1}",
                                self.inspector_width
                            );
                        }
                        ui.painter().line_segment(
                            [
                                egui::pos2(splitter_rect.center().x, splitter_rect.top()),
                                egui::pos2(splitter_rect.center().x, splitter_rect.bottom()),
                            ],
                            Stroke::new(1.0, colors.border),
                        );
                        ui.allocate_ui_with_layout(
                            egui::vec2(self.inspector_width, body_height),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                self.draw_sidebar_preview(ui, colors);
                            },
                        );
                    });
                } else {
                    {
                        egui::ScrollArea::vertical()
                            // animated(false): required by scroll_row_into_view —
                            // see src/ui/list.rs.
                            .animated(false)
                            .id_salt("fb_list")
                            .auto_shrink([false, false])
                            .max_height(body_height)
                            .show(ui, |ui| {
                                navigate_to = if layout.shows_details() {
                                    self.draw_details_table(ui, colors)
                                } else {
                                    self.draw_compact_list(ui, colors)
                                };
                            });
                    }
                }

                self.draw_status_bar(ui, colors, show_inspector);

                if let Some((path, is_dir)) = navigate_to {
                    if is_dir {
                        self.navigate_into(path);
                    } else {
                        self.open_file(&path);
                    }
                }
            });

        self.draw_quick_look_modal(ui.ctx(), colors);
        self.draw_pending_operation_modal(ui.ctx(), colors);
        self.draw_rename_modal(ui.ctx(), colors);
    }

    fn handle_key(&mut self, input: &egui::InputState) -> crate::app::app_trait::KeyDisposition {
        use crate::app::app_trait::KeyDisposition;
        let mode = if self.in_search {
            Mode::Search
        } else if self.entries.is_empty() {
            Mode::Empty
        } else {
            Mode::Normal
        };
        let action = classify_key(input, self.in_search);
        log::debug!("file_browser: handle_key mode={mode:?} action={action:?}");

        if self.pending_operation.is_some() {
            match action {
                Some(FileBrowserAction::Escape) => {
                    self.cancel_pending_operation();
                    return KeyDisposition::Consumed;
                }
                Some(FileBrowserAction::Activate) => {
                    if let Err(err) = self.confirm_pending_operation() {
                        self.error = Some(err);
                    }
                    return KeyDisposition::Consumed;
                }
                _ => return KeyDisposition::Consumed,
            }
        }

        if self.rename_path.is_some() {
            match action {
                Some(FileBrowserAction::Escape) => {
                    self.cancel_rename_modal();
                    return KeyDisposition::Consumed;
                }
                Some(FileBrowserAction::Activate) => {
                    self.confirm_rename_modal();
                    return KeyDisposition::Consumed;
                }
                _ => return KeyDisposition::Consumed,
            }
        }

        if self.quick_look_open {
            match action {
                Some(FileBrowserAction::Escape) | Some(FileBrowserAction::ToggleQuickLook) => {
                    self.dismiss_quick_look();
                    return KeyDisposition::Consumed;
                }
                Some(FileBrowserAction::Activate) => {
                    if let Some(entry) = self.selected_entry().cloned() {
                        self.dismiss_quick_look();
                        if entry.is_dir {
                            self.navigate_into(entry.path);
                        } else {
                            self.open_file(&entry.path);
                        }
                    }
                    return KeyDisposition::Consumed;
                }
                _ => return KeyDisposition::Consumed,
            }
        }

        match (mode, action) {
            // Escape: closes browser in normal/empty, exits search in search mode
            (Mode::Search, Some(FileBrowserAction::Escape)) => {
                self.exit_search();
                KeyDisposition::Consumed
            }
            (_, Some(FileBrowserAction::Escape)) => {
                self.should_close = true;
                KeyDisposition::Consumed
            }
            // Works in Normal and Empty mode alike — an empty directory is
            // still a valid handoff target. Search mode never produces this
            // action (classify_key gates on !in_search).
            (_, Some(FileBrowserAction::CdTerminalAndClose)) => {
                self.cd_terminal_and_close();
                KeyDisposition::Consumed
            }
            // NavigateUp (← / H): global — no modal conflict
            (_, Some(FileBrowserAction::NavigateUp)) => {
                self.navigate_up();
                KeyDisposition::Consumed
            }
            // Backspace: delete char in search, navigate up elsewhere
            (Mode::Search, Some(FileBrowserAction::Backspace)) => {
                self.search_query.pop();
                self.refilter();
                KeyDisposition::Consumed
            }
            (_, Some(FileBrowserAction::Backspace)) => {
                self.navigate_up();
                KeyDisposition::Consumed
            }
            // Search mode actions
            (Mode::Search, Some(FileBrowserAction::Activate)) => {
                if let Some(&entry_idx) = self.search_indices.get(self.selected) {
                    let entry = self.entries[entry_idx].clone();
                    if entry.is_dir {
                        self.exit_search();
                        self.navigate_into(entry.path);
                    } else {
                        self.open_file(&entry.path);
                        self.exit_search();
                    }
                }
                KeyDisposition::Consumed
            }
            (Mode::Search, Some(FileBrowserAction::SelectNext)) => {
                let last = self.search_indices.len().saturating_sub(1);
                self.selected = (self.selected + 1).min(last);
                self.pending_scroll = true;
                KeyDisposition::Consumed
            }
            (Mode::Search, Some(FileBrowserAction::SelectPrev)) => {
                self.selected = self.selected.saturating_sub(1);
                self.pending_scroll = true;
                KeyDisposition::Consumed
            }
            (Mode::Search, Some(FileBrowserAction::AppendText(text))) => {
                self.search_query.push_str(&text);
                self.refilter();
                KeyDisposition::Consumed
            }
            // Search mode consumes all unhandled input
            (Mode::Search, _) => KeyDisposition::Consumed,
            // Empty mode: no further keys handled
            (Mode::Empty, _) => KeyDisposition::Passthrough,
            // Normal mode actions
            (Mode::Normal, Some(FileBrowserAction::EnterSearch)) => {
                self.in_search = true;
                self.search_query.clear();
                self.refilter();
                log::info!("file_browser: entering search mode");
                KeyDisposition::Consumed
            }
            (Mode::Normal, Some(FileBrowserAction::SelectNext)) => {
                let last = self.entries.len().saturating_sub(1);
                let next = (self.selected + 1).min(last);
                if input.modifiers.shift {
                    self.extend_selection_to(next);
                } else {
                    self.set_single_selection(next);
                }
                self.pending_scroll = true;
                KeyDisposition::Consumed
            }
            (Mode::Normal, Some(FileBrowserAction::SelectPrev)) => {
                let prev = self.selected.saturating_sub(1);
                if input.modifiers.shift {
                    self.extend_selection_to(prev);
                } else {
                    self.set_single_selection(prev);
                }
                self.pending_scroll = true;
                KeyDisposition::Consumed
            }
            (Mode::Normal, Some(FileBrowserAction::SelectFirst)) => {
                self.set_single_selection(0);
                self.pending_scroll = true;
                KeyDisposition::Consumed
            }
            (Mode::Normal, Some(FileBrowserAction::SelectLast)) => {
                self.set_single_selection(self.entries.len().saturating_sub(1));
                self.pending_scroll = true;
                KeyDisposition::Consumed
            }
            (Mode::Normal, Some(FileBrowserAction::PageDown)) => {
                let last = self.entries.len().saturating_sub(1);
                self.set_single_selection((self.selected + 10).min(last));
                self.pending_scroll = true;
                KeyDisposition::Consumed
            }
            (Mode::Normal, Some(FileBrowserAction::PageUp)) => {
                self.set_single_selection(self.selected.saturating_sub(10));
                self.pending_scroll = true;
                KeyDisposition::Consumed
            }
            (Mode::Normal, Some(FileBrowserAction::Activate)) => {
                if let Some(entry) = self.selected_entry().cloned() {
                    if entry.is_dir {
                        self.navigate_into(entry.path);
                    } else {
                        self.open_file(&entry.path);
                    }
                }
                KeyDisposition::Consumed
            }
            (Mode::Normal, Some(FileBrowserAction::ToggleSort)) => {
                self.columns.toggle_legacy_sort();
                self.refresh();
                log::info!(
                    "file_browser: sort changed to {} {}",
                    self.columns.sort.column.key(),
                    self.columns.sort.direction.key()
                );
                KeyDisposition::Consumed
            }
            (Mode::Normal, Some(FileBrowserAction::ToggleInspector)) => {
                self.toggle_inspector();
                KeyDisposition::Consumed
            }
            (Mode::Normal, Some(FileBrowserAction::ToggleQuickLook)) => {
                self.toggle_quick_look();
                KeyDisposition::Consumed
            }
            (Mode::Normal, Some(FileBrowserAction::Refresh)) => {
                self.refresh();
                KeyDisposition::Consumed
            }
            (Mode::Normal, Some(FileBrowserAction::SelectAll)) => {
                self.select_all_visible();
                KeyDisposition::Consumed
            }
            (Mode::Normal, Some(FileBrowserAction::NewFolder)) => {
                if let Err(err) = self.create_new_folder() {
                    self.error = Some(err);
                }
                KeyDisposition::Consumed
            }
            (Mode::Normal, Some(FileBrowserAction::Rename)) => {
                self.open_rename_modal();
                KeyDisposition::Consumed
            }
            (Mode::Normal, Some(FileBrowserAction::Copy)) => {
                self.copy_selected(FileOperation::Copy);
                KeyDisposition::Consumed
            }
            (Mode::Normal, Some(FileBrowserAction::Cut)) => {
                self.copy_selected(FileOperation::Cut);
                KeyDisposition::Consumed
            }
            (Mode::Normal, Some(FileBrowserAction::Paste)) => {
                if let Err(err) = self.paste_into_current_dir() {
                    self.error = Some(err);
                }
                KeyDisposition::Consumed
            }
            (Mode::Normal, Some(FileBrowserAction::Duplicate)) => {
                if let Err(err) = self.duplicate_selected() {
                    self.error = Some(err);
                }
                KeyDisposition::Consumed
            }
            (Mode::Normal, Some(FileBrowserAction::MoveToTrash)) => {
                self.request_move_selected_to_trash();
                KeyDisposition::Consumed
            }
            (Mode::Normal, Some(FileBrowserAction::Reveal)) => {
                self.reveal_selected();
                KeyDisposition::Consumed
            }
            (Mode::Normal, Some(FileBrowserAction::OpenWithDefault)) => {
                self.open_selected_with_default();
                KeyDisposition::Consumed
            }
            _ => KeyDisposition::Passthrough,
        }
    }

    fn take_pending_commands(&mut self) -> Vec<AppCommand> {
        std::mem::take(&mut self.pending_cmds)
    }

    fn keyboard_capture(&self) -> bool {
        self.quick_look_open || self.pending_operation.is_some() || self.rename_path.is_some()
    }

    fn wants_close(&self) -> bool {
        self.should_close
    }

    fn sync_cwd(&mut self, new_cwd: &std::path::Path) {
        self.sync_cwd(new_cwd.to_path_buf());
    }

    fn serialize_state(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "cwd": self.cwd.display().to_string(),
            "selected": self.selected,
            "columns": self.columns.to_json(),
        }))
    }

    fn restore_state(&mut self, state: &serde_json::Value) {
        if let Some(cwd) = state["cwd"].as_str() {
            let path = PathBuf::from(cwd);
            if path.is_dir() {
                self.cwd = path;
                self.refresh();
            }
        }
        if let Some(sel) = state["selected"].as_u64() {
            self.selected = (sel as usize).min(self.entries.len().saturating_sub(1));
        }
        if state.get("columns").is_some() {
            self.columns = ColumnModel::from_json(&state["columns"]);
            self.refresh();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, Key, Modifiers, RawInput};
    use helpers::SortDescriptor;

    fn key_event(key: Key, modifiers: Modifiers) -> Event {
        Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    fn run_handle_key(app: &mut FileBrowserApp, events: Vec<Event>) -> bool {
        use crate::app::app_trait::KeyDisposition;
        let ctx = egui::Context::default();
        let modifiers = events
            .iter()
            .find_map(|event| match event {
                Event::Key { modifiers, .. } => Some(*modifiers),
                _ => None,
            })
            .unwrap_or_default();
        let raw = RawInput {
            events,
            modifiers,
            ..Default::default()
        };
        let mut consumed = false;
        let _ = ctx.run(raw, |ctx| {
            ctx.input(|i| {
                consumed = app.handle_key(i) == KeyDisposition::Consumed;
            });
        });
        consumed
    }

    fn make_empty_dir_app() -> (FileBrowserApp, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let app = FileBrowserApp::new(dir.path().to_path_buf());
        (app, dir)
    }

    fn make_populated_dir_app() -> (FileBrowserApp, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("file.txt"), b"hi").expect("write");
        let app = FileBrowserApp::new(dir.path().to_path_buf());
        (app, dir)
    }

    #[test]
    fn layout_policy_keeps_narrow_panes_compact() {
        assert_eq!(
            FileBrowserLayout::for_width(DETAILS_TABLE_MIN_WIDTH - 1.0),
            FileBrowserLayout::CompactList
        );
        assert_eq!(
            FileBrowserLayout::for_width(DETAILS_TABLE_MIN_WIDTH),
            FileBrowserLayout::DetailsTable
        );
        assert_eq!(
            FileBrowserLayout::for_width(INSPECTOR_MIN_WIDTH),
            FileBrowserLayout::DetailsTable
        );
    }

    #[test]
    fn layout_policy_exposes_details_and_inspector_separately() {
        assert!(!FileBrowserLayout::CompactList.shows_details());
        assert!(FileBrowserLayout::DetailsTable.shows_details());
    }

    #[test]
    fn inspector_is_explicit_not_width_triggered() {
        let (mut app, _dir) = make_populated_dir_app();
        assert!(!app.inspector_open);
        assert!(!app.should_show_inspector(INSPECTOR_MIN_WIDTH));

        app.toggle_inspector();

        assert!(app.inspector_open);
        assert!(app.should_show_inspector(INSPECTOR_MIN_WIDTH));
        assert!(!app.should_show_inspector(INSPECTOR_MIN_WIDTH - 1.0));
    }

    #[test]
    fn i_toggles_inspector_in_normal_mode() {
        let (mut app, _dir) = make_populated_dir_app();
        let consumed = run_handle_key(&mut app, vec![key_event(Key::I, Modifiers::default())]);
        assert!(consumed);
        assert!(app.inspector_open);

        run_handle_key(&mut app, vec![key_event(Key::I, Modifiers::default())]);
        assert!(!app.inspector_open);
    }

    #[test]
    fn browsing_never_queues_cd_request() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir(dir.path().join("sub")).expect("mkdir");
        let mut app = FileBrowserApp::new(dir.path().to_path_buf());
        app.take_pending_commands();

        app.navigate_into(dir.path().join("sub"));
        app.navigate_up();

        let cmds = app.take_pending_commands();
        assert!(
            !cmds
                .iter()
                .any(|c| matches!(c, AppCommand::CdRequest { .. })),
            "browsing must not write cd into the linked terminal"
        );
        assert!(!app.should_close);
    }

    #[test]
    fn t_hands_off_cwd_and_closes() {
        let (mut app, dir) = make_populated_dir_app();
        let consumed = run_handle_key(&mut app, vec![key_event(Key::T, Modifiers::default())]);

        assert!(consumed);
        assert!(app.should_close);
        let cd_cwd = app.take_pending_commands().into_iter().find_map(|c| match c {
            AppCommand::CdRequest { cwd, .. } => Some(cwd),
            _ => None,
        });
        assert_eq!(cd_cwd.as_deref(), Some(dir.path().to_string_lossy().as_ref()));
    }

    #[test]
    fn t_in_search_mode_is_not_a_handoff() {
        let (mut app, _dir) = make_populated_dir_app();
        run_handle_key(&mut app, vec![key_event(Key::Slash, Modifiers::default())]);
        assert!(app.in_search);

        let consumed = run_handle_key(&mut app, vec![key_event(Key::T, Modifiers::default())]);

        assert!(consumed);
        assert!(!app.should_close);
        assert!(!app
            .take_pending_commands()
            .iter()
            .any(|c| matches!(c, AppCommand::CdRequest { .. })));
    }

    #[test]
    fn elide_path_keeps_leaf_visible() {
        let ctx = egui::Context::default();
        let _ = ctx.run(RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let font = egui::FontId::monospace(11.0);
                let full = "/Users/ian/Documents/GitHub/PLEXI/src/file_browser";
                assert_eq!(elide_path_leading(ui, full, font.clone(), 10_000.0), full);

                let narrow = elide_path_leading(ui, full, font.clone(), 160.0);
                assert!(narrow.starts_with('\u{2026}'));
                assert!(narrow.ends_with("file_browser"));
                assert!(narrow.chars().count() < full.chars().count());

                assert!(elide_path_leading(ui, full, font, 0.0).is_empty());
            });
        });
    }

    #[test]
    fn space_opens_quick_look_without_opening_file() {
        let (mut app, _dir) = make_file_only_dir_app();
        let consumed = run_handle_key(&mut app, vec![key_event(Key::Space, Modifiers::default())]);

        assert!(consumed);
        assert!(app.quick_look_open);
        assert!(app.opened_files.is_empty());
    }

    #[test]
    fn escape_dismisses_quick_look_before_closing_browser() {
        let (mut app, _dir) = make_file_only_dir_app();
        run_handle_key(&mut app, vec![key_event(Key::Space, Modifiers::default())]);
        assert!(app.quick_look_open);

        let consumed = run_handle_key(&mut app, vec![key_event(Key::Escape, Modifiers::default())]);

        assert!(consumed);
        assert!(!app.quick_look_open);
        assert!(!app.should_close);
    }

    #[test]
    fn quick_look_captures_host_app_shortcuts() {
        let (mut app, _dir) = make_file_only_dir_app();
        assert!(!app.keyboard_capture());

        run_handle_key(&mut app, vec![key_event(Key::Space, Modifiers::default())]);

        assert!(app.quick_look_open);
        assert!(app.keyboard_capture());
    }

    // ── Regression tests for issue #926 ──────────────────────────────────────
    // The empty-dir guard previously swallowed Escape, H, and ← without acting.

    #[test]
    fn empty_dir_escape_closes() {
        let (mut app, _dir) = make_empty_dir_app();
        assert!(app.entries.is_empty());
        let consumed = run_handle_key(&mut app, vec![key_event(Key::Escape, Modifiers::default())]);
        assert!(consumed, "Escape must be consumed in empty dir");
        assert!(
            app.should_close,
            "Escape must set should_close in empty dir"
        );
    }

    #[test]
    fn empty_dir_arrow_left_navigates_up() {
        let (mut app, _dir) = make_empty_dir_app();
        let parent = app.cwd.parent().map(|p| p.to_path_buf()).unwrap();
        let consumed = run_handle_key(
            &mut app,
            vec![key_event(Key::ArrowLeft, Modifiers::default())],
        );
        assert!(consumed, "← must be consumed in empty dir");
        assert_eq!(app.cwd, parent, "← must navigate to parent in empty dir");
    }

    #[test]
    fn empty_dir_h_navigates_up() {
        let (mut app, _dir) = make_empty_dir_app();
        let parent = app.cwd.parent().map(|p| p.to_path_buf()).unwrap();
        let consumed = run_handle_key(&mut app, vec![key_event(Key::H, Modifiers::default())]);
        assert!(consumed, "H must be consumed in empty dir");
        assert_eq!(app.cwd, parent, "H must navigate to parent in empty dir");
    }

    #[test]
    fn empty_dir_backspace_navigates_up() {
        let (mut app, _dir) = make_empty_dir_app();
        let parent = app.cwd.parent().map(|p| p.to_path_buf()).unwrap();
        let consumed = run_handle_key(
            &mut app,
            vec![key_event(Key::Backspace, Modifiers::default())],
        );
        assert!(consumed);
        assert_eq!(app.cwd, parent);
    }

    #[test]
    fn empty_dir_other_keys_not_consumed() {
        let (mut app, _dir) = make_empty_dir_app();
        let consumed = run_handle_key(
            &mut app,
            vec![key_event(Key::ArrowDown, Modifiers::default())],
        );
        assert!(!consumed, "↓ must not be consumed in empty dir");
    }

    // ── Normal mode smoke tests ───────────────────────────────────────────────

    #[test]
    fn normal_mode_escape_closes() {
        let (mut app, _dir) = make_populated_dir_app();
        run_handle_key(&mut app, vec![key_event(Key::Escape, Modifiers::default())]);
        assert!(app.should_close);
    }

    #[test]
    fn normal_mode_slash_enters_search() {
        let (mut app, _dir) = make_populated_dir_app();
        run_handle_key(&mut app, vec![key_event(Key::Slash, Modifiers::default())]);
        assert!(app.in_search);
    }

    // ── Search mode smoke tests ───────────────────────────────────────────────

    #[test]
    fn search_escape_exits_search() {
        let (mut app, _dir) = make_populated_dir_app();
        app.in_search = true;
        app.search_query = "abc".to_string();
        run_handle_key(&mut app, vec![key_event(Key::Escape, Modifiers::default())]);
        assert!(!app.in_search);
        assert!(
            !app.should_close,
            "Escape in search must not close the browser"
        );
    }

    #[test]
    fn handle_key_has_no_key_pressed_calls() {
        // Structural invariant: all key_pressed calls live in classify_key, not handle_key.
        // This test fails at compile time if violated — it's a documentation check.
        // The real enforcement is: grep the source for key_pressed outside classify_key.
        let src = include_str!("mod.rs");
        let classify_start = src
            .find("fn classify_key")
            .expect("classify_key must exist");
        let handle_key_start = src.find("fn handle_key").expect("handle_key must exist");
        let handle_key_body = &src[handle_key_start..];
        let after_classify = &src[classify_start..handle_key_start];
        // classify_key should contain key_pressed calls (for scroll keys)
        assert!(
            after_classify.contains("key_pressed"),
            "classify_key must contain key_pressed calls"
        );
        // classify_key must also use key_pressed_no_repeat for action/activation keys (Escape, Enter, etc.)
        // scroll keys (ArrowDown, ArrowUp, j, k) still use key_pressed to allow repeating
        assert!(
            after_classify.contains("key_pressed_no_repeat"),
            "classify_key must use key_pressed_no_repeat for action/activation keys"
        );
        // handle_key body must not contain key_pressed calls
        let handle_body_end = handle_key_body[10..]
            .find("fn ")
            .map(|i| i + 10)
            .unwrap_or(handle_key_body.len());
        let handle_body = &handle_key_body[..handle_body_end];
        assert!(
            !handle_body.contains("key_pressed"),
            "handle_key must not contain key_pressed calls — they belong in classify_key"
        );
    }

    #[test]
    fn key_pressed_no_repeat_requires_non_repeat_event() {
        // key_pressed_no_repeat returns false when no events are present.
        let ctx = egui::Context::default();
        let _ = ctx.run(RawInput::default(), |ctx| {
            ctx.input(|i| {
                assert!(!super::key_pressed_no_repeat(i, Key::ArrowRight));
            });
        });
    }

    #[test]
    fn key_pressed_no_repeat_returns_true_for_fresh_press() {
        // egui marks the first press of a key (repeat=false), so key_pressed_no_repeat
        // must return true for a fresh key press event.
        let ctx = egui::Context::default();
        let raw = RawInput {
            events: vec![key_event(Key::ArrowRight, Modifiers::default())],
            ..Default::default()
        };
        let _ = ctx.run(raw, |ctx| {
            ctx.input(|i| {
                assert!(super::key_pressed_no_repeat(i, Key::ArrowRight));
            });
        });
    }

    // ── Enter / l / → open non-directory files (#138) ──────────────────────

    fn make_file_only_dir_app() -> (FileBrowserApp, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("readme.txt"), b"hi").expect("write file");
        let app = FileBrowserApp::new(dir.path().to_path_buf());
        (app, dir)
    }

    fn make_mixed_dir_app() -> (FileBrowserApp, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir(dir.path().join("subdir")).expect("create subdir");
        std::fs::write(dir.path().join("notes.txt"), b"hi").expect("write file");
        let app = FileBrowserApp::new(dir.path().to_path_buf());
        (app, dir)
    }

    fn make_three_file_app() -> (FileBrowserApp, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("alpha.txt"), b"a").expect("write alpha");
        std::fs::write(dir.path().join("bravo.txt"), b"b").expect("write bravo");
        std::fs::write(dir.path().join("charlie.txt"), b"c").expect("write charlie");
        let mut app = FileBrowserApp::new(dir.path().to_path_buf());
        app.columns.sort = SortDescriptor {
            column: ColumnId::Name,
            direction: SortDirection::Asc,
        };
        app.refresh();
        (app, dir)
    }

    #[test]
    fn shift_arrow_extends_selection_range() {
        let (mut app, _dir) = make_three_file_app();

        let consumed = run_handle_key(
            &mut app,
            vec![key_event(
                Key::ArrowDown,
                Modifiers {
                    shift: true,
                    ..Default::default()
                },
            )],
        );

        assert!(consumed);
        assert_eq!(app.selected_count(), 2);
        assert!(app
            .selected_paths()
            .iter()
            .any(|p| p.ends_with("alpha.txt")));
        assert!(app
            .selected_paths()
            .iter()
            .any(|p| p.ends_with("bravo.txt")));
    }

    #[test]
    fn command_a_selects_all_visible_entries() {
        let (mut app, _dir) = make_three_file_app();

        let consumed = run_handle_key(
            &mut app,
            vec![key_event(
                Key::A,
                Modifiers {
                    command: true,
                    ..Default::default()
                },
            )],
        );

        assert!(consumed);
        assert_eq!(app.selected_count(), 3);
    }

    #[test]
    fn duplicate_selected_file_creates_copy_and_refreshes_entries() {
        let (mut app, dir) = make_three_file_app();

        app.duplicate_selected().expect("duplicate");

        assert!(dir.path().join("alpha copy.txt").exists());
        assert!(app
            .entries
            .iter()
            .any(|entry| entry.name == "alpha copy.txt"));
    }

    #[test]
    fn rename_prompt_confirms_selected_file_rename() {
        let (mut app, dir) = make_three_file_app();

        app.open_rename_modal();
        app.rename_buffer = "renamed.txt".to_string();
        app.confirm_rename_modal();

        assert!(!dir.path().join("alpha.txt").exists());
        assert!(dir.path().join("renamed.txt").exists());
        assert!(app.rename_path.is_none());
        assert_eq!(
            app.selected_entry().map(|entry| entry.name.as_str()),
            Some("renamed.txt")
        );
    }

    #[test]
    fn copy_and_paste_selected_file_creates_unique_copy() {
        let (mut app, dir) = make_three_file_app();

        app.copy_selected(FileOperation::Copy);
        let created = app.paste_into_current_dir().expect("paste");

        assert_eq!(created.len(), 1);
        assert!(created[0].exists());
        assert_ne!(created[0], dir.path().join("alpha.txt"));
    }

    #[test]
    fn confirmed_trash_moves_selected_file_into_local_trash() {
        let (mut app, dir) = make_three_file_app();

        app.request_move_selected_to_trash();
        assert!(app.pending_operation.is_some());
        app.confirm_pending_operation().expect("confirm trash");

        assert!(!dir.path().join("alpha.txt").exists());
        assert!(dir.path().join(".Trash").join("alpha.txt").exists());
        assert!(app.pending_operation.is_none());
    }

    #[test]
    fn enter_on_file_opens_file_and_browser_stays_open() {
        let (mut app, _dir) = make_file_only_dir_app();
        assert!(
            !app.entries.is_empty(),
            "entries must be populated after new()"
        );
        assert!(
            !app.entries[0].is_dir,
            "first entry must be a file (no dirs in fixture)"
        );
        let consumed = run_handle_key(&mut app, vec![key_event(Key::Enter, Modifiers::default())]);
        assert!(consumed, "Enter on a file must be consumed");
        assert_eq!(
            app.opened_files.len(),
            1,
            "Enter on a file must call open_file"
        );
        assert!(
            app.opened_files[0].ends_with("readme.txt"),
            "must open the selected file"
        );
        assert!(
            !app.should_close,
            "browser must stay open after opening a file"
        );
    }

    #[test]
    fn l_key_on_file_opens_file() {
        let (mut app, _dir) = make_file_only_dir_app();
        let consumed = run_handle_key(&mut app, vec![key_event(Key::L, Modifiers::default())]);
        assert!(consumed);
        assert_eq!(app.opened_files.len(), 1, "l on a file must call open_file");
    }

    #[test]
    fn arrow_right_on_file_opens_file() {
        let (mut app, _dir) = make_file_only_dir_app();
        let consumed = run_handle_key(
            &mut app,
            vec![key_event(Key::ArrowRight, Modifiers::default())],
        );
        assert!(consumed);
        assert_eq!(app.opened_files.len(), 1, "→ on a file must call open_file");
    }

    #[test]
    fn enter_on_dir_navigates_not_opens() {
        let (mut app, _dir) = make_mixed_dir_app();
        // dirs sort first — selected=0 is the subdir
        assert!(
            app.entries[0].is_dir,
            "first entry must be the subdirectory"
        );
        let consumed = run_handle_key(&mut app, vec![key_event(Key::Enter, Modifiers::default())]);
        assert!(consumed);
        assert!(
            app.opened_files.is_empty(),
            "Enter on a dir must navigate, not open_file"
        );
    }

    #[test]
    fn search_enter_on_file_opens_and_exits_search() {
        let (mut app, _dir) = make_file_only_dir_app();
        // enter search mode
        run_handle_key(&mut app, vec![key_event(Key::Slash, Modifiers::default())]);
        assert!(app.in_search, "slash must enter search mode");
        // press Enter to open the (only) filtered result
        let consumed = run_handle_key(&mut app, vec![key_event(Key::Enter, Modifiers::default())]);
        assert!(consumed);
        assert_eq!(
            app.opened_files.len(),
            1,
            "Enter in search mode on a file must call open_file"
        );
        assert!(!app.in_search, "search mode must exit after opening a file");
    }

    // ── Search mode: letter keys must append to query, not fire vim actions ──
    // Regression tests for issue #258.

    #[test]
    fn search_mode_h_does_not_navigate_up() {
        let (mut app, _dir) = make_populated_dir_app();
        let original_cwd = app.cwd.clone();
        app.in_search = true;
        app.refilter();
        run_handle_key(&mut app, vec![key_event(Key::H, Modifiers::default())]);
        assert_eq!(
            app.cwd, original_cwd,
            "H in search mode must not navigate to parent"
        );
    }

    #[test]
    fn search_mode_h_appends_to_query() {
        let (mut app, _dir) = make_populated_dir_app();
        app.in_search = true;
        app.refilter();
        run_handle_key(
            &mut app,
            vec![
                key_event(Key::H, Modifiers::default()),
                egui::Event::Text("h".to_string()),
            ],
        );
        assert_eq!(
            app.search_query, "h",
            "H in search mode must append to query"
        );
    }

    #[test]
    fn search_mode_l_does_not_activate() {
        let (mut app, _dir) = make_file_only_dir_app();
        app.in_search = true;
        app.refilter();
        run_handle_key(&mut app, vec![key_event(Key::L, Modifiers::default())]);
        assert!(
            app.opened_files.is_empty(),
            "L in search mode must not open a file"
        );
    }

    #[test]
    fn search_mode_l_appends_to_query() {
        let (mut app, _dir) = make_populated_dir_app();
        app.in_search = true;
        app.refilter();
        run_handle_key(
            &mut app,
            vec![
                key_event(Key::L, Modifiers::default()),
                egui::Event::Text("l".to_string()),
            ],
        );
        assert_eq!(
            app.search_query, "l",
            "L in search mode must append to query"
        );
    }

    #[test]
    fn search_mode_j_appends_to_query() {
        let (mut app, _dir) = make_populated_dir_app();
        app.in_search = true;
        app.refilter();
        run_handle_key(
            &mut app,
            vec![
                key_event(Key::J, Modifiers::default()),
                egui::Event::Text("j".to_string()),
            ],
        );
        assert_eq!(
            app.search_query, "j",
            "J in search mode must append to query"
        );
    }

    #[test]
    fn search_mode_k_appends_to_query() {
        let (mut app, _dir) = make_populated_dir_app();
        app.in_search = true;
        app.refilter();
        run_handle_key(
            &mut app,
            vec![
                key_event(Key::K, Modifiers::default()),
                egui::Event::Text("k".to_string()),
            ],
        );
        assert_eq!(
            app.search_query, "k",
            "K in search mode must append to query"
        );
    }

    #[test]
    fn search_mode_s_does_not_toggle_sort() {
        let (mut app, _dir) = make_populated_dir_app();
        let original_sort = app.columns.sort;
        app.in_search = true;
        app.refilter();
        run_handle_key(
            &mut app,
            vec![
                key_event(Key::S, Modifiers::default()),
                egui::Event::Text("s".to_string()),
            ],
        );
        assert_eq!(
            app.columns.sort, original_sort,
            "S in search mode must not toggle sort"
        );
        assert_eq!(
            app.search_query, "s",
            "S in search mode must append to query"
        );
    }

    #[test]
    fn serialized_state_persists_column_preferences() {
        let (mut app, _dir) = make_populated_dir_app();
        app.columns.toggle_sort(ColumnId::Size);
        app.columns.resize_column(ColumnId::Name, 344.0);
        app.columns.set_column_visible(ColumnId::Created, true);
        app.columns.move_column(ColumnId::Created, -1);
        app.columns.folders_on_top = false;

        let state = app.serialize_state().expect("state");
        let mut restored = FileBrowserApp::new(app.cwd.clone());
        restored.restore_state(&state);

        assert_eq!(restored.columns.sort.column, ColumnId::Size);
        assert_eq!(restored.columns.sort.direction, SortDirection::Desc);
        assert!(!restored.columns.folders_on_top);
        assert_eq!(
            restored
                .columns
                .columns
                .iter()
                .find(|column| column.id == ColumnId::Name)
                .map(|column| column.width),
            Some(344.0)
        );
        assert_eq!(
            restored
                .columns
                .columns
                .iter()
                .find(|column| column.id == ColumnId::Created)
                .map(|column| column.visible),
            Some(true)
        );
        let created_index = restored
            .columns
            .columns
            .iter()
            .position(|column| column.id == ColumnId::Created)
            .expect("created column");
        let modified_index = restored
            .columns
            .columns
            .iter()
            .position(|column| column.id == ColumnId::Modified)
            .expect("modified column");
        assert!(created_index < modified_index);
    }

    #[test]
    fn refresh_preserving_filter_rebuilds_search_indices_after_sort_change() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("aaa.txt"), b"a").expect("write a");
        std::fs::write(dir.path().join("bbb.txt"), b"b").expect("write b");
        let mut app = FileBrowserApp::new(dir.path().to_path_buf());
        app.columns.sort = SortDescriptor {
            column: ColumnId::Name,
            direction: SortDirection::Asc,
        };
        app.refresh();
        app.in_search = true;
        app.search_query = "b".to_string();
        app.refilter();
        assert_eq!(
            app.selected_entry().map(|entry| entry.name.as_str()),
            Some("bbb.txt")
        );

        app.columns.sort = SortDescriptor {
            column: ColumnId::Name,
            direction: SortDirection::Desc,
        };
        app.refresh_preserving_filter();

        assert_eq!(
            app.selected_entry().map(|entry| entry.name.as_str()),
            Some("bbb.txt"),
            "search indices must follow the refreshed entry order"
        );
    }

    #[test]
    fn details_columns_keep_enabled_columns_reachable_at_breakpoint_width() {
        let (mut app, _dir) = make_populated_dir_app();
        for id in ColumnId::ALL {
            app.columns.set_column_visible(id, true);
        }
        let rect = egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(DETAILS_TABLE_MIN_WIDTH, DETAILS_HEADER_H),
        );
        let columns = app.details_columns(rect);

        assert_eq!(columns.len(), ColumnId::ALL.len());
        for (column, cell) in columns {
            assert!(
                cell.width() > 0.0,
                "{} column should remain reachable",
                column.id.key()
            );
        }
    }

    #[test]
    fn search_mode_r_appends_to_query() {
        let (mut app, _dir) = make_populated_dir_app();
        app.in_search = true;
        app.refilter();
        run_handle_key(
            &mut app,
            vec![
                key_event(Key::R, Modifiers::default()),
                egui::Event::Text("r".to_string()),
            ],
        );
        assert_eq!(
            app.search_query, "r",
            "R in search mode must append to query"
        );
    }

    #[test]
    fn arrow_down_still_selects_next_in_search() {
        let (mut app, _dir) = make_populated_dir_app();
        // add a second file so there's something to move to
        std::fs::write(app.cwd.join("other.txt"), b"x").expect("write");
        app.refresh();
        app.in_search = true;
        app.refilter();
        assert!(
            app.search_indices.len() >= 2,
            "need at least 2 entries for this test"
        );
        let before = app.selected;
        run_handle_key(
            &mut app,
            vec![key_event(Key::ArrowDown, Modifiers::default())],
        );
        assert_eq!(
            app.selected,
            before + 1,
            "↓ must still move selection in search mode"
        );
        assert_eq!(app.search_query, "", "↓ must not append to query");
    }
}

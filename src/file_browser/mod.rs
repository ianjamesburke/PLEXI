mod helpers;
mod icons;
pub mod intent;

/// Re-exported so the host open resolver (`app::canvas_bindings`) can use the
/// same media classification the File Explorer does (#2283).
pub(crate) use helpers::MediaKind;
pub use helpers::{ColumnId, Entry, SortDirection};
pub use intent::{
    CandidateResolution, FileExplorerContext, FileExplorerIntent, PaletteCandidate,
    parse_file_explorer_intent, rank_file_explorer_candidates,
};

use crate::app::app_trait::{App, AppCommand, AppRenderContext};
use crate::ui::hints::{HintBar, HintGroup};
use crate::ui::list;
use crate::ui::overlay::ModalShell;
use crate::ui::style;
use crate::ui::text_field::TextField;
use crate::ui::theme::Colors;
use egui::{Color32, Stroke, StrokeKind};
use image::imageops::FilterType;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use helpers::{
    ColumnConfig, ColumnModel, DirStats, TextPreview, format_modified, format_size,
    is_text_preview_candidate, read_text_preview, sort_entries,
};
use icons::paint_entry_icon;

const DETAILS_TABLE_MIN_WIDTH: f32 = 560.0;
const INSPECTOR_MIN_WIDTH: f32 = 920.0;
const DIR_PREVIEW_CAP: usize = 500;
const DETAILS_HEADER_H: f32 = style::TABLE_HEADER_H;
const DETAILS_ROW_H: f32 = style::LIST_ROW_DENSE_H;
const FONT_SIZE_DEFAULT: f32 = style::TEXT_HINT;
const FONT_SIZE_MIN: f32 = 9.0;
const FONT_SIZE_MAX: f32 = 32.0;
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
const PALETTE_MAX_VISIBLE_CANDIDATES: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Normal,
    Search,
    Palette,
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
    EnterPaletteSlash,
    EnterPaletteCommand,
    TabComplete,
    ToggleSort,
    ToggleInspector,
    ToggleQuickLook,
    ToggleHidden,
    Refresh,
    CdTerminalAndClose,
    SelectAll,
    NewFile,
    NewFolder,
    Rename,
    Copy,
    CopyPath,
    Cut,
    Paste,
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

/// Paint `texture` centered in `slot`, scaled down to fit without cropping or
/// distortion. Never scales up: a small image sits at its native size in the
/// middle of the slot rather than turning into a blurry stretch.
fn paint_fitted_texture(
    painter: &egui::Painter,
    slot: egui::Rect,
    texture: &egui::TextureHandle,
) {
    let tex_size = egui::vec2(texture.size()[0] as f32, texture.size()[1] as f32);
    let scale = (slot.width() / tex_size.x)
        .min(slot.height() / tex_size.y)
        .min(1.0);
    let image_rect = egui::Rect::from_center_size(slot.center(), tex_size * scale.max(0.01));
    painter.image(
        texture.id(),
        image_rect,
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        Color32::WHITE,
    );
}

#[derive(Debug, Clone)]
enum PendingFileOperation {
    MoveToTrash { paths: Vec<PathBuf> },
}

fn key_pressed_no_repeat(input: &crate::app::input_router::PlexiInput, key: egui::Key) -> bool {
    input.events().iter().any(
        |e| matches!(e, egui::Event::Key { key: k, pressed: true, repeat: false, .. } if *k == key),
    )
}

fn cmd_key_pressed_no_repeat(
    input: &crate::app::input_router::PlexiInput,
    key: egui::Key,
    shift: bool,
) -> bool {
    input.events().iter().any(|e| {
        matches!(
            e,
            egui::Event::Key {
                key: k,
                pressed: true,
                repeat: false,
                modifiers,
                ..
            } if *k == key
                && (modifiers.command || input.modifiers().command)
                && (modifiers.shift || input.modifiers().shift) == shift
        )
    })
}

// text_entry_active (search or palette) gates all letter keys so that
// j/k/h/l/s/r/slash fall through to AppendText instead of firing navigation or
// action commands while the user is typing a query. Arrow keys, Enter, Escape,
// and Backspace always work.
fn classify_key(
    input: &crate::app::input_router::PlexiInput,
    text_entry_active: bool,
) -> Option<FileBrowserAction> {
    if !text_entry_active && cmd_key_pressed_no_repeat(input, egui::Key::Period, true) {
        return Some(FileBrowserAction::ToggleHidden);
    }
    if !text_entry_active && cmd_key_pressed_no_repeat(input, egui::Key::R, true) {
        return Some(FileBrowserAction::Refresh);
    }
    if !text_entry_active && cmd_key_pressed_no_repeat(input, egui::Key::C, true) {
        return Some(FileBrowserAction::CopyPath);
    }
    if !text_entry_active && cmd_key_pressed_no_repeat(input, egui::Key::A, false) {
        return Some(FileBrowserAction::SelectAll);
    }
    if cmd_key_pressed_no_repeat(input, egui::Key::N, false) {
        return Some(FileBrowserAction::NewFile);
    }
    if cmd_key_pressed_no_repeat(input, egui::Key::N, true) {
        return Some(FileBrowserAction::NewFolder);
    }
    if cmd_key_pressed_no_repeat(input, egui::Key::K, false) {
        return Some(FileBrowserAction::EnterPaletteCommand);
    }
    if cmd_key_pressed_no_repeat(input, egui::Key::F, false) {
        return Some(FileBrowserAction::EnterSearch);
    }
    // Command chords belong to the host shortcut router. F2 and bare `r` are
    // the file-manager rename keys and cannot collide with pane rename (Cmd+R).
    if !text_entry_active
        && !input.modifiers().any()
        && (key_pressed_no_repeat(input, egui::Key::F2)
            || key_pressed_no_repeat(input, egui::Key::R))
    {
        return Some(FileBrowserAction::Rename);
    }
    if !text_entry_active
        && !input.modifiers().any()
        && key_pressed_no_repeat(input, egui::Key::Y)
    {
        return Some(FileBrowserAction::CopyPath);
    }
    if !text_entry_active
        && !input.modifiers().shift
        && (cmd_key_pressed_no_repeat(input, egui::Key::C, false)
            || input.events().iter().any(|e| matches!(e, egui::Event::Copy)))
    {
        return Some(FileBrowserAction::Copy);
    }
    if !text_entry_active
        && !input.modifiers().shift
        && (cmd_key_pressed_no_repeat(input, egui::Key::X, false)
            || input.events().iter().any(|e| matches!(e, egui::Event::Cut)))
    {
        return Some(FileBrowserAction::Cut);
    }
    if !text_entry_active
        && !input.modifiers().shift
        && (cmd_key_pressed_no_repeat(input, egui::Key::V, false)
            || input
                .events()
                .iter()
                .any(|e| matches!(e, egui::Event::Paste(_))))
    {
        return Some(FileBrowserAction::Paste);
    }
    // Cmd+D and Cmd+Shift+D are host split chords. Do not bind them inside
    // File Browser unless the full shortcut map is reviewed.
    if !text_entry_active && cmd_key_pressed_no_repeat(input, egui::Key::Backspace, false) {
        return Some(FileBrowserAction::MoveToTrash);
    }
    if !text_entry_active && cmd_key_pressed_no_repeat(input, egui::Key::O, false) {
        return Some(FileBrowserAction::OpenWithDefault);
    }
    if !text_entry_active && cmd_key_pressed_no_repeat(input, egui::Key::Enter, false) {
        return Some(FileBrowserAction::Reveal);
    }
    if key_pressed_no_repeat(input, egui::Key::Escape) {
        return Some(FileBrowserAction::Escape);
    }
    if text_entry_active
        && !input.modifiers().any()
        && key_pressed_no_repeat(input, egui::Key::Tab)
    {
        return Some(FileBrowserAction::TabComplete);
    }
    if key_pressed_no_repeat(input, egui::Key::Backspace) {
        return Some(FileBrowserAction::Backspace);
    }
    if input.key_pressed(egui::Key::ArrowDown)
        || (!text_entry_active && input.key_pressed(egui::Key::J) && !input.modifiers().any())
    {
        return Some(FileBrowserAction::SelectNext);
    }
    if input.key_pressed(egui::Key::ArrowUp)
        || (!text_entry_active && input.key_pressed(egui::Key::K) && !input.modifiers().any())
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
    if !input.modifiers().command
        && (key_pressed_no_repeat(input, egui::Key::Enter)
            || (!text_entry_active && key_pressed_no_repeat(input, egui::Key::ArrowRight))
            || (!text_entry_active && key_pressed_no_repeat(input, egui::Key::L)))
    {
        return Some(FileBrowserAction::Activate);
    }
    if !input.modifiers().command
        && ((!text_entry_active && key_pressed_no_repeat(input, egui::Key::ArrowLeft))
            || (!text_entry_active && key_pressed_no_repeat(input, egui::Key::H)))
    {
        return Some(FileBrowserAction::NavigateUp);
    }
    if !text_entry_active
        && key_pressed_no_repeat(input, egui::Key::Slash)
        && !input.modifiers().command
    {
        return Some(FileBrowserAction::EnterPaletteSlash);
    }
    if !text_entry_active
        && key_pressed_no_repeat(input, egui::Key::S)
        && !input.modifiers().any()
    {
        return Some(FileBrowserAction::ToggleSort);
    }
    if !text_entry_active
        && key_pressed_no_repeat(input, egui::Key::I)
        && !input.modifiers().any()
    {
        return Some(FileBrowserAction::ToggleInspector);
    }
    if !text_entry_active
        && key_pressed_no_repeat(input, egui::Key::Space)
        && !input.modifiers().any()
    {
        return Some(FileBrowserAction::ToggleQuickLook);
    }
    if !text_entry_active
        && key_pressed_no_repeat(input, egui::Key::T)
        && !input.modifiers().any()
    {
        return Some(FileBrowserAction::CdTerminalAndClose);
    }
    let mut text = String::new();
    for event in input.events() {
        match event {
            egui::Event::Text(t) => text.push_str(t),
            egui::Event::Paste(p) if text_entry_active => text.push_str(p),
            _ => {}
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
    font_size: f32,
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
    pending_clipboard_text: Option<String>,
    pending_operation: Option<PendingFileOperation>,
    rename_path: Option<PathBuf>,
    rename_buffer: String,
    pending_cmds: Vec<AppCommand>,
    /// When true, the next draw_list pass will scroll the selected row into view.
    pending_scroll: bool,
    /// Remembers which entry was selected when leaving a directory,
    /// so navigating back restores the selection.
    directory_selection_memory: std::collections::HashMap<PathBuf, String>,
    // Fuzzy + deep search (stint 0814)
    in_search: bool,
    search_query: String,
    search_indices: Vec<usize>, // indices into `entries` that match the query
    search_session: crate::search::SearchSession,
    search_focus_pending: bool,
    // Action palette (`/` and `Cmd+K`, stint 0813)
    in_palette: bool,
    palette_query: String,
    palette_selected: usize,
    palette_focus_pending: bool,
    palette_cursor_to_end: bool,
    palette_suppress_initial_slash: bool,
    should_close: bool,
    /// Watch on `cwd`, armed lazily from `service_external` and re-armed when
    /// `cwd` moves, so files an agent creates, deletes or renames show up.
    watch: Option<(PathBuf, crate::host::path_watch::PathWatch)>,
    /// In tests: collects paths passed to open_file instead of spawning the system opener.
    #[cfg(test)]
    pub(crate) opened_files: Vec<PathBuf>,
    /// In tests: records the most recent path string copied via `CopyPath`.
    #[cfg(test)]
    pub(crate) last_copied_path: Option<String>,
    /// In tests: captures the last rendered status-bar geometry for alignment assertions.
    #[cfg(test)]
    pub(crate) last_status_bar_layout: Option<StatusBarLayout>,
}

#[cfg(test)]
#[derive(Clone, Debug)]
pub(crate) struct StatusBarLayout {
    pub(crate) row_rect: egui::Rect,
    pub(crate) item_count_rect: egui::Rect,
    pub(crate) chip_rects: Vec<egui::Rect>,
    pub(crate) label_rects: Vec<egui::Rect>,
}

/// Moves `paths` to the user's real Trash, so Finder's Put Back and Empty
/// Trash see them. `NSFileManager` is chosen over the crate's Finder default
/// because it needs no Automation permission prompt.
fn move_to_system_trash(paths: &[PathBuf]) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let context = {
        let mut context = trash::TrashContext::default();
        trash::macos::TrashContextExtMacos::set_delete_method(
            &mut context,
            trash::macos::DeleteMethod::NsFileManager,
        );
        context
    };
    #[cfg(not(target_os = "macos"))]
    let context = trash::TrashContext::default();
    context
        .delete_all(paths)
        .map_err(|err| format!("Unable to move {} item(s) to the Trash: {err}", paths.len()))
}

impl FileBrowserApp {
    pub fn new(cwd: PathBuf) -> Self {
        log::info!("file_browser: opened with middle-elided filenames and tabular counts");
        let mut app = Self {
            cwd,
            entries: Vec::new(),
            selected: 0,
            multi_selected: BTreeSet::new(),
            selection_anchor: None,
            columns: ColumnModel::default(),
            font_size: FONT_SIZE_DEFAULT,
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
            pending_clipboard_text: None,
            pending_operation: None,
            rename_path: None,
            rename_buffer: String::new(),
            pending_cmds: Vec::new(),
            pending_scroll: false,
            directory_selection_memory: std::collections::HashMap::new(),
            in_search: false,
            search_query: String::new(),
            search_indices: Vec::new(),
            search_session: crate::search::SearchSession::new(),
            search_focus_pending: false,
            in_palette: false,
            palette_query: String::new(),
            palette_selected: 0,
            palette_focus_pending: false,
            palette_cursor_to_end: false,
            palette_suppress_initial_slash: false,
            should_close: false,
            watch: None,
            #[cfg(test)]
            opened_files: Vec::new(),
            #[cfg(test)]
            last_copied_path: None,
            #[cfg(test)]
            last_status_bar_layout: None,
        };
        app.refresh();
        app
    }

    fn font_scale(&self) -> f32 {
        self.font_size / FONT_SIZE_DEFAULT
    }

    fn row_height(&self) -> f32 {
        (DETAILS_ROW_H * self.font_scale()).round()
    }

    fn header_height(&self) -> f32 {
        (DETAILS_HEADER_H * self.font_scale()).round()
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
                        let path = e.path();
                        let is_dir = e.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
                        if name.starts_with('.') && !self.columns.show_hidden {
                            return None;
                        }
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

    fn shallow_walk_entries(&self) -> Vec<crate::search::WalkEntry> {
        self.entries
            .iter()
            .map(|e| crate::search::WalkEntry {
                path: e.path.clone(),
                rel_path: e.name.clone(),
                file_name: e.name.clone(),
                is_dir: e.is_dir,
                size_bytes: e.size_bytes,
                modified: e.modified,
                depth: 1,
            })
            .collect()
    }

    fn sync_search_indices(&mut self) {
        self.search_indices = self
            .search_session
            .hits()
            .iter()
            .filter_map(|hit| self.entries.iter().position(|e| e.path == hit.path))
            .collect();
    }

    fn refresh_preserving_filter(&mut self) {
        self.refresh();
        if self.in_search {
            let shallow = self.shallow_walk_entries();
            let hidden = crate::search::HiddenPolicy::from_show_hidden(self.columns.show_hidden);
            self.search_session.update_query(
                &self.cwd,
                hidden,
                &self.search_query,
                &shallow,
                true,
            );
            self.sync_search_indices();
            self.selected = self
                .selected
                .min(self.search_session.hits().len().saturating_sub(1));
        }
    }

    /// Refresh after an external change, keeping the selected path selected
    /// (the index alone would drift when a file lands above it) and the filter
    /// intact. Does not scroll: the user may be looking elsewhere.
    fn refresh_after_external_change(&mut self) {
        let selected_path = self.selected_entry_or_hit().map(|entry| entry.path);
        let scroll = self.pending_scroll;
        self.refresh_preserving_filter();
        if let Some(path) = selected_path {
            self.select_path(&path);
        }
        self.pending_scroll = scroll;
        log::info!(
            "file_browser: listing refreshed after external change cwd={} entries={}",
            self.cwd.display(),
            self.entries.len()
        );
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
    /// Hands the path to the host's single open resolver via `OpenArtifact`
    /// (`OpenInPane`): the host consults `[file_handlers]`, manifest
    /// `file_types`, builtin media players, and finally the OS default opener
    /// (#2283). The File Explorer no longer routes by extension itself — that
    /// logic lives in one place so terminal-driven and Explorer-driven opens
    /// resolve identically.
    fn open_file(&mut self, path: &Path) {
        #[cfg(test)]
        self.opened_files.push(path.to_path_buf());
        if !cfg!(test) {
            log::info!(
                "file_browser: requesting host open-in-pane for '{}'",
                path.display()
            );
        }
        self.pending_cmds.push(AppCommand::OpenArtifact {
            sender_pane_id: 0, // dispatch.rs stamps the real pane_id
            path: path.to_string_lossy().to_string(),
            mode: crate::protocol::ArtifactOpenMode::OpenInPane,
        });
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

    fn selected_search_hit(&self) -> Option<&crate::search::SearchHit> {
        if self.in_search {
            self.search_session.hits().get(self.selected)
        } else {
            None
        }
    }

    fn entry_from_search_hit(hit: &crate::search::SearchHit) -> Entry {
        let ext = hit
            .path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase());
        let is_image = ext
            .as_deref()
            .map(|e| matches!(e, "png" | "jpg" | "jpeg" | "gif" | "bmp" | "tiff" | "webp"))
            .unwrap_or(false);
        Entry {
            name: hit.file_name.clone(),
            path: hit.path.clone(),
            is_dir: hit.is_dir,
            is_image,
            size_bytes: hit.size_bytes,
            modified: hit.modified,
            created: None,
            extension: ext,
            permissions: "rw".to_string(),
        }
    }

    #[cfg(test)]
    pub(crate) fn search_result_count(&self) -> usize {
        self.search_session.hits().len()
    }

    fn selected_entry_or_hit(&self) -> Option<Entry> {
        if let Some(entry) = self.selected_entry() {
            return Some(entry.clone());
        }
        self.selected_search_hit().map(Self::entry_from_search_hit)
    }

    fn selected_entry(&self) -> Option<&Entry> {
        if self.in_search {
            let hit = self.search_session.hits().get(self.selected)?;
            self.entries.iter().find(|e| e.path == hit.path)
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
        if self.in_search {
            self.selected_search_hit()
                .map(|hit| vec![hit.path.clone()])
                .unwrap_or_default()
        } else {
            self.selected_visible_entry_indices()
                .into_iter()
                .filter_map(|idx| self.entries.get(idx).map(|entry| entry.path.clone()))
                .collect()
        }
    }

    fn selected_count(&self) -> usize {
        if self.in_search {
            usize::from(self.selected_search_hit().is_some())
        } else if self.multi_selected.is_empty() {
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
        let shallow = self.shallow_walk_entries();
        let hidden = crate::search::HiddenPolicy::from_show_hidden(self.columns.show_hidden);
        self.search_session
            .update_query(&self.cwd, hidden, &self.search_query, &shallow, false);
        self.sync_search_indices();
        self.selected = 0;
        self.pending_scroll = true;
    }

    fn exit_search(&mut self) {
        self.in_search = false;
        self.search_focus_pending = false;
        self.search_query.clear();
        self.search_indices.clear();
        self.search_session.clear();
        self.selected = 0;
        self.pending_scroll = true;
    }

    pub fn open_palette(&mut self, initial_query: &str) {
        if self.in_search {
            self.exit_search();
        }
        self.in_palette = true;
        self.palette_query = initial_query.to_string();
        self.palette_selected = 0;
        self.palette_focus_pending = true;
        self.palette_cursor_to_end = true;
        self.palette_suppress_initial_slash = initial_query == "/";
        log::info!(
            "file_browser: entering action palette initial_query={:?}",
            initial_query
        );
    }

    pub fn exit_palette(&mut self) {
        self.in_palette = false;
        self.palette_focus_pending = false;
        self.palette_cursor_to_end = false;
        self.palette_suppress_initial_slash = false;
        self.palette_query.clear();
        self.palette_selected = 0;
    }

    pub fn intent_context<'a>(&'a self, selected_paths: &'a [PathBuf]) -> FileExplorerContext<'a> {
        FileExplorerContext::new(
            &self.cwd,
            self.selected_entry(),
            selected_paths,
            &self.entries,
            self.columns.show_hidden,
            self.columns.folders_on_top,
        )
    }

    pub fn parse_intent(&self, input: &str) -> Option<FileExplorerIntent> {
        let selected_paths = self.selected_paths();
        let ctx = self.intent_context(&selected_paths);
        parse_file_explorer_intent(input, &ctx)
    }

    pub fn rank_palette_candidates(&self) -> Vec<PaletteCandidate> {
        let selected_paths = self.selected_paths();
        let ctx = self.intent_context(&selected_paths);
        rank_file_explorer_candidates(&self.palette_query, &ctx)
    }

    fn tab_complete_palette(&mut self) {
        let candidates = self.rank_palette_candidates();
        let Some(chosen) = candidates
            .get(self.palette_selected)
            .or_else(|| candidates.first())
        else {
            return;
        };
        match &chosen.resolution {
            CandidateResolution::NeedsSlot { prompt_prefix, .. } => {
                self.palette_query = (*prompt_prefix).to_string();
                self.palette_selected = 0;
                self.palette_focus_pending = true;
                self.palette_cursor_to_end = true;
            }
            CandidateResolution::Ready(_) => {
                let cmd = chosen
                    .slash_syntax
                    .split_whitespace()
                    .next()
                    .unwrap_or(chosen.slash_syntax);
                self.palette_query = cmd.to_string();
                self.palette_selected = 0;
                self.palette_focus_pending = true;
                self.palette_cursor_to_end = true;
            }
        }
    }

    fn submit_palette(&mut self) {
        let candidates = self.rank_palette_candidates();
        if let Some(chosen) = candidates
            .get(self.palette_selected)
            .or_else(|| candidates.first())
            .cloned()
        {
            match chosen.resolution {
                CandidateResolution::Ready(intent) => {
                    self.exit_palette();
                    if let Err(err) = self.execute_intent(intent) {
                        self.error = Some(err);
                    }
                }
                CandidateResolution::NeedsSlot { prompt_prefix, .. } => {
                    self.palette_query = prompt_prefix.to_string();
                    self.palette_selected = 0;
                    self.palette_focus_pending = true;
                    self.palette_cursor_to_end = true;
                    log::info!(
                        "file_browser: palette prompting for slot prefix={:?}",
                        prompt_prefix
                    );
                }
            }
            return;
        }

        if let Some(intent) = self.parse_intent(&self.palette_query) {
            self.exit_palette();
            if let Err(err) = self.execute_intent(intent) {
                self.error = Some(err);
            }
        }
    }

    pub fn execute_intent(&mut self, intent: FileExplorerIntent) -> Result<(), String> {
        log::info!("file_browser: palette intent={:?}", intent);
        match intent {
            FileExplorerIntent::CreateFile {
                name,
                open_in_editor,
            } => {
                self.create_named_file(&name, open_in_editor)?;
                Ok(())
            }
            FileExplorerIntent::CreateFolder { name } => {
                self.create_named_folder(&name)?;
                Ok(())
            }
            FileExplorerIntent::Rename { target, new_name } => {
                self.rename_path = Some(target);
                let result = self.rename_selected(&new_name);
                self.rename_path = None;
                self.rename_buffer.clear();
                result.map(|_| ())
            }
            FileExplorerIntent::MoveToTrash { targets } => {
                if targets.is_empty() {
                    return Err("No items selected to move to Trash".to_string());
                }
                log::info!(
                    "file_browser: requested move to trash confirmation count={}",
                    targets.len()
                );
                self.pending_operation = Some(PendingFileOperation::MoveToTrash { paths: targets });
                Ok(())
            }
            FileExplorerIntent::OpenInEditor { path } => {
                if path.is_dir() {
                    self.navigate_into(path);
                } else {
                    self.open_file(&path);
                }
                Ok(())
            }
            FileExplorerIntent::OpenWithDefault { path } => {
                log::info!("file_browser: open with default '{}'", path.display());
                self.pending_cmds.push(AppCommand::OpenArtifact {
                    sender_pane_id: 0,
                    path: path.to_string_lossy().to_string(),
                    mode: crate::protocol::ArtifactOpenMode::OpenWithDefault,
                });
                Ok(())
            }
            FileExplorerIntent::Reveal { path } => {
                log::info!("file_browser: reveal '{}'", path.display());
                self.pending_cmds.push(AppCommand::OpenArtifact {
                    sender_pane_id: 0,
                    path: path.to_string_lossy().to_string(),
                    mode: crate::protocol::ArtifactOpenMode::RevealInFinder,
                });
                Ok(())
            }
            FileExplorerIntent::CdTerminal { path } => {
                let target_dir = if path.is_dir() {
                    path
                } else {
                    path.parent()
                        .map(Path::to_path_buf)
                        .unwrap_or_else(|| self.cwd.clone())
                };
                log::info!(
                    "file_browser: explicit cd handoff to linked terminal: '{}'",
                    target_dir.display()
                );
                self.pending_cmds.push(AppCommand::CdRequest {
                    cwd: target_dir.to_string_lossy().to_string(),
                    sender_pane_id: 0,
                });
                self.should_close = true;
                Ok(())
            }
            FileExplorerIntent::SortBy { column, direction } => {
                self.columns.sort = helpers::SortDescriptor { column, direction };
                self.refresh_preserving_filter();
                log::info!(
                    "file_browser: sort changed to {} {}",
                    column.key(),
                    direction.key()
                );
                Ok(())
            }
            FileExplorerIntent::ToggleHidden { enabled } => {
                self.columns.show_hidden = enabled.unwrap_or(!self.columns.show_hidden);
                self.refresh_preserving_filter();
                log::info!(
                    "file_browser: show_hidden toggled to {}",
                    self.columns.show_hidden
                );
                Ok(())
            }
            FileExplorerIntent::ToggleFoldersFirst { enabled } => {
                self.columns.folders_on_top = enabled.unwrap_or(!self.columns.folders_on_top);
                self.refresh_preserving_filter();
                log::info!(
                    "file_browser: folders_on_top changed to {}",
                    self.columns.folders_on_top
                );
                Ok(())
            }
        }
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

    fn create_named_folder(&mut self, name: &str) -> Result<PathBuf, String> {
        let trimmed = name.trim();
        if trimmed.is_empty() || trimmed == "." || trimmed == ".." || trimmed.contains('/') {
            return Err("Folder name must be a non-empty single name".to_string());
        }
        if trimmed.starts_with('.') && !self.columns.show_hidden {
            self.columns.show_hidden = true;
        }
        let path = self.cwd.join(trimmed);
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

    fn create_named_file(&mut self, name: &str, open_in_editor: bool) -> Result<PathBuf, String> {
        let trimmed = name.trim();
        if trimmed.is_empty() || trimmed == "." || trimmed == ".." || trimmed.contains('/') {
            return Err("File name must be a non-empty single name".to_string());
        }
        if trimmed.starts_with('.') && !self.columns.show_hidden {
            self.columns.show_hidden = true;
        }
        let path = self.cwd.join(trimmed);
        if path.is_dir() {
            let msg = format!(
                "Unable to create file '{}': a folder with that name already exists",
                path.display()
            );
            log::warn!("file_browser: {msg}");
            return Err(msg);
        }
        if !path.exists() {
            fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .map_err(|err| {
                    let msg = format!("Unable to create file '{}': {err}", path.display());
                    log::warn!("file_browser: {msg}");
                    msg
                })?;
        }
        log::info!("file_browser: created file '{}'", path.display());
        self.refresh_preserving_filter();
        self.select_path(&path);
        if open_in_editor {
            self.open_file(&path);
        }
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
        // Rename stays in the source's own directory — `cwd` can differ from
        // the source's parent (search results, host-process cwd) and joining
        // it relocates the file instead of renaming it.
        let target = match source.parent() {
            Some(parent) => parent.join(trimmed),
            None => return Err(format!("'{}' has no parent directory", source.display())),
        };
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
            let (stem, ext) = if source.is_dir() {
                (file_name, None)
            } else {
                match (
                    source.file_stem().and_then(|s| s.to_str()),
                    source.extension().and_then(|e| e.to_str()),
                ) {
                    (Some(s), Some(e)) if !s.is_empty() => (s, Some(e)),
                    _ => (file_name, None),
                }
            };
            let target = Self::unique_child_path(&self.cwd, stem, ext);
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
                move_to_system_trash(&paths)?;
                for source in &paths {
                    log::info!(
                        "file_browser: moved '{}' to the system Trash",
                        source.display()
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
            self.pending_cmds.push(AppCommand::OpenArtifact {
                sender_pane_id: 0,
                path: path.to_string_lossy().to_string(),
                mode: crate::protocol::ArtifactOpenMode::RevealInFinder,
            });
        }
    }

    fn open_selected_with_default(&mut self) {
        for path in self.selected_paths() {
            log::info!("file_browser: open with default '{}'", path.display());
            self.pending_cmds.push(AppCommand::OpenArtifact {
                sender_pane_id: 0,
                path: path.to_string_lossy().to_string(),
                mode: crate::protocol::ArtifactOpenMode::OpenWithDefault,
            });
        }
    }

    fn copy_selected_path(&mut self) {
        let paths = self.selected_paths();
        if paths.is_empty() {
            return;
        }
        let text = paths
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join("\n");
        log::info!("file_browser: copied path '{}' to clipboard", text);
        self.pending_clipboard_text = Some(text.clone());
        #[cfg(test)]
        {
            self.last_copied_path = Some(text);
        }
        #[cfg(not(test))]
        if let Ok(mut clipboard) = arboard::Clipboard::new() {
            if let Err(err) = clipboard.set_text(&text) {
                log::warn!("file_browser: clipboard write failed: {err}");
            }
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
            self.search_session.hits().len()
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
        }
    }

    /// Apply one row's click and double-click to the selection. Returns the
    /// navigation target when the row was opened.
    fn apply_row_click(
        &mut self,
        ui: &egui::Ui,
        idx: usize,
        entry: &Entry,
        clicked: bool,
        double_clicked: bool,
    ) -> Option<(PathBuf, bool)> {
        if clicked {
            let modifiers = ui.input(|input| input.modifiers);
            if modifiers.shift {
                self.extend_selection_to(idx);
            } else if modifiers.command {
                self.toggle_selection(idx);
            } else {
                self.set_single_selection(idx);
            }
        }
        if double_clicked {
            self.set_single_selection(idx);
            return Some((entry.path.clone(), entry.is_dir));
        }
        None
    }

    fn draw_details_table(
        &mut self,
        ui: &mut egui::Ui,
        colors: &Colors,
    ) -> Option<(PathBuf, bool)> {
        let should_scroll = self.pending_scroll;
        self.pending_scroll = false;
        self.draw_details_header(ui, colors);

        let row_h = self.row_height();
        let mut navigate_to = None;
        if self.in_search {
            let hits = self.search_session.hits().to_vec();
            if hits.is_empty() {
                let empty_msg = if self.search_session.is_scanning() {
                    "Scanning workspace\u{2026}"
                } else {
                    "No matching files or contents"
                };
                ui.add_space(style::SPACE_SM);
                ui.colored_label(
                    colors.text_dim,
                    egui::RichText::new(empty_msg).size(self.font_size),
                );
                return None;
            }
            for (idx, hit) in hits.iter().enumerate() {
                let entry = Self::entry_from_search_hit(hit);
                let (rect, response) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), row_h),
                    egui::Sense::click(),
                );
                let selected = self.selected == idx;
                if selected {
                    crate::ui::list::scroll_row_into_view(ui, &response, should_scroll);
                }
                list::paint_table_row_background(ui, rect, selected, response.hovered(), colors);

                self.paint_search_hit_cells(ui, colors, rect, hit, &entry, selected);

                if response.clicked() {
                    self.set_single_selection(idx);
                }
                if response.double_clicked() {
                    self.set_single_selection(idx);
                    navigate_to = Some((hit.path.clone(), hit.is_dir));
                }
            }
            return navigate_to;
        }
        for (idx, actual_idx) in self.visible_entry_indices().into_iter().enumerate() {
            let entry = self.entries[actual_idx].clone();
            let (rect, response) = ui.allocate_exact_size(
                egui::vec2(ui.available_width(), row_h),
                egui::Sense::click(),
            );
            let selected = self.is_entry_selected(idx, &entry);
            if self.selected == idx {
                crate::ui::list::scroll_row_into_view(ui, &response, should_scroll);
            }
            list::paint_table_row_background(ui, rect, selected, response.hovered(), colors);

            self.paint_details_cells(ui, colors, rect, &entry, selected);

            if let Some(target) = self.apply_row_click(
                ui,
                idx,
                &entry,
                response.clicked(),
                response.double_clicked(),
            ) {
                navigate_to = Some(target);
            }
        }
        navigate_to
    }

    fn draw_details_header(&mut self, ui: &mut egui::Ui, colors: &Colors) {
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), self.header_height()),
            egui::Sense::hover(),
        );
        list::paint_table_header_background(ui, rect, colors);
        let header_font = egui::FontId::proportional(self.font_size);
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
            list::paint_table_header_label(
                ui,
                cell,
                format!("{}{}", column.id.label(), sort_suffix),
                header_font.clone(),
                colors,
            );
            ui.painter().line_segment(
                [
                    egui::pos2(cell.right(), cell.top() + 5.0),
                    egui::pos2(cell.right(), cell.bottom() - 5.0),
                ],
                Stroke::new(1.0_f32, colors.border),
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
        let is_hidden_entry = entry.name.starts_with('.');
        let name_color = if selected || !is_hidden_entry {
            colors.text_primary
        } else {
            colors.text_dim
        };
        let font = egui::FontId::proportional(self.font_size);
        let icon_size = (18.0 * self.font_scale()).round();
        for (column, mut cell) in self.details_columns(rect) {
            if column.id == ColumnId::Name {
                let icon_rect = egui::Rect::from_min_size(
                    egui::pos2(
                        cell.left() + style::SPACE_SM,
                        cell.center().y - icon_size * 0.5,
                    ),
                    egui::vec2(icon_size, icon_size),
                );
                paint_entry_icon(ui.painter(), icon_rect, entry, colors);
                cell.min.x = (icon_rect.right() + 2.0).min(cell.right());
            }
            list::paint_table_cell_text_elided(
                ui,
                cell,
                Self::entry_cell_text(entry, column.id),
                font.clone(),
                if column.id == ColumnId::Name {
                    name_color
                } else {
                    colors.text_dim
                },
                if column.id == ColumnId::Name {
                    crate::ui::text::Side::Middle
                } else {
                    crate::ui::text::Side::Trailing
                },
            );
        }
    }

    fn paint_search_hit_cells(
        &self,
        ui: &mut egui::Ui,
        colors: &Colors,
        rect: egui::Rect,
        hit: &crate::search::SearchHit,
        entry: &Entry,
        selected: bool,
    ) {
        let is_hidden_entry = hit.file_name.starts_with('.');
        let name_color = if selected || !is_hidden_entry {
            colors.text_primary
        } else {
            colors.text_dim
        };
        let font = egui::FontId::proportional(self.font_size);
        let icon_size = (18.0 * self.font_scale()).round();
        for (column, mut cell) in self.details_columns(rect) {
            if column.id == ColumnId::Name {
                let icon_rect = egui::Rect::from_min_size(
                    egui::pos2(
                        cell.left() + style::SPACE_SM,
                        cell.center().y - icon_size * 0.5,
                    ),
                    egui::vec2(icon_size, icon_size),
                );
                paint_entry_icon(ui.painter(), icon_rect, entry, colors);
                cell.min.x = (icon_rect.right() + 2.0).min(cell.right());
                let text_rect = cell.shrink2(egui::vec2(6.0, 0.0));
                if text_rect.width() > 4.0 {
                    let job = crate::ui::search_bar::build_search_hit_job(
                        hit,
                        font.clone(),
                        name_color,
                        colors,
                    );
                    let galley = ui.fonts_mut(|f| f.layout_job(job));
                    let pos = egui::pos2(
                        text_rect.left(),
                        text_rect.center().y - galley.size().y * 0.5,
                    );
                    ui.painter()
                        .with_clip_rect(text_rect)
                        .galley(pos, galley, name_color);
                }
            } else {
                list::paint_table_cell_text(
                    ui,
                    cell,
                    Self::entry_cell_text(entry, column.id),
                    font.clone(),
                    colors.text_dim,
                );
            }
        }
    }

    fn details_columns(&self, rect: egui::Rect) -> Vec<(ColumnConfig, egui::Rect)> {
        let enabled = self.columns.visible_columns().copied().collect::<Vec<_>>();
        let name_min = ColumnId::Name.min_width();
        let mut admitted_secondary = Vec::new();
        let mut used_width = name_min;

        // Progressively admit secondary columns alongside Name in priority order:
        // Modified, then Size, then any extra user-enabled columns.
        let priority_order = [
            ColumnId::Modified,
            ColumnId::Size,
            ColumnId::Kind,
            ColumnId::Created,
            ColumnId::Extension,
            ColumnId::Permissions,
        ];
        for id in priority_order {
            if let Some(col) = enabled.iter().find(|c| c.id == id).copied() {
                let col_w = col.width.max(col.id.min_width());
                if used_width + col_w <= rect.width() {
                    used_width += col_w;
                    admitted_secondary.push(col.id);
                }
            }
        }

        let visible: Vec<ColumnConfig> = enabled
            .into_iter()
            .filter(|c| c.id == ColumnId::Name || admitted_secondary.contains(&c.id))
            .collect();

        let secondary_widths: f32 = visible
            .iter()
            .filter(|c| c.id != ColumnId::Name)
            .map(|c| c.width.max(c.id.min_width()))
            .sum();
        let name_width = (rect.width() - secondary_widths)
            .max(name_min)
            .min(rect.width());

        let mut x = rect.left();
        let mut columns = Vec::with_capacity(visible.len());
        for (idx, column) in visible.iter().enumerate() {
            let col_w = if column.id == ColumnId::Name {
                name_width
            } else {
                column.width.max(column.id.min_width())
            };
            let right = if idx + 1 == visible.len() {
                rect.right()
            } else {
                (x + col_w).min(rect.right())
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
        let entry = match self.selected_entry_or_hit() {
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
            .stroke(Stroke::new(1.0_f32, colors.border))
            .corner_radius(crate::ui::style::RADIUS_SMALL)
            .inner_margin(egui::Margin::same(8))
            .show(ui, |ui| {
                ui.label(
                    egui::RichText::new(format!("Image \u{00b7} {}", entry.name))
                        .size(style::TEXT_HINT)
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
                ui.add_space(style::SPACE_SM);
                let preview_max = egui::vec2(ui.available_width(), 220.0);
                let (slot_rect, _) = ui.allocate_exact_size(preview_max, egui::Sense::hover());
                ui.painter().rect_filled(
                    slot_rect,
                    style::RADIUS_MD,
                    colors.bg_darkest.gamma_multiply(0.95),
                );
                ui.painter().rect_stroke(
                    slot_rect,
                    style::RADIUS_MD,
                    Stroke::new(1.0_f32, colors.border),
                    StrokeKind::Inside,
                );
                if let Some(texture) = &self.preview_texture {
                    paint_fitted_texture(ui.painter(), slot_rect, texture);
                } else if let Some(err) = &self.preview_error {
                    ui.painter().text(
                        slot_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        err,
                        egui::FontId::proportional(style::TEXT_META),
                        colors.danger,
                    );
                } else {
                    ui.painter().text(
                        slot_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "Loading\u{2026}",
                        egui::FontId::proportional(style::TEXT_META),
                        colors.text_dim,
                    );
                }
                ui.add_space(6.0);
                if let Some(size) = entry.size_bytes {
                    ui.label(
                        egui::RichText::new(format_size(Some(size)))
                            .size(style::TEXT_META)
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
            .stroke(Stroke::new(1.0_f32, colors.border))
            .corner_radius(crate::ui::style::RADIUS_SMALL)
            .inner_margin(egui::Margin::same(8))
            .show(ui, |ui| {
                ui.label(
                    egui::RichText::new(format!("Folder \u{00b7} {}", entry.name))
                        .size(style::TEXT_HINT)
                        .color(colors.text_primary)
                        .strong(),
                );
                ui.separator();
                ui.label(
                    egui::RichText::new(format!("Path: {}", entry.path.display()))
                        .size(style::TEXT_META)
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
                    .size(style::TEXT_META)
                    .color(colors.text_primary),
                );
                ui.label(
                    egui::RichText::new(format!(
                        "Immediate file size: {}",
                        format_size(stats.map(|s| s.total_bytes))
                    ))
                    .size(style::TEXT_META)
                    .color(colors.text_primary),
                );
            });
    }

    fn draw_text_sidebar(&mut self, ui: &mut egui::Ui, colors: &Colors, entry: &Entry) {
        let path = entry.path.clone();
        self.ensure_text_preview(&path);

        egui::Frame::new()
            .fill(colors.bg_sidebar)
            .stroke(Stroke::new(1.0_f32, colors.border))
            .corner_radius(crate::ui::style::RADIUS_SMALL)
            .inner_margin(egui::Margin::same(8))
            .show(ui, |ui| {
                ui.label(
                    egui::RichText::new(format!("Text \u{00b7} {}", entry.name))
                        .size(style::TEXT_HINT)
                        .color(colors.text_primary)
                        .strong(),
                );
                if let Some(size) = entry.size_bytes {
                    ui.label(
                        egui::RichText::new(format_size(Some(size)))
                            .size(style::TEXT_META)
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
            .stroke(Stroke::new(1.0_f32, colors.border))
            .corner_radius(crate::ui::style::RADIUS_SMALL)
            .inner_margin(egui::Margin::same(8))
            .show(ui, |ui| {
                ui.label(
                    egui::RichText::new(entry.name.clone())
                        .size(style::TEXT_HINT)
                        .color(colors.text_primary)
                        .strong(),
                );
                ui.separator();
                if let Some(size) = entry.size_bytes {
                    ui.label(
                        egui::RichText::new(format_size(Some(size)))
                            .size(style::TEXT_META)
                            .color(colors.text_dim),
                    );
                }
                if let Some(modified) = entry.modified {
                    ui.label(
                        egui::RichText::new(format!(
                            "Modified: {}",
                            format_modified(Some(modified))
                        ))
                        .size(style::TEXT_META)
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
                text.push('\u{2026}');
            }
            egui::ScrollArea::vertical()
                .max_height(max_height)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(text)
                                .monospace()
                                .size(self.font_size)
                                .color(colors.text_primary),
                        )
                        .wrap(),
                    );
                });
        } else if let Some(err) = &self.text_preview_error {
            ui.label(
                egui::RichText::new(err)
                    .size(self.font_size)
                    .color(colors.text_dim),
            );
        } else {
            ui.label(
                egui::RichText::new("Loading\u{2026}")
                    .size(self.font_size)
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
                        "Move {count} selected item{} to the Trash?",
                        if count == 1 { "" } else { "s" }
                    ))
                    .size(style::TEXT_BODY)
                    .color(colors.text_primary),
                );
                ui.add_space(style::SPACE_MD);
                ui.horizontal(|ui| {
                    if crate::ui::button::chrome_button(
                        ui,
                        "Cancel",
                        crate::ui::button::ButtonKind::Secondary,
                        colors,
                        style::BUTTON_MIN_W_MODAL,
                    )
                    .clicked()
                    {
                        self.cancel_pending_operation();
                    }
                    if crate::ui::button::chrome_button(
                        ui,
                        "Move to Trash",
                        crate::ui::button::ButtonKind::Danger,
                        colors,
                        style::BUTTON_MIN_W_MODAL,
                    )
                    .clicked()
                    {
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

    fn draw_rename_modal(&mut self, ctx: &egui::Context, colors: &Colors, pane_key: u64) {
        if self.rename_path.is_none() {
            return;
        }
        let response = ModalShell::centered("file_browser_rename")
            .title("Rename")
            .escape(true)
            .show(ctx, colors, |ui| {
                let text_response = TextField::singleline(
                    egui::Id::new(("file_browser_rename_input", pane_key)),
                    "",
                )
                .select_all_on_focus(true)
                .log_name("file_browser_rename")
                .show(ui, &mut self.rename_buffer, colors);
                let submitted = text_response.lost_focus()
                    && ui.input(|input| input.key_pressed(egui::Key::Enter));
                // Claim the rename field while the modal is open (stint 0429):
                // the reconciler grants it while this pane owns input and
                // surrenders it if ownership moves elsewhere.
                crate::ui::focus::claim_text_surface(
                    ctx,
                    crate::ui::focus::SurfaceKey::Pane(pane_key),
                    text_response.id,
                );
                if submitted {
                    self.confirm_rename_modal();
                }
                ui.add_space(style::SPACE_MD);
                ui.horizontal(|ui| {
                    if crate::ui::button::chrome_button(
                        ui,
                        "Cancel",
                        crate::ui::button::ButtonKind::Secondary,
                        colors,
                        style::BUTTON_MIN_W_MODAL,
                    )
                    .clicked()
                    {
                        self.cancel_rename_modal();
                    }
                    if crate::ui::button::chrome_button(
                        ui,
                        "Rename",
                        crate::ui::button::ButtonKind::Primary,
                        colors,
                        style::BUTTON_MIN_W_MODAL,
                    )
                    .clicked()
                    {
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
            Stroke::new(1.0_f32, colors.border),
            StrokeKind::Inside,
        );
        if let Some(texture) = &self.preview_texture {
            paint_fitted_texture(ui.painter(), slot_rect, texture);
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

    fn breadcrumb_segments(path: &Path) -> Vec<(String, PathBuf)> {
        let mut ancestors: Vec<&Path> = path.ancestors().collect();
        ancestors.reverse();
        ancestors
            .into_iter()
            .map(|ancestor| {
                let label = if ancestor.parent().is_none() {
                    ancestor.to_string_lossy().into_owned()
                } else {
                    ancestor
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| ancestor.display().to_string())
                };
                (label, ancestor.to_path_buf())
            })
            .collect()
    }

    fn draw_toolbar(
        &mut self,
        ui: &mut egui::Ui,
        colors: &Colors,
        pending_click: Option<crate::host::pane::PendingPaneClick>,
    ) {
        let mut navigate_target: Option<PathBuf> = None;
        let font_size = self.font_size;
        let font_id = egui::FontId::monospace(font_size);

        ui.horizontal(|ui| {
            let view_reserve = 72.0 * self.font_scale();
            let max_breadcrumb_w = (ui.available_width() - view_reserve).max(48.0);
            let segments = Self::breadcrumb_segments(&self.cwd);

            let measure = |ui: &egui::Ui, s: &str| -> f32 {
                ui.fonts_mut(|f| f.layout_no_wrap(s.to_owned(), font_id.clone(), egui::Color32::WHITE).size().x)
            };
            let sep_w = measure(ui, "/") + style::SPACE_XS * 2.0;
            let ellipsis_w = measure(ui, "\u{2026}") + sep_w;

            let mut start = 0usize;
            while start + 1 < segments.len() {
                let prefix_w = if start > 0 { ellipsis_w } else { 0.0 };
                let segs_w: f32 = segments[start..]
                    .iter()
                    .enumerate()
                    .map(|(i, (label, _))| {
                        let w = measure(ui, label) + style::SPACE_XS * 2.0;
                        if i > 0 && segments[start + i - 1].0 != "/" {
                            w + sep_w
                        } else {
                            w
                        }
                    })
                    .sum();
                if prefix_w + segs_w <= max_breadcrumb_w {
                    break;
                }
                start += 1;
            }

            ui.spacing_mut().item_spacing.x = 2.0;
            if start > 0 {
                ui.label(
                    egui::RichText::new("\u{2026}")
                        .size(font_size)
                        .monospace()
                        .color(colors.text_dim),
                );
                ui.label(
                    egui::RichText::new("/")
                        .size(font_size)
                        .monospace()
                        .color(colors.text_dim),
                );
            }

            let visible_slice = &segments[start..];
            for (idx, (label, seg_path)) in visible_slice.iter().enumerate() {
                let is_last = idx + 1 == visible_slice.len();
                if idx > 0 && visible_slice[idx - 1].0 != "/" {
                    ui.label(
                        egui::RichText::new("/")
                            .size(font_size)
                            .monospace()
                            .color(colors.text_dim),
                    );
                }
                let display_label = if visible_slice.len() == 1 {
                    crate::ui::text::elide(
                        ui,
                        label,
                        font_id.clone(),
                        max_breadcrumb_w,
                        crate::ui::text::Side::Leading,
                    )
                } else {
                    label.clone()
                };
                let text_color = if is_last {
                    colors.accent
                } else {
                    colors.text_primary
                };
                let btn = ui.add(
                    egui::Button::new(
                        egui::RichText::new(display_label)
                            .size(font_size)
                            .monospace()
                            .color(text_color),
                    )
                    .frame(false),
                );
                let synthetic_click =
                    pending_click.is_some_and(|click| click.targets_id(btn.id));
                if btn.clicked() || synthetic_click {
                    navigate_target = Some(seg_path.clone());
                }
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // Hand-rolled expansion of `ui.menu_button` (egui's
                // `stationary_menu_impl`) so a queued node click can open the
                // popup. `menu_button` flips its open state only from a real
                // click resolved inside `Context::begin_pass`, which
                // `plexi pane click --node` / `PendingPaneClick` bypasses
                // (stint 0469).
                let bar_id = ui.id().with("view_menu");
                let menu_button = ui.add(
                    egui::Button::new(
                        egui::RichText::new("View \u{2304}")
                            .size(style::TEXT_HINT)
                            .color(colors.text_secondary(colors.bg_elevated)),
                    )
                    .fill(colors.bg_elevated)
                    .stroke(Stroke::new(1.0_f32, colors.border)),
                );
                let synthetic_open =
                    pending_click.is_some_and(|click| click.targets_id(menu_button.id));
                if synthetic_open {
                    log::info!("file_browser: View menu opened via synthetic node click");
                }
                let set_open = if synthetic_open {
                    Some(egui::SetOpenCommand::Bool(true))
                } else if menu_button.clicked() {
                    Some(egui::SetOpenCommand::Toggle)
                } else {
                    None
                };
                egui::Popup::menu(&menu_button)
                    .id(bar_id)
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                    .open_memory(set_open)
                    .show(|ui| {
                        let mut folders_on_top = self.columns.folders_on_top;
                        if ui.checkbox(&mut folders_on_top, "Folders first").changed() {
                            self.columns.folders_on_top = folders_on_top;
                            self.refresh_preserving_filter();
                            log::info!(
                                "file_browser: folders_on_top changed to {}",
                                self.columns.folders_on_top
                            );
                        }
                        let mut show_hidden = self.columns.show_hidden;
                        let show_hidden_resp = ui.checkbox(&mut show_hidden, "Show hidden files");
                        let synthetic_toggle =
                            pending_click.is_some_and(|c| c.targets_id(show_hidden_resp.id));
                        if show_hidden_resp.changed() || synthetic_toggle {
                            self.columns.show_hidden = if show_hidden_resp.changed() {
                                show_hidden
                            } else {
                                !self.columns.show_hidden
                            };
                            self.refresh_preserving_filter();
                            log::info!(
                                "file_browser: show_hidden changed to {}",
                                self.columns.show_hidden
                            );
                        }
                        let mut inspector = self.inspector_open;
                        if ui.checkbox(&mut inspector, "Inspector").changed() {
                            self.toggle_inspector();
                        }
                        ui.separator();
                        for id in [
                            ColumnId::Kind,
                            ColumnId::Size,
                            ColumnId::Modified,
                            ColumnId::Created,
                            ColumnId::Extension,
                            ColumnId::Permissions,
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
                    });
            });
        });

        if let Some(target) = navigate_target {
            self.navigate_into(target);
        }
    }

    fn draw_search_bar(
        &mut self,
        ui: &mut egui::Ui,
        colors: &Colors,
        pane_id: u64,
        pending_click: Option<crate::host::pane::PendingPaneClick>,
    ) {
        let claim = std::mem::take(&mut self.search_focus_pending);
        let hit_count = self.search_session.hits().len();
        let scanning = self.search_session.is_scanning();
        let mut bar = crate::ui::search_bar::SearchBar::new(
            ("file_browser_search", pane_id),
            "Search files and contents\u{2026}",
        )
        .claim_focus(claim)
        .status(hit_count, scanning)
        .font_size(self.font_size)
        .pending_click(pending_click);
        if self.rename_path.is_none() {
            bar = bar.surface(crate::ui::focus::SurfaceKey::Pane(pane_id));
        }
        let out = bar.show(ui, &mut self.search_query, colors);
        if out.changed {
            self.refilter();
        }
    }

    fn draw_action_palette(
        &mut self,
        ui: &mut egui::Ui,
        colors: &Colors,
        pane_id: u64,
        pending_click: Option<crate::host::pane::PendingPaneClick>,
    ) {
        let claim = std::mem::take(&mut self.palette_focus_pending);
        let cursor_to_end = std::mem::take(&mut self.palette_cursor_to_end);
        let suppress_slash = std::mem::take(&mut self.palette_suppress_initial_slash);
        let input_id = ui.make_persistent_id(("file_browser_palette", pane_id));

        let set_cursor_end = |ctx: &egui::Context, text: &str| {
            let mut state = egui::TextEdit::load_state(ctx, input_id).unwrap_or_default();
            let end = egui::text::CCursor::new(text.chars().count());
            state
                .cursor
                .set_char_range(Some(egui::text::CCursorRange::one(end)));
            state.store(ctx, input_id);
        };

        let mut clicked_candidate: Option<usize> = None;
        let mut close_clicked = false;
        let mut submitted_from_widget = false;

        ui.add_space(style::SPACE_XS);
        egui::Frame::new()
            .fill(colors.bg_toolbar)
            .stroke(Stroke::new(1.0_f32, colors.accent.gamma_multiply(0.65)))
            .corner_radius(style::RADIUS_MD)
            .inner_margin(egui::Margin::symmetric(10, 8))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    crate::ui::badge::badge(ui, "ACTION", colors.accent, colors);

                    if self.rename_path.is_none() {
                        crate::ui::focus::register_default_text_surface(
                            ui.ctx(),
                            crate::ui::focus::SurfaceKey::Pane(pane_id),
                            input_id,
                        );
                        if claim {
                            crate::ui::focus::claim_text_surface(
                                ui.ctx(),
                                crate::ui::focus::SurfaceKey::Pane(pane_id),
                                input_id,
                            );
                        }
                    }
                    if cursor_to_end {
                        set_cursor_end(ui.ctx(), &self.palette_query);
                    }

                    let before = self.palette_query.clone();
                    let clear_reserve = 28.0;
                    let field_w = (ui.available_width() - clear_reserve).max(80.0);
                    let resp = ui
                        .allocate_ui(egui::vec2(field_w, 28.0), |ui| {
                            TextField::singleline(
                                input_id,
                                "Type a /command or intent (e.g. /new file notes.md, /hidden, rename to \u{2026})",
                            )
                            .log_name("file_browser_palette")
                            .show(ui, &mut self.palette_query, colors)
                        })
                        .inner;

                    if suppress_slash && self.palette_query.starts_with("//") {
                        self.palette_query =
                            format!("/{}", self.palette_query.trim_start_matches('/'));
                        set_cursor_end(ui.ctx(), &self.palette_query);
                    } else if cursor_to_end {
                        set_cursor_end(ui.ctx(), &self.palette_query);
                    }

                    if self.palette_query != before {
                        self.palette_selected = 0;
                    }

                    let enter_event = ui.input(|i| {
                        i.events.iter().any(|e| {
                            matches!(
                                e,
                                egui::Event::Key {
                                    key: egui::Key::Enter,
                                    pressed: true,
                                    ..
                                }
                            )
                        })
                    });
                    if resp.lost_focus() && enter_event {
                        submitted_from_widget = true;
                    }

                    let close_btn = ui.add(
                        egui::Button::new(
                            egui::RichText::new("\u{00d7}")
                                .size(self.font_size)
                                .color(colors.text_dim),
                        )
                        .frame(false),
                    );
                    let synthetic_close =
                        pending_click.is_some_and(|c| c.targets_id(close_btn.id));
                    if close_btn.clicked() || synthetic_close {
                        close_clicked = true;
                    }
                });

                let candidates = self.rank_palette_candidates();
                if !candidates.is_empty() {
                    self.palette_selected =
                        self.palette_selected.min(candidates.len().saturating_sub(1));
                }

                // Structured intent preview chip
                ui.add_space(style::SPACE_XS);
                if let Some((badge_label, chip_text, is_ready)) =
                    self.palette_preview_chip(&candidates)
                {
                    let chip_bg = if is_ready {
                        colors.accent.gamma_multiply(0.14)
                    } else {
                        colors.bg_sidebar
                    };
                    let chip_border = if is_ready {
                        colors.accent.gamma_multiply(0.55)
                    } else {
                        colors.border
                    };
                    egui::Frame::new()
                        .fill(chip_bg)
                        .stroke(Stroke::new(1.0_f32, chip_border))
                        .corner_radius(style::RADIUS_SM)
                        .inner_margin(egui::Margin::symmetric(8, 4))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                let badge_fill = if is_ready {
                                    colors.accent
                                } else {
                                    colors.bg_hover
                                };
                                crate::ui::badge::badge(ui, badge_label, badge_fill, colors);
                                ui.label(
                                    egui::RichText::new(chip_text)
                                        .size(style::TEXT_META)
                                        .monospace()
                                        .color(colors.text_primary),
                                );
                            });
                        });
                }

                // Ranked candidate list
                ui.add_space(style::SPACE_XS);
                let row_h = (22.0 * self.font_scale()).round().max(20.0);
                for (idx, candidate) in candidates
                    .iter()
                    .take(PALETTE_MAX_VISIBLE_CANDIDATES)
                    .enumerate()
                {
                    let (row_rect, resp) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), row_h),
                        egui::Sense::click(),
                    );
                    let selected = idx == self.palette_selected;
                    list::paint_table_row_background(
                        ui,
                        row_rect,
                        selected,
                        resp.hovered(),
                        colors,
                    );
                    let inner = row_rect.shrink2(egui::vec2(style::SPACE_SM, 0.0));
                    let cmd_font = egui::FontId::monospace(style::TEXT_META);
                    let cmd_color = colors.accent;
                    let cmd_galley = ui.fonts_mut(|f| {
                        f.layout_no_wrap(
                            candidate.slash_syntax.to_string(),
                            cmd_font.clone(),
                            cmd_color,
                        )
                    });
                    let cmd_pos = egui::pos2(
                        inner.left(),
                        inner.center().y - cmd_galley.size().y * 0.5,
                    );
                    let cmd_w = cmd_galley.size().x;
                    ui.painter().galley(cmd_pos, cmd_galley, cmd_color);

                    let title_left = (inner.left() + cmd_w + style::SPACE_MD).min(inner.right());
                    let title_font = egui::FontId::proportional(style::TEXT_META);
                    let title_color = colors.text_primary;
                    let title_galley = ui.fonts_mut(|f| {
                        f.layout_no_wrap(
                            candidate.title.to_string(),
                            title_font.clone(),
                            title_color,
                        )
                    });
                    let title_pos = egui::pos2(
                        title_left,
                        inner.center().y - title_galley.size().y * 0.5,
                    );
                    let title_w = title_galley.size().x;
                    ui.painter()
                        .with_clip_rect(inner)
                        .galley(title_pos, title_galley, title_color);

                    let subtitle_left = (title_left + title_w).min(inner.right());
                    if subtitle_left < inner.right() {
                        let subtitle_rect = egui::Rect::from_min_max(
                            egui::pos2(subtitle_left, inner.top()),
                            inner.max,
                        );
                        list::paint_table_cell_text(
                            ui,
                            subtitle_rect,
                            format!(" \u{2014} {}", candidate.subtitle),
                            title_font,
                            colors.text_secondary(colors.bg_toolbar),
                        );
                    }

                    if resp.clicked() {
                        clicked_candidate = Some(idx);
                    }
                }
            });

        if close_clicked {
            self.exit_palette();
        } else if let Some(idx) = clicked_candidate {
            self.palette_selected = idx;
            self.submit_palette();
        } else if submitted_from_widget && self.in_palette {
            self.submit_palette();
        }
    }

    pub(crate) fn palette_preview_chip<'a>(
        &self,
        candidates: &'a [PaletteCandidate],
    ) -> Option<(&'static str, &'a str, bool)> {
        if self.palette_query.trim() == "/" && self.palette_selected == 0 {
            return Some((
                "CMD",
                "Type a command or use \u{2191}\u{2193} to choose an action \u{2014} Enter to run",
                false,
            ));
        }
        let active = candidates.get(self.palette_selected)?;
        let is_ready = matches!(active.resolution, CandidateResolution::Ready(_));
        let badge_label = if is_ready { "INTENT" } else { "SLOT" };
        Some((badge_label, active.preview_chip.as_str(), is_ready))
    }

    fn draw_status_bar(&mut self, ui: &mut egui::Ui, colors: &Colors, show_inspector: bool) {
        ui.add_space(style::SPACE_XS);
        ui.separator();
        // Single non-wrapping row. Allocate a fixed STATUS_BAR_ROW_H rect so
        // both the left item-count label and the right-aligned inline hint bar
        // share the exact same vertical center and baseline without nested
        // modal top spacing or horizontal centering.
        let row_rect = egui::Rect::from_min_size(
            ui.cursor().min,
            egui::vec2(ui.available_width(), STATUS_BAR_ROW_H),
        );
        ui.allocate_rect(row_rect, egui::Sense::hover());
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(row_rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                let selected_label =
                    crate::ui::text::tabular_count(self.selected_count(), 3, style::TEXT_HINT);
                let mut count_job = egui::text::LayoutJob::default();
                count_job.append(
                    &format!(
                        "{} · selected: ",
                        crate::ui::text::format_count_plural(
                            self.visible_entry_count(),
                            "item",
                            "items"
                        )
                    ),
                    0.0,
                    egui::TextFormat {
                        font_id: egui::FontId::proportional(style::TEXT_HINT),
                        color: colors.text_dim,
                        ..Default::default()
                    },
                );
                count_job.append(
                    selected_label.text(),
                    0.0,
                    egui::TextFormat {
                        font_id: egui::FontId::monospace(style::TEXT_HINT),
                        // Keep the footer's established line box when mixing
                        // the counter font with proportional labels and hints.
                        line_height: Some(ui.fonts_mut(|fonts| {
                            fonts.row_height(&egui::FontId::proportional(style::TEXT_HINT))
                        })),
                        color: colors.text_dim,
                        ..Default::default()
                    },
                );
                let item_count_resp = ui.label(count_job);
                if show_inspector {
                    ui.label(
                        egui::RichText::new("\u{00b7} inspector visible")
                            .size(style::TEXT_HINT)
                            .color(colors.text_dim),
                    );
                }
                // Hints are measured, not breakpointed: drop the least
                // important groups (front of the list) until the rest fit in
                // the width left over after the item-count label.
                let rem = ui.available_width();
                let all = [
                    HintGroup::new(&["/"], "actions"),
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
                };
                let hint_resp = HintBar::new(&all[start..])
                    .inline_status_bar()
                    .show_measured(ui, colors);
                #[cfg(test)]
                {
                    self.last_status_bar_layout = Some(StatusBarLayout {
                        row_rect,
                        item_count_rect: item_count_resp.rect,
                        chip_rects: hint_resp.chip_rects,
                        label_rects: hint_resp.label_rects,
                    });
                }
                #[cfg(not(test))]
                let _ = (item_count_resp, hint_resp);
            },
        );
    }

    /// Read `show_hidden` from outside the module (cross-module host tests
    /// reach `FileBrowserApp` through `AppRuntime::Builtin` + `as_any_mut`,
    /// where the private `columns` field is unreachable).
    #[cfg(test)]
    pub(crate) fn show_hidden(&self) -> bool {
        self.columns.show_hidden
    }

    /// Queue the same `OpenArtifact` a user activation (Enter / double-click)
    /// queues. The host stamps `sender_pane_id` when it drains the command.
    #[cfg(test)]
    pub(crate) fn request_open_for_test(&mut self, path: &Path) {
        self.open_file(path);
    }
}

impl App for FileBrowserApp {
    fn service_external(
        &mut self,
        wake: &std::sync::Arc<dyn crate::app::ui_mailbox::UiWake>,
    ) -> bool {
        let mut changed = self.search_session.service_external(wake);
        if changed && self.in_search {
            self.sync_search_indices();
            self.selected = self
                .selected
                .min(self.search_session.hits().len().saturating_sub(1));
        }
        if self.watch.as_ref().map(|(dir, _)| dir) != Some(&self.cwd) {
            self.watch = None;
            match crate::host::path_watch::PathWatch::new(
                &self.cwd,
                None,
                std::sync::Arc::clone(wake),
                "file_browser",
            ) {
                Ok(watch) => self.watch = Some((self.cwd.clone(), watch)),
                Err(e) => {
                    log::warn!("file_browser: cannot watch {}: {e}", self.cwd.display());
                    return changed;
                }
            }
            return changed;
        }
        match &self.watch {
            Some((_, watch)) if watch.take_changed() => {
                self.search_session.invalidate_cache();
                self.refresh_after_external_change();
                changed = true;
            }
            _ => {}
        }
        changed
    }

    #[cfg(test)]
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn type_id(&self) -> &'static str {
        "file_browser"
    }

    fn display_name(&self) -> String {
        self.cwd
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "/".to_string())
    }

    #[cfg(test)]
    fn adjust_font_size(&mut self, delta: f32) {
        self.font_size = (self.font_size + delta).clamp(FONT_SIZE_MIN, FONT_SIZE_MAX);
        log::info!("file_browser: font_size -> {}", self.font_size);
    }

    fn ui(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &AppRenderContext<'_>,
        pending_click: Option<crate::host::pane::PendingPaneClick>,
    ) {
        let colors = ctx.colors;
        if let Some(text) = self.pending_clipboard_text.take() {
            ui.ctx().copy_text(text);
        }
        if self.in_search && self.search_session.poll() {
            self.sync_search_indices();
            self.selected = self
                .selected
                .min(self.search_session.hits().len().saturating_sub(1));
        }

        egui::Frame::new()
            .fill(colors.terminal_bg)
            .inner_margin(egui::Margin::symmetric(12, 8))
            .show(ui, |ui| {
                let show_inspector = self.should_show_inspector(ui.available_width());
                self.draw_toolbar(ui, colors, pending_click);

                if self.in_palette {
                    self.draw_action_palette(ui, colors, ctx.pane_id, pending_click);
                }
                if self.in_search {
                    self.draw_search_bar(ui, colors, ctx.pane_id, pending_click);
                }

                ui.add_space(style::SPACE_XS);

                let mut navigate_to: Option<(PathBuf, bool)> = None;
                let body_height = (ui.available_height() - STATUS_BAR_H).max(0.0);

                if let Some(err) = &self.error.clone() {
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), body_height),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.colored_label(
                                colors.text_dim,
                                egui::RichText::new(err).size(self.font_size),
                            );
                        },
                    );
                } else if self.entries.is_empty() && !self.in_search {
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), body_height),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.colored_label(
                                colors.text_dim,
                                egui::RichText::new("Empty directory").size(self.font_size),
                            );
                        },
                    );
                } else if show_inspector {
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
                            Stroke::new(1.0_f32, colors.border),
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
                }

                self.draw_status_bar(ui, colors, show_inspector);

                if let Some((path, is_dir)) = navigate_to {
                    if self.in_search {
                        self.exit_search();
                    }
                    if is_dir {
                        self.navigate_into(path);
                    } else {
                        self.open_file(&path);
                    }
                }
            });

        self.draw_quick_look_modal(ui.ctx(), colors);
        self.draw_pending_operation_modal(ui.ctx(), colors);
        self.draw_rename_modal(ui.ctx(), colors, ctx.pane_id);
    }

    fn handle_key(
        &mut self,
        input: &crate::app::input_router::PlexiInput,
    ) -> crate::app::app_trait::KeyDisposition {
        use crate::app::app_trait::KeyDisposition;
        let mode = if self.in_palette {
            Mode::Palette
        } else if self.in_search {
            Mode::Search
        } else {
            Mode::Normal
        };
        let action = classify_key(input, self.in_search || self.in_palette);
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
                    if let Some(entry) = self.selected_entry_or_hit() {
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
            // Palette mode actions
            (Mode::Palette, Some(FileBrowserAction::Escape)) => {
                self.exit_palette();
                KeyDisposition::Consumed
            }
            (Mode::Palette, Some(FileBrowserAction::Activate)) => {
                for event in input.events() {
                    match event {
                        egui::Event::Text(t) => self.palette_query.push_str(t),
                        egui::Event::Paste(p) => self.palette_query.push_str(p),
                        _ => {}
                    }
                }
                self.submit_palette();
                KeyDisposition::Consumed
            }
            (Mode::Palette, Some(FileBrowserAction::TabComplete)) => {
                self.tab_complete_palette();
                KeyDisposition::Consumed
            }
            (Mode::Palette, Some(FileBrowserAction::SelectNext)) => {
                let count = self
                    .rank_palette_candidates()
                    .len()
                    .min(PALETTE_MAX_VISIBLE_CANDIDATES);
                if count > 0 {
                    self.palette_selected = (self.palette_selected + 1).min(count - 1);
                }
                KeyDisposition::Consumed
            }
            (Mode::Palette, Some(FileBrowserAction::SelectPrev)) => {
                self.palette_selected = self.palette_selected.saturating_sub(1);
                KeyDisposition::Consumed
            }
            (Mode::Palette, Some(FileBrowserAction::Backspace)) => {
                self.palette_query.pop();
                self.palette_selected = 0;
                KeyDisposition::Consumed
            }
            (Mode::Palette, Some(FileBrowserAction::AppendText(text))) => {
                if std::mem::take(&mut self.palette_suppress_initial_slash) && text == "/" {
                    return KeyDisposition::Consumed;
                }
                self.palette_query.push_str(&text);
                if self.palette_query.starts_with("//") {
                    self.palette_query =
                        format!("/{}", self.palette_query.trim_start_matches('/'));
                }
                self.palette_selected = 0;
                KeyDisposition::Consumed
            }
            (Mode::Palette, Some(FileBrowserAction::EnterSearch)) => {
                self.exit_palette();
                self.in_search = true;
                self.search_focus_pending = true;
                self.search_query.clear();
                self.refilter();
                log::info!("file_browser: entering search mode");
                KeyDisposition::Consumed
            }
            (Mode::Palette, Some(FileBrowserAction::NewFile)) => {
                self.open_palette("/new file ");
                KeyDisposition::Consumed
            }
            (Mode::Palette, Some(FileBrowserAction::NewFolder)) => {
                self.open_palette("/new folder ");
                KeyDisposition::Consumed
            }
            (Mode::Palette, _) => KeyDisposition::Consumed,
            // Escape: closes browser in normal mode, exits search in search mode
            (Mode::Search, Some(FileBrowserAction::Escape)) => {
                self.exit_search();
                KeyDisposition::Consumed
            }
            (Mode::Normal, Some(FileBrowserAction::Escape)) => {
                self.should_close = true;
                KeyDisposition::Consumed
            }
            (Mode::Normal, Some(FileBrowserAction::CdTerminalAndClose)) => {
                self.cd_terminal_and_close();
                KeyDisposition::Consumed
            }
            // NavigateUp (← / H): global — no modal conflict
            (Mode::Normal, Some(FileBrowserAction::NavigateUp)) => {
                self.navigate_up();
                KeyDisposition::Consumed
            }
            // Backspace: delete char in search, navigate up elsewhere
            (Mode::Search, Some(FileBrowserAction::Backspace)) => {
                self.search_query.pop();
                self.refilter();
                KeyDisposition::Consumed
            }
            (Mode::Normal, Some(FileBrowserAction::Backspace)) => {
                self.navigate_up();
                KeyDisposition::Consumed
            }
            // Search mode actions
            (Mode::Search, Some(FileBrowserAction::Activate)) => {
                if let Some(hit) = self.search_session.hits().get(self.selected).cloned() {
                    if hit.is_dir {
                        self.exit_search();
                        self.navigate_into(hit.path);
                    } else {
                        self.open_file(&hit.path);
                        self.exit_search();
                    }
                }
                KeyDisposition::Consumed
            }
            (Mode::Search, Some(FileBrowserAction::SelectNext)) => {
                let last = self.search_session.hits().len().saturating_sub(1);
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
            (Mode::Search, Some(FileBrowserAction::EnterPaletteCommand)) => {
                self.open_palette("");
                KeyDisposition::Consumed
            }
            (Mode::Search, Some(FileBrowserAction::NewFile)) => {
                self.open_palette("/new file ");
                KeyDisposition::Consumed
            }
            (Mode::Search, Some(FileBrowserAction::NewFolder)) => {
                self.open_palette("/new folder ");
                KeyDisposition::Consumed
            }
            // Search mode consumes all unhandled input
            (Mode::Search, _) => KeyDisposition::Consumed,
            // Normal mode actions
            (Mode::Normal, Some(FileBrowserAction::EnterSearch)) => {
                self.in_search = true;
                self.search_focus_pending = true;
                self.search_query.clear();
                self.refilter();
                log::info!("file_browser: entering search mode");
                KeyDisposition::Consumed
            }
            (Mode::Normal, Some(FileBrowserAction::EnterPaletteSlash)) => {
                self.open_palette("/");
                KeyDisposition::Consumed
            }
            (Mode::Normal, Some(FileBrowserAction::EnterPaletteCommand)) => {
                self.open_palette("");
                KeyDisposition::Consumed
            }
            (Mode::Normal, Some(FileBrowserAction::SelectNext)) if !self.entries.is_empty() => {
                let last = self.entries.len().saturating_sub(1);
                let next = (self.selected + 1).min(last);
                if input.modifiers().shift {
                    self.extend_selection_to(next);
                } else {
                    self.set_single_selection(next);
                }
                self.pending_scroll = true;
                KeyDisposition::Consumed
            }
            (Mode::Normal, Some(FileBrowserAction::SelectPrev)) if !self.entries.is_empty() => {
                let prev = self.selected.saturating_sub(1);
                if input.modifiers().shift {
                    self.extend_selection_to(prev);
                } else {
                    self.set_single_selection(prev);
                }
                self.pending_scroll = true;
                KeyDisposition::Consumed
            }
            (Mode::Normal, Some(FileBrowserAction::SelectFirst)) if !self.entries.is_empty() => {
                self.set_single_selection(0);
                self.pending_scroll = true;
                KeyDisposition::Consumed
            }
            (Mode::Normal, Some(FileBrowserAction::SelectLast)) if !self.entries.is_empty() => {
                self.set_single_selection(self.entries.len().saturating_sub(1));
                self.pending_scroll = true;
                KeyDisposition::Consumed
            }
            (Mode::Normal, Some(FileBrowserAction::PageDown)) if !self.entries.is_empty() => {
                let last = self.entries.len().saturating_sub(1);
                self.set_single_selection((self.selected + 10).min(last));
                self.pending_scroll = true;
                KeyDisposition::Consumed
            }
            (Mode::Normal, Some(FileBrowserAction::PageUp)) if !self.entries.is_empty() => {
                self.set_single_selection(self.selected.saturating_sub(10));
                self.pending_scroll = true;
                KeyDisposition::Consumed
            }
            (Mode::Normal, Some(FileBrowserAction::Activate)) if !self.entries.is_empty() => {
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
            (Mode::Normal, Some(FileBrowserAction::ToggleQuickLook))
                if !self.entries.is_empty() =>
            {
                self.toggle_quick_look();
                KeyDisposition::Consumed
            }
            (Mode::Normal, Some(FileBrowserAction::ToggleHidden)) => {
                self.columns.show_hidden = !self.columns.show_hidden;
                self.refresh_preserving_filter();
                log::info!(
                    "file_browser: show_hidden toggled to {}",
                    self.columns.show_hidden
                );
                KeyDisposition::Consumed
            }
            (Mode::Normal, Some(FileBrowserAction::Refresh)) => {
                self.refresh();
                KeyDisposition::Consumed
            }
            (Mode::Normal, Some(FileBrowserAction::SelectAll)) if !self.entries.is_empty() => {
                self.select_all_visible();
                KeyDisposition::Consumed
            }
            (Mode::Normal, Some(FileBrowserAction::NewFile)) => {
                self.open_palette("/new file ");
                KeyDisposition::Consumed
            }
            (Mode::Normal, Some(FileBrowserAction::NewFolder)) => {
                self.open_palette("/new folder ");
                KeyDisposition::Consumed
            }
            (Mode::Normal, Some(FileBrowserAction::Rename)) if !self.entries.is_empty() => {
                self.open_rename_modal();
                KeyDisposition::Consumed
            }
            (Mode::Normal, Some(FileBrowserAction::Copy)) if !self.entries.is_empty() => {
                self.copy_selected(FileOperation::Copy);
                KeyDisposition::Consumed
            }
            (Mode::Normal, Some(FileBrowserAction::Cut)) if !self.entries.is_empty() => {
                self.copy_selected(FileOperation::Cut);
                KeyDisposition::Consumed
            }
            (Mode::Normal, Some(FileBrowserAction::Paste)) => {
                if let Err(err) = self.paste_into_current_dir() {
                    self.error = Some(err);
                }
                KeyDisposition::Consumed
            }
            (Mode::Normal, Some(FileBrowserAction::MoveToTrash)) if !self.entries.is_empty() => {
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
            (Mode::Normal, Some(FileBrowserAction::CopyPath)) => {
                self.copy_selected_path();
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

    fn submit_on_focused_text_input_enter(&self) -> bool {
        (self.in_search || self.in_palette)
            && self.rename_path.is_none()
            && self.pending_operation.is_none()
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
            "font_size": self.font_size,
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
        if let Some(size) = state["font_size"].as_f64() {
            self.font_size = (size as f32).clamp(FONT_SIZE_MIN, FONT_SIZE_MAX);
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
        let _ = ctx.run_ui(raw, |ui| {
            let ctx = ui.ctx();
            let input = crate::app::input_router::PlexiInput::take_from(ctx);
            consumed = app.handle_key(&input) == KeyDisposition::Consumed;
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
    fn details_table_flexes_name_and_progressively_hides_trailing_columns_when_narrow() {
        let (app, _dir) = make_populated_dir_app();
        let wide_rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(720.0, 24.0));
        let wide_cols = app.details_columns(wide_rect);
        let wide_ids: Vec<ColumnId> = wide_cols.iter().map(|(c, _)| c.id).collect();
        assert_eq!(
            wide_ids,
            vec![ColumnId::Name, ColumnId::Size, ColumnId::Modified]
        );
        let total_wide_w: f32 = wide_cols.iter().map(|(_, r)| r.width()).sum();
        assert!((total_wide_w - 720.0).abs() < 1.0);

        let narrow_rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(200.0, 24.0));
        let narrow_cols = app.details_columns(narrow_rect);
        let narrow_ids: Vec<ColumnId> = narrow_cols.iter().map(|(c, _)| c.id).collect();
        assert_eq!(narrow_ids, vec![ColumnId::Name]);
        assert!((narrow_cols[0].1.width() - 200.0).abs() < 1.0);
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
        let cd_cwd = app
            .take_pending_commands()
            .into_iter()
            .find_map(|c| match c {
                AppCommand::CdRequest { cwd, .. } => Some(cwd),
                _ => None,
            });
        assert_eq!(
            cd_cwd.as_deref(),
            Some(dir.path().to_string_lossy().as_ref())
        );
    }

    #[test]
    fn t_in_search_mode_is_not_a_handoff() {
        let (mut app, _dir) = make_populated_dir_app();
        run_handle_key(
            &mut app,
            vec![key_event(
                Key::F,
                Modifiers {
                    command: true,
                    ..Default::default()
                },
            )],
        );
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
    fn normal_mode_slash_enters_palette_and_cmd_f_enters_search() {
        let (mut app, _dir) = make_populated_dir_app();
        run_handle_key(&mut app, vec![key_event(Key::Slash, Modifiers::default())]);
        assert!(app.in_palette);
        assert_eq!(app.palette_query, "/");
        run_handle_key(&mut app, vec![key_event(Key::Escape, Modifiers::default())]);
        assert!(!app.in_palette);

        run_handle_key(
            &mut app,
            vec![key_event(
                Key::F,
                Modifiers {
                    command: true,
                    ..Default::default()
                },
            )],
        );
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
        let _ = ctx.run_ui(RawInput::default(), |ui| {
            let ctx = ui.ctx();
            let input = crate::app::input_router::PlexiInput::take_from(ctx);
            assert!(!super::key_pressed_no_repeat(&input, Key::ArrowRight));
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
        let _ = ctx.run_ui(raw, |ui| {
            let ctx = ui.ctx();
            let input = crate::app::input_router::PlexiInput::take_from(ctx);
            assert!(super::key_pressed_no_repeat(&input, Key::ArrowRight));
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
    fn command_d_does_not_duplicate_selected_file() {
        let (mut app, dir) = make_three_file_app();

        let consumed = run_handle_key(
            &mut app,
            vec![key_event(
                Key::D,
                Modifiers {
                    command: true,
                    ..Default::default()
                },
            )],
        );

        assert!(!consumed);
        assert!(!dir.path().join("alpha copy.txt").exists());
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
    fn confirmed_trash_moves_selected_file_to_the_system_trash() {
        let dir = tempfile::tempdir().unwrap();
        // Unique name so a leftover in the real Trash can never be mistaken
        // for this run, and so it can be cleared afterwards.
        let name = format!("plexi-0786-trash-{}.txt", crate::platform::clock::now_nanos());
        fs::write(dir.path().join(&name), "x").unwrap();
        let mut app = FileBrowserApp::new(dir.path().to_path_buf());
        app.select_path(&dir.path().join(&name));

        app.request_move_selected_to_trash();
        assert!(app.pending_operation.is_some());
        app.confirm_pending_operation().expect("confirm trash");

        assert!(!dir.path().join(&name).exists(), "the item left the directory");
        assert!(
            !dir.path().join(".Trash").exists(),
            "no in-project .Trash folder is created"
        );
        assert!(app.pending_operation.is_none());
        #[cfg(target_os = "macos")]
        if let Some(home) = dirs::home_dir() {
            let _ = fs::remove_file(home.join(".Trash").join(&name));
        }
    }

    #[test]
    fn host_external_io_refreshes_browser_with_experiments_off() {
        let _experiments = crate::release::set_test_experiments(&[]);
        let (mut app, dir) = make_three_file_app();
        app.select_path(&dir.path().join("bravo.txt"));
        let before = app.entries.len();
        let mut harness = crate::testing::HostHarness::new();
        harness.app.open_builtin_app_pane(
            Box::new(app),
            crate::app::permissions::AppPermissions::builtin(),
            dir.path().to_path_buf(),
            None,
            Some("split_h"),
            None,
        );
        fn browser(harness: &mut crate::testing::HostHarness) -> &mut FileBrowserApp {
            harness
                .app
                .windows
                .iter_mut()
                .flat_map(|w| w.panes.values_mut())
                .find_map(|pane| {
                    let crate::host::pane::AppRuntime::Builtin(app) = &mut pane.as_app_mut()?.runtime
                    else {
                        return None;
                    };
                    app.as_any_mut().downcast_mut::<FileBrowserApp>()
                })
                .unwrap()
        }
        harness.hidden_frame();
        std::thread::sleep(crate::testing::load_aware_timeout(
            std::time::Duration::from_millis(300),
        ));
        fs::write(dir.path().join("0-from-agent.txt"), "hi").unwrap();
        let deadline = std::time::Instant::now()
            + crate::testing::load_aware_timeout(std::time::Duration::from_secs(5));
        while std::time::Instant::now() < deadline {
            harness.hidden_frame();
            if browser(&mut harness).entries.len() != before {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        let app = browser(&mut harness);
        assert_eq!(app.entries.len(), before + 1);
        assert!(app.entries.iter().any(|e| e.name == "0-from-agent.txt"));
        assert_eq!(
            app.selected_entry().map(|e| e.name.as_str()),
            Some("bravo.txt")
        );
    }

    #[test]
    fn listing_picks_up_files_created_externally() {
        let (mut app, dir) = make_three_file_app();
        let wake: std::sync::Arc<dyn crate::app::ui_mailbox::UiWake> =
            std::sync::Arc::new(crate::app::ui_mailbox::RecordingWake::new());
        assert!(!app.service_external(&wake), "arming changes nothing");
        std::thread::sleep(crate::testing::load_aware_timeout(std::time::Duration::from_millis(300)));
        let before = app.entries.len();
        app.select_path(&dir.path().join("bravo.txt"));
        fs::write(dir.path().join("0-from-agent.txt"), "hi").unwrap();

        let deadline =
            std::time::Instant::now() + crate::testing::load_aware_timeout(std::time::Duration::from_secs(5));
        while std::time::Instant::now() < deadline && app.entries.len() == before {
            app.service_external(&wake);
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        assert_eq!(app.entries.len(), before + 1);
        assert!(app.entries.iter().any(|e| e.name == "0-from-agent.txt"));
        assert_eq!(
            app.selected_entry().map(|e| e.name.as_str()),
            Some("bravo.txt"),
            "selection follows the path, not the index"
        );
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

    /// Regression for stint 0351: `plexi pane key <id> enter` must drive the
    /// same `handle_key` → `open_file` path a physical Enter press takes. In
    /// prod, `open_file` hands the path to the host's `OpenArtifact` resolver
    /// (`open_file_in_app`, #2283) — that resolver has its own coverage; this
    /// test proves the CLI key-string vocabulary reaches it for a selected
    /// media file.
    #[test]
    fn cli_key_string_enter_opens_selected_media_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("clip.mp4"), b"not a real video").expect("write");
        let mut app = FileBrowserApp::new(dir.path().to_path_buf());
        assert!(!app.entries[0].is_dir, "fixture must select the media file");

        let raw = crate::app::key_str_to_egui_raw_input("enter")
            .expect("CLI key string 'enter' must map to an egui event");
        let ctx = egui::Context::default();
        let mut consumed = false;
        let _ = ctx.run_ui(raw, |ui| {
            let ctx = ui.ctx();
            let input = crate::app::input_router::PlexiInput::take_from(ctx);
            consumed = app.handle_key(&input) == crate::app::app_trait::KeyDisposition::Consumed;
        });

        assert!(consumed, "injected Enter must be consumed by the app");
        assert_eq!(
            app.opened_files.len(),
            1,
            "injected Enter must reach open_file for the selected media file"
        );
        assert!(app.opened_files[0].ends_with("clip.mp4"));
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
        let (mut app, dir) = make_mixed_dir_app();
        app.select_path(&dir.path().join("subdir"));
        assert!(
            app.selected_entry().is_some_and(|e| e.is_dir),
            "selected entry must be the subdirectory"
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
        // enter search mode via Cmd+F
        run_handle_key(
            &mut app,
            vec![key_event(
                Key::F,
                Modifiers {
                    command: true,
                    ..Default::default()
                },
            )],
        );
        assert!(app.in_search, "Cmd+F must enter search mode");
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
        app.columns.show_hidden = true;

        let state = app.serialize_state().expect("state");
        let mut restored = FileBrowserApp::new(app.cwd.clone());
        restored.restore_state(&state);

        assert_eq!(restored.columns.sort.column, ColumnId::Size);
        assert_eq!(restored.columns.sort.direction, SortDirection::Desc);
        assert!(!restored.columns.folders_on_top);
        assert!(restored.columns.show_hidden);
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

    /// Both dotfiles and dot-directories stay hidden until `show_hidden` is enabled.
    #[test]
    fn show_hidden_toggle_filters_all_dot_entries_including_dot_directories() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("visible.txt"), b"v").expect("write visible");
        std::fs::write(dir.path().join(".hidden"), b"h").expect("write hidden");
        std::fs::create_dir(dir.path().join(".config")).expect("create dot directory");
        let mut app = FileBrowserApp::new(dir.path().to_path_buf());

        assert!(!app.columns.show_hidden, "hidden files are off by default");
        assert!(app.entries.iter().any(|e| e.name == "visible.txt"));
        assert!(
            !app.entries.iter().any(|e| e.name == ".hidden"),
            "dotfile must be filtered out while show_hidden is off"
        );
        assert!(
            !app.entries.iter().any(|e| e.name == ".config"),
            "dot directory must also be filtered out while show_hidden is off"
        );

        let consumed = run_handle_key(
            &mut app,
            vec![key_event(
                Key::Period,
                Modifiers {
                    command: true,
                    shift: true,
                    ..Default::default()
                },
            )],
        );
        assert!(consumed, "Cmd+Shift+. must toggle show_hidden");
        assert!(app.columns.show_hidden);
        assert!(app.entries.iter().any(|e| e.name == "visible.txt"));
        assert!(
            app.entries.iter().any(|e| e.name == ".hidden"),
            "dotfile must appear once show_hidden is on"
        );
        assert!(
            app.entries.iter().any(|e| e.name == ".config" && e.is_dir),
            "dot directory must appear once show_hidden is on"
        );

        run_handle_key(
            &mut app,
            vec![key_event(
                Key::Period,
                Modifiers {
                    command: true,
                    shift: true,
                    ..Default::default()
                },
            )],
        );
        assert!(!app.columns.show_hidden);
        assert!(
            !app.entries.iter().any(|e| e.name == ".hidden"),
            "dotfile must be filtered again once show_hidden is toggled back off"
        );
        assert!(
            !app.entries.iter().any(|e| e.name == ".config"),
            "dot directory must be filtered again when show_hidden is toggled back off"
        );
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
    fn details_columns_keep_enabled_columns_reachable_when_wide() {
        let (mut app, _dir) = make_populated_dir_app();
        for id in ColumnId::ALL {
            app.columns.set_column_visible(id, true);
        }
        let rect = egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(960.0, DETAILS_HEADER_H),
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

    #[test]
    fn adjust_font_size_scales_metrics_clamps_and_persists_in_state() {
        let (mut app, _dir) = make_populated_dir_app();
        let default_row_h = app.row_height();
        let default_header_h = app.header_height();
        assert_eq!(app.font_size, style::TEXT_HINT);

        app.adjust_font_size(4.0);
        assert_eq!(app.font_size, style::TEXT_HINT + 4.0);
        assert!(app.row_height() > default_row_h);
        assert!(app.header_height() > default_header_h);

        let state = app.serialize_state().expect("serialize");
        let mut restored = FileBrowserApp::new(app.cwd.clone());
        restored.restore_state(&state);
        assert_eq!(restored.font_size, style::TEXT_HINT + 4.0);

        app.adjust_font_size(-100.0);
        assert_eq!(app.font_size, FONT_SIZE_MIN);
        app.adjust_font_size(100.0);
        assert_eq!(app.font_size, FONT_SIZE_MAX);
    }

    #[test]
    fn empty_dir_supports_new_folder_and_paste() {
        let (mut src_app, _src_dir) = make_three_file_app();
        src_app.copy_selected(FileOperation::Copy);
        let clipboard = src_app.operation_clipboard.clone();

        let (mut empty_app, empty_dir) = make_empty_dir_app();
        empty_app.operation_clipboard = clipboard;
        assert!(empty_app.entries.is_empty());

        let consumed_paste = run_handle_key(
            &mut empty_app,
            vec![key_event(
                Key::V,
                Modifiers {
                    command: true,
                    ..Default::default()
                },
            )],
        );
        assert!(consumed_paste, "Cmd+V must work in an empty directory");
        assert!(empty_dir.path().join("alpha.txt").exists());

        let (mut empty_app2, empty_dir2) = make_empty_dir_app();
        let consumed_mkdir = run_handle_key(
            &mut empty_app2,
            vec![key_event(
                Key::N,
                Modifiers {
                    command: true,
                    shift: true,
                    ..Default::default()
                },
            )],
        );
        assert!(
            consumed_mkdir,
            "Cmd+Shift+N must work in an empty directory"
        );
        assert!(empty_app2.in_palette);
        assert_eq!(empty_app2.palette_query, "/new folder ");
        assert!(
            !empty_dir2.path().join("Untitled Folder").exists(),
            "Cmd+Shift+N must prompt for folder name without creating Untitled Folder on disk first"
        );
        run_handle_key(
            &mut empty_app2,
            vec![
                Event::Text("created_dir".to_string()),
                key_event(Key::Enter, Modifiers::default()),
            ],
        );
        assert!(empty_dir2.path().join("created_dir").is_dir());
    }

    #[test]
    fn reveal_and_open_with_default_emit_os_modes_not_open_in_pane() {
        let (mut app, dir) = make_file_only_dir_app();
        app.take_pending_commands();

        let consumed_reveal = run_handle_key(
            &mut app,
            vec![key_event(
                Key::Enter,
                Modifiers {
                    command: true,
                    ..Default::default()
                },
            )],
        );
        assert!(consumed_reveal);
        let cmds = app.take_pending_commands();
        assert_eq!(cmds.len(), 1);
        assert!(matches!(
            &cmds[0],
            AppCommand::OpenArtifact {
                path,
                mode: crate::protocol::ArtifactOpenMode::RevealInFinder,
                ..
            } if path == &dir.path().join("readme.txt").display().to_string()
        ));

        let consumed_open = run_handle_key(
            &mut app,
            vec![key_event(
                Key::O,
                Modifiers {
                    command: true,
                    ..Default::default()
                },
            )],
        );
        assert!(consumed_open);
        let cmds = app.take_pending_commands();
        assert_eq!(cmds.len(), 1);
        assert!(matches!(
            &cmds[0],
            AppCommand::OpenArtifact {
                path,
                mode: crate::protocol::ArtifactOpenMode::OpenWithDefault,
                ..
            } if path == &dir.path().join("readme.txt").display().to_string()
        ));
    }

    #[test]
    fn copy_and_paste_preserves_extension_before_copy_suffix() {
        let (mut app, dir) = make_three_file_app();
        app.copy_selected(FileOperation::Copy);

        let first = app.paste_into_current_dir().expect("first paste");
        assert_eq!(first, vec![dir.path().join("alpha 1.txt")]);

        let second = app.paste_into_current_dir().expect("second paste");
        assert_eq!(second, vec![dir.path().join("alpha 2.txt")]);
    }

    #[test]
    fn r_and_f2_open_rename_modal_and_cmd_shift_r_refreshes() {
        let (mut app, dir) = make_three_file_app();
        let consumed = run_handle_key(&mut app, vec![key_event(Key::R, Modifiers::default())]);
        assert!(consumed);
        assert_eq!(app.rename_path, Some(dir.path().join("alpha.txt")));
        app.cancel_rename_modal();

        let consumed_f2 = run_handle_key(&mut app, vec![key_event(Key::F2, Modifiers::default())]);
        assert!(consumed_f2);
        assert_eq!(app.rename_path, Some(dir.path().join("alpha.txt")));
        app.cancel_rename_modal();

        std::fs::write(dir.path().join("delta.txt"), b"d").expect("write delta");
        let consumed_refresh = run_handle_key(
            &mut app,
            vec![key_event(
                Key::R,
                Modifiers {
                    command: true,
                    shift: true,
                    ..Default::default()
                },
            )],
        );
        assert!(consumed_refresh);
        assert!(app.entries.iter().any(|e| e.name == "delta.txt"));
    }

    #[test]
    fn y_and_cmd_shift_c_copy_selected_path() {
        let (mut app, dir) = make_three_file_app();
        let expected = dir.path().join("alpha.txt").display().to_string();

        let consumed_y = run_handle_key(&mut app, vec![key_event(Key::Y, Modifiers::default())]);
        assert!(consumed_y);
        assert_eq!(app.pending_clipboard_text.take(), Some(expected.clone()));

        let consumed_cmd_shift_c = run_handle_key(
            &mut app,
            vec![key_event(
                Key::C,
                Modifiers {
                    command: true,
                    shift: true,
                    ..Default::default()
                },
            )],
        );
        assert!(consumed_cmd_shift_c);
        assert_eq!(app.pending_clipboard_text.take(), Some(expected));
    }

    #[test]
    fn breadcrumb_segments_split_ancestors_in_order() {
        let dir = tempfile::tempdir().expect("tempdir");
        let nested = dir.path().join("alpha").join("beta");
        std::fs::create_dir_all(&nested).expect("mkdir");
        let segments = FileBrowserApp::breadcrumb_segments(&nested);
        assert!(segments.len() >= 3);
        assert_eq!(segments.last().unwrap().0, "beta");
        assert_eq!(segments.last().unwrap().1, nested);
        assert_eq!(segments[segments.len() - 2].0, "alpha");
        assert_eq!(segments[segments.len() - 2].1, dir.path().join("alpha"));
    }

    #[test]
    fn cmd_f_and_slash_enter_search_and_claim_pane_text_surface() {
        let (mut app, _dir) = make_three_file_app();
        let consumed = run_handle_key(
            &mut app,
            vec![key_event(
                Key::F,
                Modifiers {
                    command: true,
                    ..Default::default()
                },
            )],
        );
        assert!(consumed, "Cmd+F must enter search mode");
        assert!(app.in_search);
        assert!(app.search_focus_pending);
        assert!(app.submit_on_focused_text_input_enter());

        // Render one UI pass with a bound pane_id and verify SurfaceKey::Pane(pane_id) is registered
        // as default and claimed in the frame's SurfaceRegistry.
        let ctx = egui::Context::default();
        let colors = Colors::from_config(&crate::config::ThemeConfig::default());
        let _ = ctx.run_ui(RawInput::default(), |ui| {
            app.draw_search_bar(ui, &colors, 42, None);
        });
        let registry = crate::ui::focus::drain_text_surfaces(&ctx);
        let key = crate::ui::focus::SurfaceKey::Pane(42);
        assert!(
            registry.default_for(key).is_some() && registry.claim_for(key).is_some(),
            "SearchBar must register its TextInput under SurfaceKey::Pane(pane_id) and claim focus"
        );
    }

    #[test]
    fn search_mode_supports_paste_and_backspace_without_parent_navigation() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sub = dir.path().join("child");
        std::fs::create_dir_all(&sub).expect("mkdir child");
        std::fs::write(sub.join("alpha.txt"), b"hello").expect("write alpha");
        std::fs::write(dir.path().join("parent_file.txt"), b"parent").expect("write parent");

        let mut app = FileBrowserApp::new(sub.clone());
        // Seed file operation clipboard to verify Cmd+V / Paste in search mode does NOT paste files.
        app.operation_clipboard = Some(FileOperationClipboard {
            operation: FileOperation::Copy,
            paths: vec![dir.path().join("parent_file.txt")],
        });

        run_handle_key(
            &mut app,
            vec![key_event(
                Key::F,
                Modifiers {
                    command: true,
                    ..Default::default()
                },
            )],
        );
        assert!(app.in_search);

        let pasted = run_handle_key(&mut app, vec![Event::Paste("alp".to_string())]);
        assert!(pasted, "Event::Paste in search mode must append to search_query");
        assert_eq!(app.search_query, "alp");
        assert!(
            !sub.join("parent_file.txt").exists(),
            "pasting into search query must not trigger file paste_into_current_dir"
        );

        // Backspace while search_query is non-empty deletes the trailing character and stays in sub.
        let backspaced = run_handle_key(&mut app, vec![key_event(Key::Backspace, Modifiers::default())]);
        assert!(backspaced);
        assert_eq!(app.search_query, "al");
        assert_eq!(app.cwd, sub, "Backspace in search mode must not navigate to parent directory");

        // Even when search_query is emptied, Backspace in search mode must not navigate to parent.
        app.search_query.clear();
        app.refilter();
        let backspaced_empty = run_handle_key(
            &mut app,
            vec![key_event(Key::Backspace, Modifiers::default())],
        );
        assert!(backspaced_empty);
        assert_eq!(
            app.cwd, sub,
            "Backspace on empty search query must stay in search mode and never navigate up"
        );
    }

    #[test]
    fn deep_search_discovers_subdirectory_paths_and_file_contents_and_activates_them() {
        let dir = tempfile::tempdir().expect("tempdir");
        let nested_dir = dir.path().join("sub").join("nested_target");
        std::fs::create_dir_all(&nested_dir).expect("mkdir nested");
        let nested_file = nested_dir.join("deep_widget.rs");
        std::fs::write(&nested_file, b"pub struct DeepWidget;\n").expect("write nested_file");
        let docs_dir = dir.path().join("docs");
        std::fs::create_dir_all(&docs_dir).expect("mkdir docs");
        let content_file = docs_dir.join("architecture.md");
        std::fs::write(
            &content_file,
            b"# Architecture\nUniqueNeedleToken lives inside this markdown body.\n",
        )
        .expect("write content_file");

        let mut app = FileBrowserApp::new(dir.path().to_path_buf());
        let wake: std::sync::Arc<dyn crate::app::ui_mailbox::UiWake> =
            std::sync::Arc::new(crate::app::ui_mailbox::RecordingWake::new());

        // 1. Enter search and query a nested directory ("nested_target").
        app.in_search = true;
        app.search_query = "nested_target".to_string();
        app.refilter();

        let deadline = std::time::Instant::now()
            + crate::testing::load_aware_timeout(std::time::Duration::from_secs(5));
        while std::time::Instant::now() < deadline {
            let _ = app.service_external(&wake);
            if app.search_session.hits().iter().any(|h| h.path == nested_dir)
                && !app.search_session.is_scanning()
            {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(15));
        }
        let dir_idx = app
            .search_session
            .hits()
            .iter()
            .position(|h| h.path == nested_dir && h.is_dir)
            .expect("deep search must find nested_target directory");
        app.selected = dir_idx;

        // Pressing Enter on a directory search hit navigates cwd into that directory and exits search.
        let entered_dir = run_handle_key(&mut app, vec![key_event(Key::Enter, Modifiers::default())]);
        assert!(entered_dir);
        assert!(!app.in_search, "activating directory result must exit search mode");
        assert_eq!(
            app.cwd, nested_dir,
            "activating directory result must navigate cwd into that directory"
        );

        // Navigate back to root and test deep content search ("UniqueNeedleToken").
        app.navigate_into(dir.path().to_path_buf());
        app.in_search = true;
        app.search_query = "UniqueNeedleToken".to_string();
        app.refilter();

        let deadline2 = std::time::Instant::now()
            + crate::testing::load_aware_timeout(std::time::Duration::from_secs(5));
        while std::time::Instant::now() < deadline2 {
            let _ = app.service_external(&wake);
            if app
                .search_session
                .hits()
                .iter()
                .any(|h| h.path == content_file && h.kind.content_snippet().is_some())
                && !app.search_session.is_scanning()
            {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(15));
        }
        let hit_idx = app
            .search_session
            .hits()
            .iter()
            .position(|h| h.path == content_file)
            .expect("deep content search must find UniqueNeedleToken inside docs/architecture.md");
        let snippet = app.search_session.hits()[hit_idx]
            .kind
            .content_snippet()
            .expect("content hit must include line snippet");
        assert_eq!(snippet.line_number, 2);
        assert!(snippet.line_preview.contains("UniqueNeedleToken"));

        app.selected = hit_idx;
        let entered_file =
            run_handle_key(&mut app, vec![key_event(Key::Enter, Modifiers::default())]);
        assert!(entered_file);
        assert!(!app.in_search, "activating file/content result must exit search mode");
        assert_eq!(app.opened_files.last(), Some(&content_file));
    }

    #[test]
    fn status_bar_hints_align_on_item_count_baseline_at_wide_and_narrow_widths() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("alpha.rs"), b"fn main() {}\n").expect("write alpha.rs");
        std::fs::write(dir.path().join("beta.toml"), b"[package]\n").expect("write beta.toml");

        let colors = Colors::from_config(&crate::config::ThemeConfig::default());
        let ctx = egui::Context::default();
        crate::ui::theme::setup_fonts(&ctx);

        for (w, h) in [(1100.0_f32, 720.0_f32), (640.0_f32, 520.0_f32)] {
            let mut app = FileBrowserApp::new(dir.path().to_path_buf());
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w, h))),
                ..Default::default()
            };
            let _ = ctx.run_ui(raw, |ui| {
                egui::CentralPanel::default().show_inside(ui, |ui| {
                    let render_ctx = AppRenderContext {
                        colors: &colors,
                        pane_id: 1,
                    };
                    app.ui(ui, &render_ctx, None);
                });
            });

            let layout = app
                .last_status_bar_layout
                .as_ref()
                .expect("status bar layout recorded");
            assert!(
                !layout.chip_rects.is_empty() && !layout.label_rects.is_empty(),
                "status bar must render key chips and labels at {w}x{h}"
            );
            for label_rect in &layout.label_rects {
                assert!(
                    (label_rect.center().y - layout.item_count_rect.center().y).abs() <= 0.5,
                    "hint label center y ({}) must align with item count center y ({}) at {w}x{h}",
                    label_rect.center().y,
                    layout.item_count_rect.center().y,
                );
                assert!(
                    (label_rect.max.y - layout.item_count_rect.max.y).abs() <= 0.5,
                    "hint label baseline ({}) must align with item count baseline ({}) at {w}x{h}",
                    label_rect.max.y,
                    layout.item_count_rect.max.y,
                );
            }
            for chip_rect in &layout.chip_rects {
                assert!(
                    (chip_rect.center().y - layout.item_count_rect.center().y).abs() <= 0.5,
                    "key chip center y ({}) must align with item count center y ({}) at {w}x{h}",
                    chip_rect.center().y,
                    layout.item_count_rect.center().y,
                );
            }
            let first_chip = layout.chip_rects.first().unwrap();
            let last_label = layout.label_rects.last().unwrap();
            assert!(
                first_chip.min.x >= layout.item_count_rect.max.x + style::SPACE_MD,
                "first hint chip ({}) must not collide with item count label ({}) at {w}x{h}",
                first_chip.min.x,
                layout.item_count_rect.max.x,
            );
            assert!(
                (layout.row_rect.max.x - last_label.max.x).abs() <= 2.0,
                "rightmost hint label ({}) must sit flush with status bar right edge ({}) at {w}x{h}",
                last_label.max.x,
                layout.row_rect.max.x,
            );
        }
    }

    #[test]
    fn parse_file_explorer_intent_resolves_slash_and_natural_language_phrases() {
        let dir = tempfile::tempdir().expect("tempdir");
        let main_rs = dir.path().join("main.rs");
        let docs_dir = dir.path().join("docs");
        std::fs::write(&main_rs, b"fn main() {}\n").expect("write main.rs");
        std::fs::create_dir(&docs_dir).expect("mkdir docs");

        let mut app = FileBrowserApp::new(dir.path().to_path_buf());
        // Select main.rs explicitly for selection-contextual phrases like "rename to main_v2.rs".
        let main_idx = app
            .entries
            .iter()
            .position(|e| e.path == main_rs)
            .expect("main.rs entry");
        app.selected = main_idx;

        let selected_paths = app.selected_paths();
        let ctx = app.intent_context(&selected_paths);

        assert_eq!(
            parse_file_explorer_intent("/hidden", &ctx),
            Some(FileExplorerIntent::ToggleHidden { enabled: None })
        );
        assert_eq!(
            parse_file_explorer_intent("/new file notes.md", &ctx),
            Some(FileExplorerIntent::CreateFile {
                name: "notes.md".to_string(),
                open_in_editor: true,
            })
        );
        assert_eq!(
            parse_file_explorer_intent("/new folder specs", &ctx),
            Some(FileExplorerIntent::CreateFolder {
                name: "specs".to_string(),
            })
        );
        assert_eq!(
            parse_file_explorer_intent("rename to main_v2.rs", &ctx),
            Some(FileExplorerIntent::Rename {
                target: main_rs.clone(),
                new_name: "main_v2.rs".to_string(),
            })
        );
        assert_eq!(
            parse_file_explorer_intent("/sort size asc", &ctx),
            Some(FileExplorerIntent::SortBy {
                column: ColumnId::Size,
                direction: SortDirection::Asc,
            })
        );
        assert_eq!(
            parse_file_explorer_intent("show hidden files", &ctx),
            Some(FileExplorerIntent::ToggleHidden {
                enabled: Some(true),
            })
        );

        // Verify additional variants and serde round-trip on FileExplorerIntent.
        assert_eq!(
            parse_file_explorer_intent("/trash", &ctx),
            Some(FileExplorerIntent::MoveToTrash {
                targets: vec![main_rs.clone()],
            })
        );
        assert_eq!(
            parse_file_explorer_intent("/open", &ctx),
            Some(FileExplorerIntent::OpenInEditor {
                path: main_rs.clone(),
            })
        );
        assert_eq!(
            parse_file_explorer_intent("/default", &ctx),
            Some(FileExplorerIntent::OpenWithDefault {
                path: main_rs.clone(),
            })
        );
        assert_eq!(
            parse_file_explorer_intent("/reveal", &ctx),
            Some(FileExplorerIntent::Reveal {
                path: main_rs.clone(),
            })
        );
        assert_eq!(
            parse_file_explorer_intent("/cd", &ctx),
            Some(FileExplorerIntent::CdTerminal {
                path: dir.path().to_path_buf(),
            })
        );
        assert_eq!(
            parse_file_explorer_intent("/folders-first", &ctx),
            Some(FileExplorerIntent::ToggleFoldersFirst { enabled: None })
        );

        let intent = FileExplorerIntent::CreateFile {
            name: "hello.rs".to_string(),
            open_in_editor: true,
        };
        let encoded = serde_json::to_string(&intent).expect("serialize intent");
        let decoded: FileExplorerIntent =
            serde_json::from_str(&encoded).expect("deserialize intent");
        assert_eq!(decoded, intent);
    }

    #[test]
    fn execute_create_file_intent_creates_selects_and_queues_open_artifact() {
        let (mut app, dir) = make_three_file_app();
        app.take_pending_commands();

        app.execute_intent(FileExplorerIntent::CreateFile {
            name: "hello.rs".to_string(),
            open_in_editor: true,
        })
        .expect("CreateFile intent must execute cleanly");

        let expected_path = dir.path().join("hello.rs");
        assert!(expected_path.is_file(), "hello.rs must be created in cwd");
        assert_eq!(
            app.selected_entry().map(|e| &e.path),
            Some(&expected_path),
            "created file must be selected in FileBrowserApp"
        );
        assert_eq!(app.opened_files.last(), Some(&expected_path));

        let cmds = app.take_pending_commands();
        assert_eq!(cmds.len(), 1, "CreateFile with open_in_editor must queue OpenArtifact");
        assert!(matches!(
            &cmds[0],
            AppCommand::OpenArtifact {
                path,
                mode: crate::protocol::ArtifactOpenMode::OpenInPane,
                ..
            } if path == &expected_path.display().to_string()
        ));
    }

    #[test]
    fn palette_slash_hidden_toggles_show_hidden_and_claims_text_surface() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("visible.rs"), b"fn main() {}\n").expect("write visible");
        std::fs::write(dir.path().join(".env"), b"SECRET=1\n").expect("write hidden");

        let mut app = FileBrowserApp::new(dir.path().to_path_buf());
        assert!(!app.columns.show_hidden);

        // Press `/` to open the action palette pre-filled with `/`.
        let entered = run_handle_key(&mut app, vec![key_event(Key::Slash, Modifiers::default())]);
        assert!(entered);
        assert!(app.in_palette);
        assert_eq!(app.palette_query, "/");
        assert!(app.submit_on_focused_text_input_enter());

        // Render one UI pass of the action palette and verify SurfaceKey::Pane(pane_id) is claimed.
        let ctx = egui::Context::default();
        let colors = Colors::from_config(&crate::config::ThemeConfig::default());
        let _ = ctx.run_ui(RawInput::default(), |ui| {
            app.draw_action_palette(ui, &colors, 77, None);
        });
        let registry = crate::ui::focus::drain_text_surfaces(&ctx);
        let key = crate::ui::focus::SurfaceKey::Pane(77);
        assert!(
            registry.default_for(key).is_some() && registry.claim_for(key).is_some(),
            "Action palette must register its TextInput under SurfaceKey::Pane(pane_id) and claim focus"
        );

        // Type `hidden` (completing `/hidden`) and press Enter.
        let toggled = run_handle_key(
            &mut app,
            vec![
                Event::Text("hidden".to_string()),
                key_event(Key::Enter, Modifiers::default()),
            ],
        );
        assert!(toggled);
        assert!(!app.in_palette, "palette must close after executing /hidden");
        assert!(
            app.columns.show_hidden,
            "/hidden + Enter must toggle columns.show_hidden on"
        );
        assert!(
            app.entries.iter().any(|e| e.name == ".env"),
            "hidden file must appear in entries after /hidden executes"
        );
    }

    #[test]
    fn cmd_n_and_palette_new_folder_slot_prompt_do_not_create_untitled_folder_first() {
        let (mut app, dir) = make_three_file_app();

        // 1. Pressing Cmd+Shift+N opens the palette pre-filled with `/new folder ` without touching disk.
        let opened = run_handle_key(
            &mut app,
            vec![key_event(
                Key::N,
                Modifiers {
                    command: true,
                    shift: true,
                    ..Default::default()
                },
            )],
        );
        assert!(opened);
        assert!(app.in_palette);
        assert_eq!(app.palette_query, "/new folder ");
        assert!(
            !dir.path().join("Untitled Folder").exists(),
            "Cmd+Shift+N must not create Untitled Folder on disk before user enters a name"
        );

        // User types `specs` and presses Enter.
        let created = run_handle_key(
            &mut app,
            vec![
                Event::Text("specs".to_string()),
                key_event(Key::Enter, Modifiers::default()),
            ],
        );
        assert!(created);
        assert!(!app.in_palette);
        assert!(dir.path().join("specs").is_dir());
        assert!(!dir.path().join("Untitled Folder").exists());
        assert_eq!(
            app.selected_entry().map(|e| e.name.as_str()),
            Some("specs")
        );

        // 2. Selecting `/new folder` without a name argument and pressing Enter pre-fills `/new folder `.
        run_handle_key(&mut app, vec![key_event(Key::Slash, Modifiers::default())]);
        assert_eq!(app.palette_query, "/");
        run_handle_key(
            &mut app,
            vec![
                Event::Text("new folder".to_string()),
                key_event(Key::Enter, Modifiers::default()),
            ],
        );
        assert!(
            app.in_palette,
            "selecting /new folder without an argument must stay in palette for slot entry"
        );
        assert_eq!(app.palette_query, "/new folder ");
        assert!(!dir.path().join("Untitled Folder").exists());

        run_handle_key(
            &mut app,
            vec![
                Event::Text("design_docs".to_string()),
                key_event(Key::Enter, Modifiers::default()),
            ],
        );
        assert!(!app.in_palette);
        assert!(dir.path().join("design_docs").is_dir());
    }

    #[test]
    fn bare_slash_palette_query_shows_neutral_command_mode_hint_chip() {
        let (mut app, _dir) = make_three_file_app();
        run_handle_key(&mut app, vec![key_event(Key::Slash, Modifiers::default())]);
        assert!(app.in_palette);
        assert_eq!(app.palette_query, "/");
        assert_eq!(app.palette_selected, 0);

        let candidates = app.rank_palette_candidates();
        let (badge_label, chip_text, is_ready) = app
            .palette_preview_chip(&candidates)
            .expect("palette preview chip");
        assert_eq!(badge_label, "CMD");
        assert!(
            chip_text.contains("Type a command"),
            "bare `/` at index 0 must show neutral command-mode hint instead of SLOT prompt, got: {chip_text}"
        );
        assert!(!is_ready);

        // Moving down with ↓ switches the preview chip to the selected candidate's SLOT/INTENT preview.
        run_handle_key(
            &mut app,
            vec![key_event(Key::ArrowDown, Modifiers::default())],
        );
        assert_eq!(app.palette_selected, 1);
        let (moved_badge, _moved_text, _moved_ready) = app
            .palette_preview_chip(&candidates)
            .expect("palette preview chip after arrow down");
        assert!(matches!(moved_badge, "INTENT" | "SLOT"));
    }

    #[test]
    fn sort_candidates_have_concrete_slash_syntax_and_human_readable_subtitles() {
        let (_app, dir) = make_three_file_app();
        let ctx = intent::FileExplorerContext::new(dir.path(), None, &[], &[], false, true);
        let candidates = intent::rank_file_explorer_candidates("/sort", &ctx);
        assert!(!candidates.is_empty());

        let size_desc = candidates
            .iter()
            .find(|c| c.slash_syntax == "/sort size desc")
            .expect("expected `/sort size desc` candidate");
        assert_eq!(size_desc.subtitle, "Largest files first");
        assert_ne!(size_desc.subtitle, size_desc.slash_syntax);

        for candidate in candidates
            .iter()
            .filter(|c| c.slash_syntax.starts_with("/sort"))
        {
            assert_ne!(
                candidate.slash_syntax, "/sort <column> [asc|desc]",
                "sort candidate must have concrete slash_syntax, not template placeholder"
            );
            assert_ne!(
                candidate.subtitle, candidate.slash_syntax,
                "sort candidate subtitle must be human-readable, not identical to slash_syntax"
            );
        }
    }

    #[test]
    fn cmd_n_opens_new_file_prompt_and_cmd_shift_n_opens_new_folder_prompt() {
        let (mut app, _dir) = make_three_file_app();

        let opened_file = run_handle_key(
            &mut app,
            vec![key_event(
                Key::N,
                Modifiers {
                    command: true,
                    shift: false,
                    ..Default::default()
                },
            )],
        );
        assert!(opened_file);
        assert!(app.in_palette);
        assert_eq!(app.palette_query, "/new file ");

        app.exit_palette();
        assert!(!app.in_palette);

        let opened_folder = run_handle_key(
            &mut app,
            vec![key_event(
                Key::N,
                Modifiers {
                    command: true,
                    shift: true,
                    ..Default::default()
                },
            )],
        );
        assert!(opened_folder);
        assert!(app.in_palette);
        assert_eq!(app.palette_query, "/new folder ");
    }

    #[test]
    fn host_harness_file_browser_palette_executes_hidden_and_create_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("existing.rs"), b"pub fn existing() {}\n")
            .expect("write existing.rs");
        std::fs::write(dir.path().join(".secret"), b"hidden\n").expect("write .secret");

        let mut h = crate::testing::HostHarness::new();
        h.app.open_builtin_app_pane(
            Box::new(FileBrowserApp::new(dir.path().to_path_buf())),
            crate::app::permissions::AppPermissions::builtin(),
            dir.path().to_path_buf(),
            None,
            None,
            None,
        );
        h.run_frames(2);

        let tap_key = |h: &mut crate::testing::HostHarness, key: Key, modifiers: Modifiers| {
            h.frame(RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 800.0),
                )),
                modifiers,
                events: vec![
                    egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers,
                    },
                    egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed: false,
                        repeat: false,
                        modifiers,
                    },
                ],
                ..Default::default()
            });
        };

        // 1. Press `/` in the File Explorer pane, type `hidden`, and press Enter.
        tap_key(&mut h, Key::Slash, Modifiers::NONE);
        h.frame(RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            events: vec![egui::Event::Text("hidden".to_string())],
            ..Default::default()
        });
        tap_key(&mut h, Key::Enter, Modifiers::NONE);
        h.run_frames(1);

        let show_hidden = h.app.windows[0]
            .panes
            .values_mut()
            .find_map(|p| {
                let crate::host::pane::AppRuntime::Builtin(b) = &mut p.as_app_mut()?.runtime else {
                    return None;
                };
                b.as_any_mut().downcast_ref::<FileBrowserApp>()
            })
            .map(|fb| fb.columns.show_hidden)
            .expect("file_browser runtime");
        assert!(
            show_hidden,
            "HostHarness `/` -> `hidden` -> `Enter` must toggle columns.show_hidden"
        );

        // 2. Press `/` again, type `new file hello.rs`, and press Enter: creates hello.rs, selects it, and opens in editor.
        tap_key(&mut h, Key::Slash, Modifiers::NONE);
        h.frame(RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            events: vec![egui::Event::Text("new file hello.rs".to_string())],
            ..Default::default()
        });
        tap_key(&mut h, Key::Enter, Modifiers::NONE);
        h.run_frames(2);

        let hello_path = dir.path().join("hello.rs");
        assert!(hello_path.is_file(), "hello.rs must exist on disk");
        assert!(
            h.pane_count() >= 2,
            "CreateFile with open_in_editor must open hello.rs in a sibling editor pane"
        );
    }
}

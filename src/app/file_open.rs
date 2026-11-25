//! `file.open`: open one file or directory in the Plexi surface that handles it.
//!
//! One operation serves `plexi file open` and its shorthand `plexi open`; the
//! Explorer's file activation uses the same operation. Every open runs one
//! pipeline, in order: resolve, authorize, classify, deliver.
//!
//! - [`resolve_open_target`] resolves the caller's literal path once, against
//!   the caller's working directory, to the canonical target. That canonical
//!   path is the only one authorized, classified and delivered.
//! - Authorization checks the canonical target against the calling pane's
//!   context authority (`pane_may_access_context_root`) before anything reads
//!   it or launches a handler.
//! - [`classify_open_target`] refuses anything that is not a regular file or a
//!   directory, then picks the handler. Directories open in Explorer. Files
//!   follow the cascade: the user's `[file_handlers]` override, the handler a
//!   plugin manifest declares for the extension (`file_types`), a built-in
//!   media player, then the built-in text editor for a known text extension or
//!   for content that reads as UTF-8 text. Classification never executes the
//!   file and reads at most [`CONTENT_SAMPLE_BYTES`].
//! - Delivery gives the handler a fresh pane carrying the target, and the
//!   receipt reports only what was actually delivered.

use std::io::Read;
use std::path::{Path, PathBuf};

use super::file_handlers::FileHandler;
use super::view_trait::ViewEmitter;
use super::PlexiApp;
use crate::broker::caller::RequestOrigin;
use crate::host::pane::ViewRuntime;
use crate::protocol::{ArtifactOpenMode, OpenPlacement};
use crate::spatial::tiling::PaneId;

/// The operation id both CLI spellings and every in-host entry point share.
pub(crate) const OPERATION_ID: &str = "file.open";

/// Bytes read to decide whether a file without a known extension is text.
pub(crate) const CONTENT_SAMPLE_BYTES: usize = 8 * 1024;

const TEXT_EDITOR: &str = "text-editor";
const EXPLORER: &str = "file_browser";

/// The surface that receives an opened target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum OpenHandler {
    /// Explorer rooted at the directory.
    Explorer,
    /// The built-in text editor.
    TextEditor,
    /// A view launched with the path as its only argument: a built-in media
    /// player or a plugin that claims the file type.
    View(String),
    /// The OS default opener, selected by an explicit association or action.
    System,
}

impl OpenHandler {
    pub(crate) fn id(&self) -> &str {
        match self {
            Self::Explorer => EXPLORER,
            Self::TextEditor => TEXT_EDITOR,
            Self::View(id) => id,
            Self::System => "os",
        }
    }
}

/// Why a handler was chosen. Reported in the receipt as `matched_by`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MatchedBy {
    Directory,
    UserOverride,
    Explicit,
    Manifest,
    Media,
    Extension,
    Content,
}

impl MatchedBy {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Directory => "directory",
            Self::UserOverride => "user_override",
            Self::Explicit => "explicit",
            Self::Manifest => "manifest",
            Self::Media => "media",
            Self::Extension => "extension",
            Self::Content => "content",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Classified {
    pub(crate) handler: OpenHandler,
    pub(crate) matched_by: MatchedBy,
}

/// The associations that apply to one target in the caller's context.
#[derive(Debug, Clone, Default)]
pub(crate) struct OpenAssociations {
    /// The raw `[file_handlers]` spec for the target's extension.
    pub(crate) user_override: Option<String>,
    /// The plugin a manifest `file_types` entry declares for the extension.
    pub(crate) manifest_handler: Option<String>,
}

/// A typed refusal. `code` is the stable `error_code` in the reply.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum FileOpenError {
    #[error("no path given")]
    EmptyPath,
    #[error(
        "cannot resolve relative path {path:?}: the caller's working directory is unavailable"
    )]
    CwdUnavailable { path: String },
    #[error("{} does not exist", path.display())]
    NotFound { path: PathBuf },
    #[error("{} is a symbolic link to a missing target", path.display())]
    DanglingLink { path: PathBuf },
    #[error("cannot read {}: {reason}", path.display())]
    Unreadable { path: PathBuf, reason: String },
    #[error("no Plexi handler opens {}: {reason}", path.display())]
    NoHandler { path: PathBuf, reason: String },
    #[error("{} is not a regular file or directory; it is never opened or read", path.display())]
    SpecialFile { path: PathBuf },
    #[error("{} is outside the calling pane's authority (root {})", path.display(), root.display())]
    OutsideAuthority { path: PathBuf, root: PathBuf },
    #[error("{} cannot be passed to {handler}: the path is not valid UTF-8", path.display())]
    UnrepresentablePath { path: PathBuf, handler: String },
    #[error("{handler} could not load {}: {reason}", path.display())]
    LoadFailed {
        path: PathBuf,
        handler: String,
        reason: String,
        /// A live editor that holds the failed load, when one is reported.
        pane: Option<PaneId>,
    },
    #[error("the OS opener failed: {reason}")]
    ExternalOpenFailed { reason: String },
    #[error("the caller could not be identified: {reason}")]
    CallerUnidentified { reason: String },
    #[error("handler {handler:?} is unavailable: {reason}")]
    HandlerUnavailable { handler: String, reason: String },
    #[error("no anchor pane: a caller outside every Plexi pane must pass --from <pane>")]
    NoAnchor,
    #[error("anchor pane {pane} does not exist")]
    AnchorNotFound { pane: u64 },
    #[error("anchor pane {pane} is in another context than the calling pane")]
    AnchorOutsideContext { pane: u64 },
    #[error("{} is already open in pane {existing_pane_id}; a second editor would be a second writable copy", path.display())]
    PlacementConflict {
        path: PathBuf,
        existing_pane_id: PaneId,
    },
}

impl FileOpenError {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::EmptyPath => "invalid_path",
            Self::CwdUnavailable { .. } => "cwd_unavailable",
            Self::NotFound { .. } => "not_found",
            Self::DanglingLink { .. } => "dangling_link",
            Self::Unreadable { .. } => "unreadable",
            Self::NoHandler { .. } => "no_handler",
            Self::SpecialFile { .. } => "special_file",
            Self::OutsideAuthority { .. } => "outside_authority",
            Self::UnrepresentablePath { .. } => "unrepresentable_path",
            Self::LoadFailed { .. } => "load_failed",
            Self::ExternalOpenFailed { .. } => "external_open_failed",
            Self::CallerUnidentified { .. } => "caller_unidentified",
            Self::HandlerUnavailable { .. } => "handler_unavailable",
            Self::NoAnchor => "no_anchor",
            Self::AnchorNotFound { .. } => "anchor_not_found",
            Self::AnchorOutsideContext { .. } => "anchor_outside_context",
            Self::PlacementConflict { .. } => "placement_conflict",
        }
    }

    fn reply(&self) -> serde_json::Value {
        let mut reply = serde_json::json!({
            "error": self.to_string(),
            "error_code": self.code(),
        });
        match self {
            Self::PlacementConflict {
                existing_pane_id, ..
            }
            | Self::LoadFailed {
                pane: Some(existing_pane_id),
                ..
            } => reply["pane_id"] = serde_json::json!(existing_pane_id),
            _ => {}
        }
        reply
    }
}

/// Resolve the caller's literal `path` once, against `cwd`, to the canonical
/// target. A missing file or a link to a missing target is an error here,
/// before any handler is chosen.
pub(crate) fn resolve_open_target(
    path: impl AsRef<Path>,
    cwd: Option<&Path>,
) -> Result<PathBuf, FileOpenError> {
    let literal = path.as_ref();
    if literal.as_os_str().is_empty() {
        return Err(FileOpenError::EmptyPath);
    }
    let joined = if literal.is_absolute() {
        literal.to_path_buf()
    } else {
        cwd.ok_or_else(|| FileOpenError::CwdUnavailable {
            path: literal.display().to_string(),
        })?
        .join(literal)
    };
    match std::fs::symlink_metadata(&joined) {
        Ok(meta) if meta.file_type().is_symlink() && std::fs::metadata(&joined).is_err() => {
            return Err(FileOpenError::DanglingLink { path: joined });
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(FileOpenError::NotFound { path: joined });
        }
        Err(error) => {
            return Err(FileOpenError::Unreadable {
                path: joined,
                reason: error.to_string(),
            })
        }
    }
    std::fs::canonicalize(&joined).map_err(|error| FileOpenError::Unreadable {
        path: joined,
        reason: error.to_string(),
    })
}

fn lowercase_extension(target: &Path) -> String {
    target
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase()
}

fn open_target_is_dir(target: &Path) -> Result<bool, FileOpenError> {
    // Gate on the file type first, before any association can pick a
    // handler that would read it: a FIFO, device or socket is never opened.
    let meta = std::fs::metadata(target).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            FileOpenError::NotFound {
                path: target.to_path_buf(),
            }
        } else {
            FileOpenError::Unreadable {
                path: target.to_path_buf(),
                reason: error.to_string(),
            }
        }
    })?;
    if !(meta.is_dir() || meta.is_file()) {
        return Err(FileOpenError::SpecialFile {
            path: target.to_path_buf(),
        });
    }
    Ok(meta.is_dir())
}

/// Choose the handler for `target`. See the module docs for the order.
pub(crate) fn classify_open_target(
    target: &Path,
    associations: &OpenAssociations,
) -> Result<Classified, FileOpenError> {
    let classified = |handler, matched_by| {
        Ok(Classified {
            handler,
            matched_by,
        })
    };
    if open_target_is_dir(target)? {
        return classified(OpenHandler::Explorer, MatchedBy::Directory);
    }
    if let Some(spec) = &associations.user_override {
        match FileHandler::parse(spec) {
            Ok(FileHandler::Os) => return classified(OpenHandler::System, MatchedBy::UserOverride),
            Ok(FileHandler::View(id)) => {
                return classified(view_handler(id), MatchedBy::UserOverride)
            }
            Err(error) => log::warn!(
                "{OPERATION_ID}: [file_handlers] entry for {} ignored ({error})",
                target.display()
            ),
        }
    }
    if let Some(id) = &associations.manifest_handler {
        return classified(view_handler(id.clone()), MatchedBy::Manifest);
    }
    if let Some(id) = crate::file_browser::MediaKind::for_path(target).player_plugin_id() {
        return classified(OpenHandler::View(id.to_string()), MatchedBy::Media);
    }
    if crate::app::text_editor_view::is_text_editable_ext(&lowercase_extension(target)) {
        return classified(OpenHandler::TextEditor, MatchedBy::Extension);
    }
    match sample_is_text(target) {
        Ok(true) => classified(OpenHandler::TextEditor, MatchedBy::Content),
        Ok(false) => Err(FileOpenError::NoHandler {
            path: target.to_path_buf(),
            reason: "the file is not text and no handler claims its type".to_string(),
        }),
        Err(error) => Err(error),
    }
}

/// The argument a string-argument handler receives for `target`. A path that
/// is not valid UTF-8 has no lossless string form, so it is refused rather
/// than sent as a lossy spelling that could name a different file.
/// An open the host has resolved without touching the disk: the target,
/// the associations of the requesting pane's context and the directory a
/// launched handler starts in.
pub(crate) struct OpenRequest {
    pub(crate) target: std::path::PathBuf,
    pub(crate) associations: OpenAssociations,
    pub(crate) cwd: std::path::PathBuf,
    /// A text editor already holds the target: opening focuses it, so its
    /// body is not read.
    pub(crate) editor_open: bool,
}

/// Everything opening a target reads from disk, gathered by
/// [`prepare_open`] so a caller can run it off the UI thread.
pub(crate) struct PreparedOpen {
    pub(crate) target: std::path::PathBuf,
    pub(crate) cwd: std::path::PathBuf,
    pub(crate) classified: Result<Classified, FileOpenError>,
    /// The classification without the manifest handler, for when that
    /// handler fails to launch. `None` when no manifest handler applied.
    pub(crate) fallback: Option<Result<Classified, FileOpenError>>,
    /// The text editor's read, when either classification picked it and no
    /// editor already held the target.
    pub(crate) text: Option<crate::pane_ops::create::TextEditorRead>,
}

/// Do the disk half of opening `request.target`: stat, content sample, and
/// the text editor's read. Never touches host state.
pub(crate) fn prepare_open(request: OpenRequest) -> PreparedOpen {
    let OpenRequest {
        target,
        mut associations,
        cwd,
        editor_open,
    } = request;
    let classified = classify_open_target(&target, &associations);
    // The cascade without the manifest handler, read now so a failed
    // manifest launch needs no second trip to the disk.
    let fallback = associations
        .manifest_handler
        .take()
        .map(|_| classify_open_target(&target, &associations));
    let edits = |result: &Result<Classified, FileOpenError>| {
        matches!(
            result,
            Ok(Classified {
                handler: OpenHandler::TextEditor,
                ..
            })
        )
    };
    let text = (!editor_open && (edits(&classified) || fallback.as_ref().is_some_and(edits)))
        .then(|| crate::pane_ops::create::TextEditorRead::read(&target, &cwd));
    #[cfg(test)]
    prepared_on::record(&target);
    PreparedOpen {
        target,
        cwd,
        classified,
        fallback,
        text,
    }
}

/// Which thread prepared each open, for tests that keep disk reads off the
/// UI thread. Keyed by target so parallel tests do not see each other.
#[cfg(test)]
pub(crate) mod prepared_on {
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;
    use std::thread::ThreadId;

    static PREPARED: Mutex<Vec<(PathBuf, ThreadId)>> = Mutex::new(Vec::new());

    pub(super) fn record(target: &Path) {
        if let Ok(mut prepared) = PREPARED.lock() {
            prepared.push((target.to_path_buf(), std::thread::current().id()));
        }
    }

    /// The threads that prepared an open of `target`.
    pub(crate) fn threads(target: &Path) -> Vec<ThreadId> {
        PREPARED.lock().map_or_else(
            |_| Vec::new(),
            |prepared| {
                prepared
                    .iter()
                    .filter(|(path, _)| path == target)
                    .map(|(_, thread)| *thread)
                    .collect()
            },
        )
    }
}

pub(crate) fn handler_arg(target: &Path, handler: &str) -> Result<String, FileOpenError> {
    target
        .to_str()
        .map(str::to_string)
        .ok_or_else(|| FileOpenError::UnrepresentablePath {
            path: target.to_path_buf(),
            handler: handler.to_string(),
        })
}

/// The active child of every tab group above `tile`, innermost first.
fn tab_actives(
    window: &crate::host::context::Window,
    tile: egui_tiles::TileId,
) -> Vec<(egui_tiles::TileId, Option<egui_tiles::TileId>)> {
    let mut actives = Vec::new();
    let mut current = tile;
    while let Some((tabs_id, _)) = window.find_ancestor_tabs(current) {
        if let Some(egui_tiles::Tile::Container(egui_tiles::Container::Tabs(tabs))) =
            window.tree.tiles.get(tabs_id)
        {
            actives.push((tabs_id, tabs.active));
        }
        current = tabs_id;
    }
    actives
}

/// Show again what the tab groups above `anchor` showed before an open:
/// a recorded group gets its recorded child back; a group the open created
/// shows the anchor's side.
fn restore_tab_actives(
    window: &mut crate::host::context::Window,
    anchor: egui_tiles::TileId,
    recorded: &[(egui_tiles::TileId, Option<egui_tiles::TileId>)],
) {
    let mut current = anchor;
    while let Some((tabs_id, child)) = window.find_ancestor_tabs(current) {
        let show = recorded
            .iter()
            .find(|(group, _)| *group == tabs_id)
            .map_or(Some(child), |(_, active)| *active);
        if let (Some(show), Some(egui_tiles::Tile::Container(egui_tiles::Container::Tabs(tabs)))) =
            (show, window.tree.tiles.get_mut(tabs_id))
        {
            tabs.set_active(show);
        }
        current = tabs_id;
    }
}

/// A user association or manifest claim naming the editor opens the editor.
fn view_handler(id: String) -> OpenHandler {
    if id == TEXT_EDITOR {
        OpenHandler::TextEditor
    } else {
        OpenHandler::View(id)
    }
}

/// Whether the first [`CONTENT_SAMPLE_BYTES`] of a regular file read as text:
/// no NUL byte and valid UTF-8, allowing a character cut by the sample's end.
/// The caller has already established that `target` is a regular file.
fn sample_is_text(target: &Path) -> Result<bool, FileOpenError> {
    let mut sample = Vec::with_capacity(CONTENT_SAMPLE_BYTES);
    std::fs::File::open(target)
        .and_then(|file| {
            file.take(CONTENT_SAMPLE_BYTES as u64)
                .read_to_end(&mut sample)
        })
        .map_err(|error| FileOpenError::Unreadable {
            path: target.to_path_buf(),
            reason: error.to_string(),
        })?;
    if sample.contains(&0) {
        return Ok(false);
    }
    Ok(match std::str::from_utf8(&sample) {
        Ok(_) => true,
        // `error_len() == None` means the input ended inside a character.
        Err(error) => error.error_len().is_none(),
    })
}

/// One `HostRequest::FileOpen`, minus the host-stamped identity.
#[derive(Clone, Copy)]
pub(super) struct FileOpenRequest<'a> {
    pub(super) path: &'a Path,
    pub(super) cwd: Option<&'a Path>,
    pub(super) from_pane_id: Option<u64>,
    pub(super) placement: Option<OpenPlacement>,
    pub(super) no_focus: bool,
    pub(super) mode: ArtifactOpenMode,
}

/// What `serve_file_open` delivered.
struct Delivered {
    status: &'static str,
    pane_id: Option<PaneId>,
}

impl PlexiApp {
    /// The associations for `target` in context `context_id`.
    pub(super) fn open_associations(&mut self, context_id: u64, target: &Path) -> OpenAssociations {
        let ext = lowercase_extension(target);
        OpenAssociations {
            user_override: self
                .config
                .file_handlers
                .as_ref()
                .and_then(|handlers| handlers.get(&ext))
                .cloned(),
            manifest_handler: self
                .registries
                .view_for_context(context_id, &self.router)
                .handler_for_ext(&ext)
                .map(str::to_string),
        }
    }

    /// Serve `HostRequest::FileOpen` and write its one receipt.
    pub(super) fn serve_file_open(
        &mut self,
        request: &FileOpenRequest<'_>,
        origin: &RequestOrigin,
        response_file: &str,
    ) {
        let reply = self
            .file_open(request, origin)
            .unwrap_or_else(|error| error.reply());
        crate::rpc::write_json_response(response_file, reply);
    }

    pub(super) fn serve_explorer_file_open(
        &mut self,
        emitter: ViewEmitter,
        path: &Path,
        mode: ArtifactOpenMode,
    ) {
        if !self.view_instance_mut(emitter).is_some_and(|pane| {
            matches!(&pane.runtime, ViewRuntime::Builtin(view) if view.type_id() == EXPLORER)
        }) {
            return;
        }
        let request = FileOpenRequest {
            path,
            cwd: None,
            from_pane_id: Some(emitter.pane_id),
            placement: None,
            no_focus: false,
            mode,
        };
        let result = self
            .file_open(&request, &RequestOrigin::HostUi)
            .map(|_| ())
            .map_err(|error| error.to_string());
        if let Some(pane) = self.view_instance_mut(emitter) {
            if let ViewRuntime::Builtin(view) = &mut pane.runtime {
                view.file_open_finished(path, result);
            }
        }
    }

    fn file_open(
        &mut self,
        request: &FileOpenRequest<'_>,
        origin: &RequestOrigin,
    ) -> Result<serde_json::Value, FileOpenError> {
        self.resolve_and_deliver_file(request, origin)
            .inspect_err(|error| {
                log::info!(
                    "{OPERATION_ID}: refused path={:?} cwd={:?} caller={} from={:?} \
                     error_code={} error={error}",
                    request.path,
                    request.cwd,
                    origin.class(),
                    request.from_pane_id,
                    error.code()
                );
            })
    }

    /// Resolve, authorize, classify, and deliver the same canonical path.
    fn resolve_and_deliver_file(
        &mut self,
        request: &FileOpenRequest<'_>,
        origin: &RequestOrigin,
    ) -> Result<serde_json::Value, FileOpenError> {
        use crate::broker::caller::SocketCaller;
        let FileOpenRequest {
            path,
            cwd,
            from_pane_id,
            placement,
            no_focus,
            mode,
        } = *request;
        let window_of = |app: &Self, pane| {
            app.find_pane_in_any_window(pane)
                .map(|(window, _)| window)
                .ok_or(FileOpenError::AnchorNotFound { pane })
        };
        // The anchor only places. Authority comes from the caller's live
        // context, the same narrowing for every pane (stint 1285): a pane
        // the person opened gets no wider reach than an agent's. An outside
        // process acts in the anchor's context with no pane authority.
        let pane_caller = match origin {
            RequestOrigin::Socket(SocketCaller::Pane(pane)) => Some(pane),
            RequestOrigin::Socket(SocketCaller::External(_))
            | RequestOrigin::HostUi
            | RequestOrigin::HostSystem => None,
            RequestOrigin::Socket(SocketCaller::Unidentified(reason)) => {
                return Err(FileOpenError::CallerUnidentified {
                    reason: reason.to_string(),
                })
            }
            RequestOrigin::SpawnQueue | RequestOrigin::PluginGuest(_) => {
                return Err(FileOpenError::CallerUnidentified {
                    reason: format!("file.open is not served to {}", origin.class()),
                })
            }
        };
        if let Some(sealed) = pane_caller.and_then(|pane| pane.authority.grant().sealed_reason()) {
            return Err(FileOpenError::CallerUnidentified {
                reason: sealed.explanation().to_string(),
            });
        }
        let (anchor, context_id) = match (pane_caller, from_pane_id) {
            (Some(pane), None) => (pane.visible_pane, pane.context_id),
            (Some(pane), Some(from)) => {
                if self.windows[window_of(self, from)?].context_id != pane.context_id {
                    return Err(FileOpenError::AnchorOutsideContext { pane: from });
                }
                (from, pane.context_id)
            }
            (None, Some(from)) => (from, self.windows[window_of(self, from)?].context_id),
            (None, None) => return Err(FileOpenError::NoAnchor),
        };
        window_of(self, anchor)?;
        let root = self
            .context_authority_root(context_id)
            .ok_or(FileOpenError::AnchorNotFound { pane: anchor })?;

        let target = resolve_open_target(path, cwd)?;
        // The receipt names the target, so it must have an exact string form.
        handler_arg(&target, "the receipt")?;
        if !self.scope_may_access(&root, &target) {
            return Err(FileOpenError::OutsideAuthority { root, path: target });
        }
        let Classified {
            handler,
            matched_by,
        } = match mode {
            ArtifactOpenMode::OpenInPane => {
                let associations = self.open_associations(context_id, &target);
                classify_open_target(&target, &associations)?
            }
            ArtifactOpenMode::OpenWithDefault | ArtifactOpenMode::RevealInFinder => {
                open_target_is_dir(&target)?;
                Classified {
                    handler: OpenHandler::System,
                    matched_by: MatchedBy::Explicit,
                }
            }
        };

        let delivered = self.deliver_file_open(
            &target,
            &handler,
            anchor,
            placement,
            no_focus,
            mode == ArtifactOpenMode::RevealInFinder,
        )?;
        let focused = delivered
            .pane_id
            .is_some_and(|pane| self.pane_is_shown_focused(pane));
        let placement_name = match delivered.status {
            "opened" => placement.unwrap_or(OpenPlacement::Right).as_str(),
            "reused" => "existing",
            _ => "external",
        };
        log::info!(
            "{OPERATION_ID}: status={} path={path:?} target={} handler={} matched_by={} \
             context_id={context_id} anchor={anchor} caller={} placement={placement_name} \
             pane_id={:?} focused={focused}",
            delivered.status,
            target.display(),
            handler.id(),
            matched_by.as_str(),
            origin.class(),
            delivered.pane_id,
        );
        Ok(serde_json::json!({
            "status": delivered.status,
            "path": target.display().to_string(),
            "handler": handler.id(),
            "matched_by": matched_by.as_str(),
            "context_id": context_id,
            "pane_id": delivered.pane_id,
            "placement": placement_name,
            "focused": focused,
        }))
    }

    /// Whether `pane` is the focused pane of the active window and visible:
    /// every tab group above it shows it.
    fn pane_is_shown_focused(&self, pane: PaneId) -> bool {
        let window = &self.windows[self.active_window];
        let Some(tile) = window.tree.tiles.find_pane(&pane) else {
            return false;
        };
        if window.focused_pane != Some(tile) {
            return false;
        }
        let mut current = tile;
        while let Some((tabs_id, child)) = window.find_ancestor_tabs(current) {
            match window.tree.tiles.get(tabs_id) {
                Some(egui_tiles::Tile::Container(egui_tiles::Container::Tabs(tabs)))
                    if tabs.active == Some(child) => {}
                _ => return false,
            }
            current = tabs_id;
        }
        true
    }

    fn deliver_file_open(
        &mut self,
        target: &Path,
        handler: &OpenHandler,
        anchor: PaneId,
        placement: Option<OpenPlacement>,
        no_focus: bool,
        reveal: bool,
    ) -> Result<Delivered, FileOpenError> {
        // Every handler gets a fresh instance carrying the target: a plugin's
        // `[launch] on_launch` reuse cannot deliver arguments to a running
        // instance, so honoring it here would report a file that never opened.
        let (id, args, cwd_override) = match handler {
            OpenHandler::System => {
                super::canvas_bindings::shell_open(target, reveal)
                    .map_err(|reason| FileOpenError::ExternalOpenFailed { reason })?;
                return Ok(Delivered {
                    status: "external_dispatched",
                    pane_id: None,
                });
            }
            OpenHandler::Explorer => (EXPLORER, Vec::new(), Some(target.to_path_buf())),
            OpenHandler::TextEditor => (TEXT_EDITOR, vec![handler_arg(target, TEXT_EDITOR)?], None),
            OpenHandler::View(id) => (id.as_str(), vec![handler_arg(target, id)?], None),
        };
        let (anchor_window, anchor_tile) = self
            .find_pane_in_any_window(anchor)
            .ok_or(FileOpenError::AnchorNotFound { pane: anchor })?;
        let context_id = self.windows[anchor_window].context_id;
        // Decide on the id the launcher will dispatch on: a contributed view
        // resolves to its owning plugin. Only a view that opens its path
        // argument may receive a file; anything else (a terminal would run
        // the path as a command) is refused before it starts. The launch
        // below passes the requested id, which the launcher resolves the same
        // way (`resolve_launch_id`), so an explicit view id keeps naming its
        // view.
        if matches!(handler, OpenHandler::View(_)) {
            let owner = self.resolve_launch_id(context_id, id).map_err(|reason| {
                FileOpenError::HandlerUnavailable {
                    handler: id.to_string(),
                    reason,
                }
            })?;
            if !crate::pane_ops::create::opens_path_argument(&owner) {
                return Err(FileOpenError::HandlerUnavailable {
                    handler: id.to_string(),
                    reason: format!("it resolves to {owner}, which is not a view that opens files"),
                });
            }
        }

        // One writable buffer per file and context.
        if *handler == OpenHandler::TextEditor {
            let existing = (0..self.windows.len())
                .filter(|idx| self.windows[*idx].context_id == context_id)
                .find_map(|idx| self.find_open_text_editor_tile(idx, target))
                .map(|(_, pane)| pane);
            if let Some(existing_pane_id) = existing {
                // A buffer that never loaded this file is not the file.
                if let Some(Err(reason)) = self
                    .find_pane_in_any_window(existing_pane_id)
                    .and_then(|(window, _)| self.windows[window].panes.get(&existing_pane_id))
                    .and_then(|pane| pane.as_view())
                    .and_then(|view| view.runtime.document_load())
                {
                    return Err(FileOpenError::LoadFailed {
                        path: target.to_path_buf(),
                        handler: TEXT_EDITOR.to_string(),
                        reason,
                        pane: Some(existing_pane_id),
                    });
                }
                if placement.is_some() {
                    return Err(FileOpenError::PlacementConflict {
                        path: target.to_path_buf(),
                        existing_pane_id,
                    });
                }
                if !no_focus {
                    self.pane_navigate(existing_pane_id);
                }
                return Ok(Delivered {
                    status: "reused",
                    pane_id: Some(existing_pane_id),
                });
            }
        }

        let placement = placement.unwrap_or(OpenPlacement::Right);
        let active = self.active_window;
        let window_count = self.windows.len();
        let shown_tabs = tab_actives(&self.windows[anchor_window], anchor_tile);
        let (target_window, restore_tile) = if placement == OpenPlacement::Window {
            (self.open_empty_window_beside(anchor_window), None)
        } else {
            let saved = self.windows[anchor_window].focused_pane;
            self.set_window_focused_pane(anchor_window, anchor_tile);
            (anchor_window, saved)
        };
        let predicted = self.host.next_pane_id();
        // The anchor redirect and the launch are staging, not a focus move
        // the person made: they record no history. The one recorded step is
        // the final navigation below, from where focus really was.
        let recording = std::mem::replace(&mut self.navigating_history, true);
        let launched = self.launch_view_by_id_with_layout_at(
            crate::pane_ops::create::IdViewLaunchOptions {
                target_win: target_window,
                bypass_on_launch: true,
                follow_file_browser_cwd: false,
            },
            id,
            Some(placement.layout_hint().to_string()),
            &args,
            cwd_override,
        );
        self.navigating_history = recording;
        let unavailable = |reason: String| FileOpenError::HandlerUnavailable {
            handler: id.to_string(),
            reason,
        };
        let outcome = match launched {
            Ok(None) => {
                let view = self.windows[target_window]
                    .panes
                    .get(&predicted)
                    .and_then(|pane| pane.as_view())
                    .map(|view| (view.runtime.type_id(), view.runtime.document_load()));
                match view {
                    Some(("launch_failed", _)) => {
                        // A capability preflight failed; the tile holds no file.
                        self.close_pane_by_id(predicted);
                        Err(unavailable(
                            "the plugin is missing capabilities it requires".to_string(),
                        ))
                    }
                    Some((_, Some(Err(reason)))) => {
                        // The pane opened on an empty buffer, not the file.
                        self.close_pane_by_id(predicted);
                        Err(FileOpenError::LoadFailed {
                            path: target.to_path_buf(),
                            handler: id.to_string(),
                            reason,
                            pane: None,
                        })
                    }
                    Some(_) => Ok(Delivered {
                        status: "opened",
                        pane_id: Some(predicted),
                    }),
                    None => Err(unavailable("the handler did not open a pane".to_string())),
                }
            }
            Ok(Some(other)) => Err(unavailable(format!(
                "the handler focused pane {other} instead of opening the file"
            ))),
            Err(reason) => Err(unavailable(reason)),
        };
        if placement == OpenPlacement::Window
            && self.windows.len() > window_count
            && self.windows[target_window].panes.is_empty()
        {
            // The window was pushed last and nothing landed in it.
            self.windows.remove(target_window);
        }
        // Undo the staging: the anchor's tab groups show what they showed
        // before and keyboard focus returns to where it was, so an unfocused
        // open (or a failed one) leaves a new tab hidden.
        self.active_window = active;
        if placement != OpenPlacement::Window {
            restore_tab_actives(&mut self.windows[anchor_window], anchor_tile, &shown_tabs);
            self.restore_window_focused_pane(anchor_window, restore_tile);
        }
        if let Ok(Delivered {
            pane_id: Some(pane),
            ..
        }) = &outcome
        {
            if !no_focus && self.windows[active].context_id == context_id {
                self.pane_navigate(*pane);
            }
        }
        outcome
    }

    /// Push an empty window to the right of `win_idx`'s row, in its context.
    /// Does not activate it. Returns its index.
    fn open_empty_window_beside(&mut self, win_idx: usize) -> usize {
        let context_id = self.windows[win_idx].context_id;
        let grid_y = self.windows[win_idx].grid_y;
        let grid_x = self
            .windows
            .iter()
            .filter(|w| w.context_id == context_id && w.grid_y == grid_y)
            .map(|w| w.grid_x)
            .max()
            .map_or(0, |x| x + 1);
        let window_id = self.next_window_id;
        self.next_window_id += 1;
        let path = self.windows[win_idx].path.clone();
        self.windows.push(crate::host::context::Window {
            name: String::new(),
            path,
            tree: egui_tiles::Tree::empty(format!("file_open_{window_id}")),
            panes: Default::default(),
            focused_pane: None,
            zoomed_pane: None,
            grid_x,
            grid_y,
            window_id,
            context_id,
            tree_history: Default::default(),
        });
        log::info!(
            "{OPERATION_ID}: created window={window_id} context={context_id} grid=({grid_x},{grid_y})"
        );
        self.windows.len() - 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo_file(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join(name)
    }

    fn none() -> OpenAssociations {
        OpenAssociations::default()
    }

    #[test]
    fn real_justfile_opens_in_the_editor_by_content() {
        let target = resolve_open_target("justfile", Some(Path::new(env!("CARGO_MANIFEST_DIR"))))
            .expect("justfile resolves");
        assert_eq!(target, repo_file("justfile").canonicalize().unwrap());
        assert_eq!(
            classify_open_target(&target, &none()).unwrap(),
            Classified {
                handler: OpenHandler::TextEditor,
                matched_by: MatchedBy::Content
            }
        );
    }

    #[test]
    fn real_readme_opens_in_the_editor_by_extension() {
        let target =
            resolve_open_target("README.md", Some(Path::new(env!("CARGO_MANIFEST_DIR")))).unwrap();
        assert_eq!(
            classify_open_target(&target, &none()).unwrap(),
            Classified {
                handler: OpenHandler::TextEditor,
                matched_by: MatchedBy::Extension
            }
        );
    }

    #[test]
    fn relative_paths_resolve_against_the_callers_cwd_once() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("docs")).unwrap();
        std::fs::write(dir.path().join("README.md"), "root").unwrap();
        std::fs::write(dir.path().join("docs/README.md"), "docs").unwrap();
        let docs = dir.path().join("docs");
        let target = resolve_open_target("README.md", Some(&docs)).unwrap();
        assert_eq!(target, docs.join("README.md").canonicalize().unwrap());
        // An absolute path ignores the cwd.
        let absolute = dir.path().join("README.md");
        assert_eq!(
            resolve_open_target(absolute.to_str().unwrap(), Some(&docs)).unwrap(),
            absolute.canonicalize().unwrap()
        );
    }

    #[test]
    fn a_leading_dash_name_is_a_literal_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("-notes.txt"), "x").unwrap();
        let target = resolve_open_target("-notes.txt", Some(dir.path())).unwrap();
        assert_eq!(target.file_name().unwrap(), "-notes.txt");
    }

    #[test]
    fn unresolvable_paths_are_typed_errors() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            resolve_open_target("", Some(dir.path()))
                .unwrap_err()
                .code(),
            "invalid_path"
        );
        assert_eq!(
            resolve_open_target("missing.md", Some(dir.path()))
                .unwrap_err()
                .code(),
            "not_found"
        );
        assert_eq!(
            resolve_open_target("README.md", None).unwrap_err().code(),
            "cwd_unavailable"
        );
        std::os::unix::fs::symlink(dir.path().join("gone"), dir.path().join("link")).unwrap();
        assert_eq!(
            resolve_open_target("link", Some(dir.path()))
                .unwrap_err()
                .code(),
            "dangling_link"
        );
    }

    #[test]
    fn binary_content_has_no_handler() {
        let dir = tempfile::tempdir().unwrap();
        let blob = dir.path().join("blob");
        std::fs::write(&blob, [0x7f, b'E', b'L', b'F', 0, 1, 2]).unwrap();
        assert_eq!(
            classify_open_target(&blob, &none()).unwrap_err().code(),
            "no_handler"
        );
        let invalid = dir.path().join("latin1");
        std::fs::write(&invalid, [b'a', 0xe9, b'b']).unwrap();
        assert_eq!(
            classify_open_target(&invalid, &none()).unwrap_err().code(),
            "no_handler"
        );
    }

    #[test]
    fn a_character_cut_by_the_sample_end_is_still_text() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("long");
        let mut body = "a".repeat(CONTENT_SAMPLE_BYTES - 1).into_bytes();
        body.extend_from_slice("é".as_bytes());
        std::fs::write(&path, body).unwrap();
        assert_eq!(
            classify_open_target(&path, &none()).unwrap().matched_by,
            MatchedBy::Content
        );
    }

    #[test]
    fn handler_cascade_is_override_then_manifest_then_builtin() {
        let dir = tempfile::tempdir().unwrap();
        let notes = dir.path().join("notes.md");
        std::fs::write(&notes, "# hi").unwrap();
        let both = OpenAssociations {
            user_override: Some("app:mdviewer".into()),
            manifest_handler: Some("other".into()),
        };
        assert_eq!(
            classify_open_target(&notes, &both).unwrap(),
            Classified {
                handler: OpenHandler::View("mdviewer".into()),
                matched_by: MatchedBy::UserOverride
            }
        );
        let os = OpenAssociations {
            user_override: Some("os".into()),
            manifest_handler: None,
        };
        assert_eq!(
            classify_open_target(&notes, &os).unwrap().handler,
            OpenHandler::System
        );
        let manifest = OpenAssociations {
            user_override: Some("cmd:bad".into()),
            manifest_handler: Some("other".into()),
        };
        assert_eq!(
            classify_open_target(&notes, &manifest).unwrap(),
            Classified {
                handler: OpenHandler::View("other".into()),
                matched_by: MatchedBy::Manifest
            }
        );
        std::fs::write(dir.path().join("photo.png"), [0x89, b'P', b'N', b'G']).unwrap();
        assert_eq!(
            classify_open_target(&dir.path().join("photo.png"), &none())
                .unwrap()
                .handler,
            OpenHandler::View("image-viewer".into())
        );
        assert_eq!(
            classify_open_target(dir.path(), &both).unwrap(),
            Classified {
                handler: OpenHandler::Explorer,
                matched_by: MatchedBy::Directory
            }
        );
    }

    /// A FIFO blocks any reader. Whatever name or association it carries,
    /// it is refused before a handler is chosen.
    #[test]
    fn a_special_file_is_refused_before_any_handler_is_chosen() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["pipe", "pipe.txt", "pipe.md"] {
            let fifo = dir.path().join(name);
            let status = std::process::Command::new("/usr/bin/mkfifo")
                .arg(&fifo)
                .status()
                .unwrap();
            assert!(status.success());
            for associations in [
                none(),
                OpenAssociations {
                    user_override: Some("app:text-editor".into()),
                    manifest_handler: None,
                },
                OpenAssociations {
                    user_override: Some("os".into()),
                    manifest_handler: None,
                },
                OpenAssociations {
                    user_override: None,
                    manifest_handler: Some("viewer".into()),
                },
            ] {
                assert_eq!(
                    classify_open_target(&fifo, &associations)
                        .unwrap_err()
                        .code(),
                    "special_file",
                    "{name} with {associations:?}"
                );
            }
        }
    }

    /// A path that is not valid UTF-8 has no lossless string form; it is
    /// refused instead of being sent under a lossy spelling.
    #[test]
    fn a_non_utf8_path_is_refused_for_string_handlers() {
        use std::os::unix::ffi::OsStrExt;
        let raw = Path::new(std::ffi::OsStr::from_bytes(b"/tmp/caf\xe9.txt"));
        assert_eq!(
            handler_arg(raw, "text-editor").unwrap_err().code(),
            "unrepresentable_path"
        );
        assert_eq!(
            handler_arg(Path::new("/tmp/café.txt"), "text-editor").unwrap(),
            "/tmp/café.txt"
        );
    }

    /// The OS opener receives the path's exact bytes.
    #[test]
    fn the_os_opener_receives_the_exact_path_bytes() {
        use std::os::unix::ffi::OsStrExt;
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("argv");
        let opener = dir.path().join("opener");
        std::fs::write(
            &opener,
            format!(
                "#!/bin/sh\nprintf '%s' \"$1\" > '{}.tmp' && mv '{0}.tmp' '{0}'\n",
                out.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&opener, std::fs::Permissions::from_mode(0o755)).unwrap();
        super::super::canvas_bindings::set_test_opener(Some(opener.to_str().unwrap()));
        let raw = Path::new(std::ffi::OsStr::from_bytes(b"/tmp/caf\xe9.txt"));
        super::super::canvas_bindings::shell_open(raw, false).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !out.exists() {
            assert!(std::time::Instant::now() < deadline, "opener never ran");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(std::fs::read(&out).unwrap(), b"/tmp/caf\xe9.txt");
    }
}

//! Canvas Terminal Binding Primitives dispatch (#78).
//!
//! The primitives are routed by `host::wasm_pane::route_command` into
//! `AppCommand` variants and end up here once the parent `PlexiApp` drains
//! the deferred queue. Implementation lives in this module rather than
//! `app/mod.rs` to keep the surface readable — the binding-primitive layer
//! is conceptually one feature, not a scattering of unrelated cases.

use super::PlexiApp;
use crate::app_protocol::{ArtifactOpenMode, PathTokenMode, PlexiEvent};
use crate::host::pane::{Pane, TerminalPane};
use crate::spatial::tiling::PaneId;
use egui_term::BackendCommand;
use egui_tiles::Tile;

impl PlexiApp {
    /// `RequestLinkedTerminal` — open a fresh terminal pane next to the
    /// requesting app pane and emit `LinkedTerminalReady` back.
    ///
    /// Strategy: locate the sender's tile, allocate a new pane id, build a
    /// `TerminalPane`, and reuse the existing `split_with_new_pane`
    /// helper (same path used by `Cmd+N`, `split_focused`, etc.) so the
    /// tree manipulation stays in one place. Sender's `linked_pane_id` is
    /// updated so `CdRequest` and the bindings primitives all reference
    /// the new terminal.
    pub(super) fn dispatch_request_linked_terminal(
        &mut self,
        sender_pane_id: PaneId,
        request_id: String,
        cwd: Option<String>,
        place_below: bool,
    ) {
        let active = self.active_window;

        // Resolve cwd: explicit > sender's workspace_root > home.
        let workspace_root = self.windows[active]
            .panes
            .get(&sender_pane_id)
            .and_then(|p| p.as_app())
            .map(|a| a.workspace_root.clone());
        let resolved_cwd = cwd
            .map(std::path::PathBuf::from)
            .or(workspace_root)
            .or_else(|| dirs::home_dir());

        // Find the sender's tile so we can split next to it. Without the
        // tile we have no anchor for the split — bail out and notify the
        // app so the blocking helper unblocks instead of hanging.
        let Some(sender_tile) = find_tile_for_pane(&self.windows[active].tree, sender_pane_id)
        else {
            log::warn!(
                "RequestLinkedTerminal: sender pane {sender_pane_id} not in tree; \
                 dropping with empty Ready"
            );
            self.queue_event_to_pane(
                sender_pane_id,
                PlexiEvent::LinkedTerminalReady {
                    request_id,
                    terminal_pane_id: 0,
                },
            );
            return;
        };

        // Allocate the terminal pane.
        let new_id = self.host.alloc_pane_id();
        let ctx_id = self.windows[active].context_id;
        let ctx_name = self.context_name_for(ctx_id);
        let ctx_desc = self.context_description_for(ctx_id);
        let ctx_root = self.context_root_for(ctx_id);
        let ctx_depth = self.context_depth_for(ctx_id);
        let settings = Self::make_backend_settings(
            new_id,
            resolved_cwd,
            &self.colors,
            ctx_id,
            &ctx_name,
            &ctx_desc,
            ctx_root.as_ref(),
            ctx_depth,
        );
        let Some(term) = TerminalPane::new(
            new_id,
            self.ctx.clone(),
            self.pty_event_tx.clone(),
            settings,
            self.default_font_size,
        ) else {
            log::error!("RequestLinkedTerminal: failed to create TerminalPane for pane {new_id}");
            self.queue_event_to_pane(
                sender_pane_id,
                PlexiEvent::LinkedTerminalReady {
                    request_id,
                    terminal_pane_id: 0,
                },
            );
            return;
        };
        self.windows[active]
            .panes
            .insert(new_id, Pane::Terminal(Box::new(term)));

        // Split the tree directly, adjacent to sender_tile, without touching focused_pane.
        // place_below=false → side-by-side (vertical=true), Canvas app left, terminal right.
        // place_below=true  → stacked (vertical=false), app on top, terminal underneath —
        // the CLI renderer uses this so its form sits above its output.
        // focused_pane is deliberately NOT updated — the Canvas app retains keyboard focus
        // so the user doesn't lose input mid-flow.
        let share =
            crate::host::command::ShareRatio::new(1.0, 1.0).expect("1:1 is a valid ShareRatio");
        let _new_tile = crate::pane_ops::insert_split_tile(
            &mut self.windows[active].tree,
            Some(sender_tile),
            new_id,
            !place_below, // vertical=true → right; vertical=false → below
            share,
            false, // new_pane_first = false → new terminal is the second (right/bottom) child
        );

        // Update the sender's linked_pane_id so legacy CdRequest also
        // routes to this terminal.
        if let Some(pane) = self.windows[active].panes.get_mut(&sender_pane_id) {
            if let Some(app) = pane.as_app_mut() {
                app.linked_pane_id = Some(new_id);
            }
        }

        log::info!(
            "RequestLinkedTerminal: pane {sender_pane_id} → terminal {new_id} \
             (request_id={request_id})"
        );
        self.queue_event_to_pane(
            sender_pane_id,
            PlexiEvent::LinkedTerminalReady {
                request_id,
                terminal_pane_id: new_id,
            },
        );
    }

    /// `RunInLinkedTerminal` — write `command\n` to the referenced
    /// terminal's PTY. `echo` is preserved on the wire but currently
    /// observational — PTY-level echo is shell-controlled. A future
    /// revision can suppress echo via shell-aware injection (e.g. set
    /// `stty -echo` first), but that surface bleeds into shell internals
    /// and is out of scope for v3.5.
    pub(super) fn dispatch_run_in_linked_terminal(
        &mut self,
        sender_pane_id: PaneId,
        terminal_pane_id: PaneId,
        command: String,
        echo: bool,
    ) {
        let active = self.active_window;
        let linked = self.windows[active]
            .panes
            .get(&sender_pane_id)
            .and_then(|p| p.as_app())
            .and_then(|a| a.linked_pane_id);
        if linked != Some(terminal_pane_id) {
            log::warn!(
                "RunInLinkedTerminal: pane {sender_pane_id} rejected — terminal {terminal_pane_id} \
                 not linked (linked={linked:?})"
            );
            return;
        }
        let Some(term) = self.windows[active]
            .panes
            .get_mut(&terminal_pane_id)
            .and_then(|p| p.as_terminal_mut())
        else {
            log::warn!(
                "RunInLinkedTerminal: terminal pane {terminal_pane_id} not found; dropping '{command}'"
            );
            return;
        };
        let payload = format!("{command}\n");
        log::debug!("RunInLinkedTerminal: pane {terminal_pane_id} ← {payload:?} (echo={echo})");
        term.backend
            .process_command(BackendCommand::Write(payload.into_bytes()));
    }

    /// `InsertPathToken` — write a path at the terminal's cursor. In
    /// `Replace` mode, send Ctrl-W (kill-word, ASCII 0x17) first so the
    /// shell's readline removes the partial token.
    pub(super) fn dispatch_insert_path_token(
        &mut self,
        sender_pane_id: PaneId,
        terminal_pane_id: PaneId,
        path: String,
        mode: PathTokenMode,
    ) {
        let active = self.active_window;
        let linked = self.windows[active]
            .panes
            .get(&sender_pane_id)
            .and_then(|p| p.as_app())
            .and_then(|a| a.linked_pane_id);
        if linked != Some(terminal_pane_id) {
            log::warn!(
                "InsertPathToken: pane {sender_pane_id} rejected — terminal {terminal_pane_id} \
                 not linked (linked={linked:?})"
            );
            return;
        }
        let Some(term) = self.windows[active]
            .panes
            .get_mut(&terminal_pane_id)
            .and_then(|p| p.as_terminal_mut())
        else {
            log::warn!(
                "InsertPathToken: terminal pane {terminal_pane_id} not found; dropping '{path}'"
            );
            return;
        };

        let mut bytes: Vec<u8> = Vec::new();
        if matches!(mode, PathTokenMode::Replace) {
            // 0x17 = Ctrl-W = kill-word in readline default mode.
            bytes.push(0x17);
        }
        bytes.extend(quote_for_shell(&path).into_bytes());
        log::debug!("InsertPathToken: pane {terminal_pane_id} mode={mode:?} path={path:?}");
        term.backend.process_command(BackendCommand::Write(bytes));
    }

    /// `RequestCommandPreview` — return what would run in which cwd,
    /// without executing. Useful for "do you want to run X in /tmp/foo?"
    /// modals.
    pub(super) fn dispatch_command_preview(
        &mut self,
        sender_pane_id: PaneId,
        request_id: String,
        terminal_pane_id: PaneId,
        command: String,
    ) {
        let active = self.active_window;
        let linked = self.windows[active]
            .panes
            .get(&sender_pane_id)
            .and_then(|p| p.as_app())
            .and_then(|a| a.linked_pane_id);
        if linked != Some(terminal_pane_id) {
            log::warn!(
                "RequestCommandPreview: pane {sender_pane_id} rejected — terminal {terminal_pane_id} \
                 not linked (linked={linked:?})"
            );
            self.queue_event_to_pane(
                sender_pane_id,
                PlexiEvent::CommandPreview {
                    request_id,
                    command,
                    would_run_in_cwd: String::new(),
                },
            );
            return;
        }
        let cwd = self.windows[active]
            .panes
            .get(&terminal_pane_id)
            .and_then(|p| p.as_terminal())
            .and_then(|t| crate::host::shell::get_pid_cwd(t.backend.child_pid()))
            .map(|p| p.display().to_string())
            .unwrap_or_default();

        log::debug!(
            "RequestCommandPreview: pane {terminal_pane_id} command={command:?} cwd={cwd:?}"
        );
        self.queue_event_to_pane(
            sender_pane_id,
            PlexiEvent::CommandPreview {
                request_id,
                command,
                would_run_in_cwd: cwd,
            },
        );
    }

    /// `OpenArtifact` — route to the file browser (`OpenInPane`) or shell
    /// out to `open` / `open -R` (`OpenWithDefault` / `RevealInFinder`).
    /// On non-macOS platforms `open` is unavailable; the host logs a
    /// warning and the request becomes a no-op rather than failing the
    /// frame.
    pub(super) fn dispatch_open_artifact(
        &mut self,
        sender_pane_id: PaneId,
        path: String,
        mode: ArtifactOpenMode,
    ) {
        let active = self.active_window;
        let workspace_root = self.windows[active]
            .panes
            .get(&sender_pane_id)
            .and_then(|p| p.as_app())
            .map(|a| a.workspace_root.clone());
        // workspace_root is None for builtin apps (which run with trusted
        // AppPermissions::builtin()) — they bypass the path check intentionally.
        // For process apps the root is always set; None here means the pane was
        // already removed (race between close and dispatch), so we allow through
        // rather than silently dropping a legitimate late-arriving command.
        //
        // Two gates, both must pass:
        //   1. Lexical (normalize_path collapses ".." without I/O) — a cheap
        //      first reject for obvious `../` traversal.
        //   2. Canonical (resolves symlinks via the filesystem) — a symlink that
        //      lives inside the workspace but points outside passes the lexical
        //      check, so `open` would follow it out of the sandbox (#2242). The
        //      canonical gate rejects targets whose *real* path escapes the root.
        if let Some(ref root) = workspace_root {
            let p = std::path::Path::new(&path);
            let absolute = if p.is_absolute() {
                p.to_path_buf()
            } else {
                root.join(p)
            };
            let normalized = normalize_path(&absolute);
            if !normalized.starts_with(root) {
                log::warn!(
                    "OpenArtifact: pane {sender_pane_id} rejected — path {path:?} outside \
                     workspace {root:?}"
                );
                return;
            }
            if !canonical_within_workspace(root, &absolute) {
                log::warn!(
                    "OpenArtifact: pane {sender_pane_id} rejected — real path of {path:?} \
                     escapes workspace {root:?} (symlink)"
                );
                return;
            }
        }
        log::info!("OpenArtifact: path={path:?} mode={mode:?}");
        match mode {
            ArtifactOpenMode::OpenInPane => {
                // Directories open the file browser; files run through the
                // single open resolver (#2283): `[file_handlers]` → manifest
                // `file_types` → builtin media players → OS default.
                let p = std::path::PathBuf::from(&path);
                if p.is_dir() {
                    // Reuse the file-browser open path, but anchored at
                    // the requested cwd. The current `open_file_browser`
                    // anchors at the focused pane's cwd; we replicate
                    // its body here with our path.
                    self.open_file_browser_at(p);
                } else {
                    self.open_file_in_app(sender_pane_id, &path);
                }
            }
            ArtifactOpenMode::RevealInFinder => shell_open(&path, true),
            ArtifactOpenMode::OpenWithDefault => shell_open(&path, false),
        }
    }

    /// Queue an outbound `PlexiEvent` to the app pane identified by
    /// `pane_id`. No-op when the pane has gone away (it can during a
    /// pending dispatch — the user closed the canvas app between request
    /// and response).
    fn queue_event_to_pane(&mut self, pane_id: PaneId, event: PlexiEvent) {
        let active = self.active_window;
        if let Some(pane) = self.windows[active].panes.get_mut(&pane_id) {
            if let Some(app) = pane.as_app_mut() {
                app.runtime.queue_outbound_event(event);
            }
        }
    }

    /// Internal: open the file browser app rooted at `cwd`.
    /// Mirror of `open_file_browser` but driven by an explicit path
    /// instead of the focused-pane cwd, since OpenArtifact specifies the
    /// directory it wants browsed.
    fn open_file_browser_at(&mut self, cwd: std::path::PathBuf) {
        use crate::app::app_trait::App;
        let app: Box<dyn App> = Box::new(crate::file_browser::FileBrowserApp::new(cwd.clone()));
        let perms = crate::app::permissions::AppPermissions::builtin();
        self.open_builtin_app_pane(
            app,
            perms,
            cwd,
            Some("cwd".to_string()),
            Some("split_v"),
            Some(0.5),
        );
    }

    /// The single file-open resolver (#2283). Decides which handler opens
    /// `path` and launches it. Resolution order, first match wins:
    ///   (a) user `[file_handlers]` override
    ///   (b) app manifest `file_types` association
    ///   (c) builtin media players (`MediaKind`)
    ///   (d) builtin text-editor for text/source files
    ///   (e) OS default opener — mandatory fallback
    /// Any `app:` target that is absent or fails to launch falls through to the
    /// OS opener rather than silently doing nothing — this is what makes
    /// "Enter on a video with no installed player" open in the OS player
    /// instead of vanishing. `sender_pane_id` is the pane that requested the
    /// open (the explorer, or the terminal behind it); the text-editor lands as
    /// a split to its right.
    fn open_file_in_app(&mut self, sender_pane_id: PaneId, path: &str) {
        use crate::app::file_handlers::FileHandler;
        let ext = std::path::Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        // (a) user `[file_handlers]` override.
        if let Some(spec) = self
            .config
            .file_handlers
            .as_ref()
            .and_then(|m| m.get(&ext))
            .cloned()
        {
            match FileHandler::parse(&spec) {
                Some(FileHandler::Os) => {
                    log::info!("open: '{path}' → OS default (file_handlers override)");
                    shell_open(path, false);
                    return;
                }
                Some(FileHandler::App(id)) => {
                    if self.launch_app_with_path(&id, path) {
                        log::info!("open: '{path}' → app '{id}' (file_handlers override)");
                        return;
                    }
                    log::warn!(
                        "open: file_handler app '{id}' for .{ext} unavailable — OS fallback for '{path}'"
                    );
                    shell_open(path, false);
                    return;
                }
                Some(FileHandler::Cmd(cmd)) => {
                    log::warn!(
                        "open: cmd handler '{cmd}' for .{ext} not yet implemented — OS fallback for '{path}'"
                    );
                    shell_open(path, false);
                    return;
                }
                None => log::warn!(
                    "open: invalid file_handler '{spec}' for .{ext} — ignoring, continuing resolution"
                ),
            }
        }

        // (b) app manifest `file_types` association.
        if let Some(id) = self.registry.handler_for_ext(&ext).map(|s| s.to_string()) {
            if self.launch_app_with_path(&id, path) {
                log::info!("open: '{path}' → app '{id}' (manifest file_types)");
                return;
            }
            log::warn!("open: manifest handler '{id}' for .{ext} failed — continuing resolution");
        }

        // (c) builtin media players.
        if let Some(id) =
            crate::file_browser::MediaKind::for_path(std::path::Path::new(path)).player_app_id()
        {
            if self.launch_app_with_path(id, path) {
                log::info!("open: '{path}' → builtin media player '{id}'");
                return;
            }
            log::warn!("open: media player '{id}' not installed — OS fallback for '{path}'");
        }

        // (d) builtin text-editor: text and source files open in a split to
        // the right of the requesting pane. Sits below user/manifest overrides
        // so a user-configured handler for e.g. `.md` still wins.
        if crate::app::text_editor_app::is_text_editable_ext(&ext) {
            log::info!("open: '{path}' → builtin text-editor (split right of pane {sender_pane_id})");
            self.open_text_file_in_split(sender_pane_id, path);
            return;
        }

        // (e) OS default — mandatory fallback.
        log::info!("open: '{path}' → OS default (resolver fallthrough: no Plexi handler)");
        shell_open(path, false);
    }

    /// Open `path` in the builtin text-editor as a `split_h` anchored at
    /// `anchor_pane_id`, so the editor lands to the right of the explorer (or
    /// terminal) that requested it. Mirrors the `SpawnPane` app-branch: it
    /// resolves the anchor's window from its pane id (never a global-focus
    /// read), retargets the launch to that window/tile, then restores the
    /// original active window. The `context_id` follows implicitly from
    /// anchoring on the pane's own window, so the split stays in the caller's
    /// context.
    fn open_text_file_in_split(&mut self, anchor_pane_id: PaneId, path: &str) {
        let original_active_window = self.active_window;
        match self.find_pane_in_any_window(anchor_pane_id) {
            Some((win_idx, tile)) => {
                self.active_window = win_idx;
                self.set_window_focused_pane(win_idx, tile);
            }
            None => {
                log::warn!(
                    "open: text-editor anchor pane {anchor_pane_id} not found — using active window"
                );
            }
        }
        // Forced (bypass the app's `on_launch` dedup): opening a specific file
        // must always load THAT file. A `focus_existing` policy would otherwise
        // resolve to an already-open editor and drop the path, so the explorer
        // would appear to do nothing for a different file.
        if let Err(e) = self.launch_app_by_id_with_layout_forced(
            "text-editor",
            Some("split_h".to_string()),
            &[path.to_string()],
            None,
        ) {
            log::warn!("open: text-editor launch for '{path}' failed — {e}; OS fallback");
            self.active_window = original_active_window;
            shell_open(path, false);
            return;
        }
        self.active_window = original_active_window;
    }

    /// Launch app `id` with `path` as its sole arg, using the app's natural
    /// placement (`None` layout → manifest `[launch] placement` → `overlay`).
    /// Returns `true` on a successful launch; `false` when the app is not
    /// available, so the resolver can fall through to the OS opener.
    fn launch_app_with_path(&mut self, id: &str, path: &str) -> bool {
        let layout = matches!(id, "image-viewer" | "video-player" | "audio-player")
            .then(|| "split_h".to_string());
        match self.launch_app_by_id_with_layout(id, layout, &[path.to_string()], None) {
            Ok(_) => true,
            Err(e) => {
                log::warn!("open: launch of app '{id}' for '{path}' failed — {e}");
                false
            }
        }
    }
}

/// Find the `TileId` of the leaf tile containing `pane_id`, if any.
fn find_tile_for_pane(
    tree: &egui_tiles::Tree<PaneId>,
    pane_id: PaneId,
) -> Option<egui_tiles::TileId> {
    for (tile_id, tile) in tree.tiles.iter() {
        if let Tile::Pane(pid) = tile {
            if *pid == pane_id {
                return Some(*tile_id);
            }
        }
    }
    None
}

/// POSIX-quote `path` if it contains shell metacharacters; otherwise
/// return verbatim. We don't escape *every* path because the common
/// case (alphanumerics, slashes, dots, dashes, underscores) is shell-
/// safe and quoting a clean path uglifies the typed command.
fn quote_for_shell(path: &str) -> String {
    if path.is_empty() {
        return "''".to_string();
    }
    let safe = path
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '-' | '_' | ':' | '@' | '+'));
    if safe {
        path.to_string()
    } else {
        // POSIX single-quote escape: every existing single quote becomes
        // '\'' (close, escaped quote, reopen).
        let escaped = path.replace('\'', "'\\''");
        format!("'{escaped}'")
    }
}

/// Scheme-validated external URL open: only `http`/`https` URLs are handed
/// to the OS opener; anything else (including `file:`, `javascript:`, bare
/// paths) is rejected. Failures return to the caller so it can surface a
/// visible state — never swallowed into a log line.
pub fn open_http_url(url: &str) -> Result<(), String> {
    let parsed = url::Url::parse(url).map_err(|e| format!("invalid URL {url:?}: {e}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(format!(
            "refusing to open non-http(s) URL scheme {:?}",
            parsed.scheme()
        ));
    }
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg(url).spawn();
    #[cfg(target_os = "linux")]
    let result = std::process::Command::new("xdg-open").arg(url).spawn();
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let result: std::io::Result<std::process::Child> = Err(std::io::Error::other(
        "URL open is macOS/Linux-only",
    ));
    match result {
        Ok(_) => Ok(()),
        Err(e) => Err(format!("OS opener failed for {url}: {e}")),
    }
}

/// Cross-platform helper: shell out to `open` (macOS) / `xdg-open`
/// (Linux). On Windows we currently log + skip — the binding-primitives
/// surface is macOS-first for v3.5; cross-platform reveals can land in
/// a follow-up.
#[cfg(not(target_arch = "wasm32"))]
fn shell_open(path: &str, reveal: bool) {
    #[cfg(target_os = "macos")]
    {
        let mut cmd = std::process::Command::new("open");
        if reveal {
            cmd.arg("-R");
        }
        cmd.arg(path);
        match cmd.spawn() {
            Ok(_) => log::debug!(
                "OpenArtifact: spawned `open{}` for {path}",
                if reveal { " -R" } else { "" }
            ),
            Err(e) => log::error!("OpenArtifact: `open` failed for {path}: {e}"),
        }
    }
    #[cfg(target_os = "linux")]
    {
        // `xdg-open` has no "reveal" flag; fall back to opening the
        // parent directory when reveal=true.
        let target = if reveal {
            std::path::Path::new(path)
                .parent()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| path.to_string())
        } else {
            path.to_string()
        };
        match std::process::Command::new("xdg-open").arg(&target).spawn() {
            Ok(_) => log::debug!("OpenArtifact: spawned xdg-open for {target}"),
            Err(e) => log::error!("OpenArtifact: xdg-open failed for {target}: {e}"),
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = (path, reveal);
        log::warn!(
            "OpenArtifact: shell-open is macOS/Linux-only in v3.5; \
             dropping reveal={reveal} path={path}"
        );
    }
}

/// True when the *real* (symlink-resolved) path of `absolute` stays inside the
/// canonicalized `workspace_root`. Complements the lexical `normalize_path`
/// check: a symlink inside the workspace pointing outside it passes the lexical
/// gate but fails here (#2242).
fn canonical_within_workspace(root: &std::path::Path, absolute: &std::path::Path) -> bool {
    let canonical_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    canonicalize_existing_prefix(absolute).starts_with(&canonical_root)
}

/// Canonicalize the deepest existing ancestor of `path` (resolving any symlinks
/// on the way) and re-append the not-yet-existing tail. Symlinks can only live
/// on the existing prefix, so this catches a workspace-internal symlink that
/// redirects outside the sandbox even when the final target does not exist yet.
/// Falls back to the lexical normalization when nothing on the chain exists.
fn canonicalize_existing_prefix(path: &std::path::Path) -> std::path::PathBuf {
    for ancestor in path.ancestors() {
        if let Ok(canonical) = ancestor.canonicalize() {
            return match path.strip_prefix(ancestor) {
                Ok(rest) => canonical.join(rest),
                Err(_) => canonical,
            };
        }
    }
    normalize_path(path)
}

fn normalize_path(path: &std::path::Path) -> std::path::PathBuf {
    use std::path::Component;
    let mut result = std::path::PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                result.pop();
            }
            Component::CurDir => {}
            c => result.push(c),
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::{canonical_within_workspace, normalize_path, quote_for_shell};
    use std::path::PathBuf;

    /// A symlink that lives inside the workspace but points outside it passes the
    /// lexical check yet must be rejected by the canonical containment gate
    /// (#2242). Regression for the OpenArtifact sandbox escape.
    #[test]
    #[cfg(unix)]
    fn canonical_check_rejects_workspace_internal_symlink_to_outside() {
        let tmp = std::env::temp_dir().join(format!("plexi-artifact-{}", uuid::Uuid::new_v4()));
        let workspace = tmp.join("workspace");
        let outside = tmp.join("outside");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let secret = outside.join("secret.txt");
        std::fs::write(&secret, b"top secret").unwrap();

        // `workspace/link` -> `../outside` (a symlink inside the workspace).
        let link = workspace.join("link");
        std::os::unix::fs::symlink(&outside, &link).unwrap();

        // Lexically, `workspace/link/secret.txt` is under the workspace root.
        let via_symlink = link.join("secret.txt");
        assert!(
            normalize_path(&via_symlink).starts_with(&workspace),
            "lexical check should pass (that is the bug)"
        );
        // Canonically, its real path escapes the workspace and must be rejected.
        assert!(
            !canonical_within_workspace(&workspace, &via_symlink),
            "canonical check must reject a symlink escaping the workspace"
        );

        // A genuine in-workspace file is still accepted.
        let inside = workspace.join("ok.txt");
        std::fs::write(&inside, b"fine").unwrap();
        assert!(
            canonical_within_workspace(&workspace, &inside),
            "canonical check must accept a real in-workspace file"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn normalize_path_collapses_parent_dirs() {
        assert_eq!(
            normalize_path(&PathBuf::from("/a/b/../c")),
            PathBuf::from("/a/c")
        );
        assert_eq!(
            normalize_path(&PathBuf::from("/a/b/c/../../d")),
            PathBuf::from("/a/d")
        );
        assert_eq!(
            normalize_path(&PathBuf::from("/a/./b/../c/.")),
            PathBuf::from("/a/c")
        );
    }

    #[test]
    fn normalize_path_cannot_escape_above_root() {
        // Pop at the root component is a no-op — result stays at /etc.
        assert_eq!(
            normalize_path(&PathBuf::from("/../etc")),
            PathBuf::from("/etc")
        );
    }

    #[test]
    fn normalize_path_traversal_is_blocked_by_workspace_check() {
        let root = PathBuf::from("/workspace");
        let escaped = normalize_path(&PathBuf::from("/workspace/../../etc/passwd"));
        assert!(
            !escaped.starts_with(&root),
            "traversal should escape workspace: {escaped:?}"
        );
    }

    #[test]
    fn quote_for_shell_safe_paths_pass_through() {
        assert_eq!(quote_for_shell("/tmp/foo"), "/tmp/foo");
        assert_eq!(quote_for_shell("./bar.txt"), "./bar.txt");
        assert_eq!(quote_for_shell("a-b_c.d"), "a-b_c.d");
    }

    #[test]
    fn quote_for_shell_unsafe_paths_get_quoted() {
        assert_eq!(quote_for_shell("/tmp/with space"), "'/tmp/with space'");
        assert_eq!(quote_for_shell("a&b"), "'a&b'");
        assert_eq!(quote_for_shell("$HOME"), "'$HOME'");
    }

    #[test]
    fn quote_for_shell_handles_embedded_quote() {
        assert_eq!(quote_for_shell("o'brien"), "'o'\\''brien'");
    }

    #[test]
    fn quote_for_shell_empty_string() {
        assert_eq!(quote_for_shell(""), "''");
    }
}

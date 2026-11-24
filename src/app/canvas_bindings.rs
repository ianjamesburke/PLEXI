//! Canvas Terminal Binding Primitives dispatch (#78).
//!
//! The primitives are routed by `host::wasm_pane::route_command` into
//! `ViewCommand` variants and end up here once the parent `PlexiApp` drains
//! the deferred queue. Implementation lives in this module rather than
//! `plugin/mod.rs` to keep the surface readable — the binding-primitive layer
//! is conceptually one feature, not a scattering of unrelated cases.

use super::PlexiApp;
use crate::host::pane::Pane;
use crate::protocol::{ArtifactOpenMode, PathTokenMode, PlexiEvent};
use crate::spatial::tiling::PaneId;
use egui_term::BackendCommand;
use egui_tiles::Tile;

impl PlexiApp {
    /// `RequestLinkedTerminal` — open a fresh terminal pane next to the
    /// requesting view pane and emit `LinkedTerminalReady` back.
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
            .and_then(|p| p.as_view())
            .map(|a| a.workspace_root.clone());
        let resolved_cwd = cwd
            .map(std::path::PathBuf::from)
            .or(workspace_root)
            .or_else(dirs::home_dir);

        // Find the sender's tile so we can split next to it. Without the
        // tile we have no anchor for the split — bail out and notify the
        // plugin so the blocking helper unblocks instead of hanging.
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

        let context_env = self.pane_context_env_for_window(active);
        // The view asked for this terminal, not the person: it is admitted as
        // that view's, with no secrets (stint 1666).
        let launch = self.plugin_terminal_launch(sender_pane_id, context_env, resolved_cwd, None);
        let new_id = launch.pane_id();
        let term = match self.construct_terminal(launch) {
            Ok(term) => term,
            Err(error) => {
                log::error!("RequestLinkedTerminal: failed to create TerminalPane for pane {new_id}: {error}");
                self.queue_event_to_pane(
                    sender_pane_id,
                    PlexiEvent::LinkedTerminalReady {
                        request_id,
                        terminal_pane_id: 0,
                    },
                );
                return;
            }
        };
        if self.windows[active]
            .panes
            .insert(new_id, Pane::Terminal(Box::new(term)))
            .is_err()
        {
            return;
        }

        // Split the tree directly, adjacent to sender_tile, without touching focused_pane.
        // place_below=false → side-by-side (vertical=true), Canvas plugin left, terminal right.
        // place_below=true  → stacked (vertical=false), plugin on top, terminal underneath —
        // the CLI renderer uses this so its form sits above its output.
        // focused_pane is deliberately NOT updated — the Canvas plugin retains keyboard focus
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
        // Focus stays on the sender, but the split can re-id its tile (stint 1230).
        self.windows[active].simplify_and_reconcile();

        // Update the sender's linked_pane_id so legacy CdRequest also
        // routes to this terminal.
        if let Some(pane) = self.windows[active].panes.get_mut(&sender_pane_id) {
            if let Some(app) = pane.as_view_mut() {
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
    pub(crate) fn dispatch_run_in_linked_terminal(
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
            .and_then(|p| p.as_view())
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
                "RunInLinkedTerminal: terminal pane {terminal_pane_id} not found; dropping command len={}",
                command.chars().count()
            );
            return;
        };
        let payload = format!("{command}\n");
        log::debug!(
            "{}",
            linked_terminal_payload_log_message(terminal_pane_id, &payload, echo)
        );
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
            .and_then(|p| p.as_view())
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
            .and_then(|p| p.as_view())
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
            "RequestCommandPreview: pane {terminal_pane_id} command_chars={} cwd={cwd:?}",
            command.chars().count()
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
    pub(crate) fn dispatch_open_artifact(
        &mut self,
        sender_pane_id: PaneId,
        path: String,
        mode: ArtifactOpenMode,
    ) {
        // The sender may live in any window, not only the active one.
        let workspace_root = self
            .find_pane_in_any_window(sender_pane_id)
            .and_then(|(window, _)| self.windows[window].panes.get(&sender_pane_id))
            .and_then(|p| p.as_view())
            .map(|a| a.workspace_root.clone());
        // workspace_root is None for builtin plugins (which run with trusted
        // PluginPermissions::builtin()) — they bypass the path check intentionally.
        // For process plugins the root is always set; None here means the pane was
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
        // The canonical path the second gate checked is the only one any mode
        // opens, so check and use name the same file.
        let target = match &workspace_root {
            Some(root) => {
                let p = std::path::Path::new(&path);
                let absolute = if p.is_absolute() {
                    p.to_path_buf()
                } else {
                    root.join(p)
                };
                if !normalize_path(&absolute).starts_with(root) {
                    log::warn!(
                        "OpenArtifact: pane {sender_pane_id} rejected — path {path:?} outside \
                         workspace {root:?}"
                    );
                    return;
                }
                let checked = canonicalize_existing_prefix(&absolute);
                if !within_canonical_root(root, &checked) {
                    log::warn!(
                        "OpenArtifact: pane {sender_pane_id} rejected — real path of {path:?} \
                         escapes workspace {root:?} (symlink)"
                    );
                    return;
                }
                checked
            }
            None => std::path::PathBuf::from(&path),
        };
        log::info!("OpenArtifact: path={path:?} target={target:?} mode={mode:?}");
        match mode {
            ArtifactOpenMode::OpenInPane => {
                // Directories open the file browser; files run through the
                // shared `file.open` classification (`super::file_open`).
                if target.is_dir() {
                    self.open_file_browser_at(target);
                } else {
                    self.open_file_in_view(sender_pane_id, &target);
                }
            }
            ArtifactOpenMode::RevealInFinder => {
                let _logged = shell_open(&target, true);
            }
            ArtifactOpenMode::OpenWithDefault => {
                let _logged = shell_open(&target, false);
            }
        }
    }

    /// Queue an outbound `PlexiEvent` to the view pane identified by
    /// `pane_id`. No-op when the pane has gone away (it can during a
    /// pending dispatch — the user closed the canvas plugin between request
    /// and response).
    fn queue_event_to_pane(&mut self, pane_id: PaneId, event: PlexiEvent) {
        let active = self.active_window;
        if let Some(pane) = self.windows[active].panes.get_mut(&pane_id) {
            if let Some(app) = pane.as_view_mut() {
                app.runtime.queue_outbound_event(event);
            }
        }
    }

    /// Internal: open the file browser plugin rooted at `cwd`.
    /// Mirror of `open_file_browser` but driven by an explicit path
    /// instead of the focused-pane cwd, since OpenArtifact specifies the
    /// directory it wants browsed.
    fn open_file_browser_at(&mut self, cwd: std::path::PathBuf) {
        if self.plugin_blocked("file_browser") {
            return;
        }
        use crate::app::view_trait::View;
        let app: Box<dyn View> = Box::new(crate::file_browser::FileBrowserView::new(cwd.clone()));
        let perms = crate::app::permissions::PluginPermissions::builtin();
        self.open_builtin_view_pane(
            app,
            perms,
            cwd,
            None, // Explicit directory intent must not follow a terminal.
            Some("split_v"),
            Some(0.5),
        );
    }

    /// Explorer file activation: open `path` with the handler
    /// `super::file_open::classify_open_target` picks in the sender's context
    /// (`[file_handlers]`, manifest `file_types`, media players, text editor).
    /// A manifest handler that fails to launch hands the file to the rest of
    /// the cascade; any other unavailable handler, or a regular file no Plexi
    /// handler claims, goes to the OS default opener. A special file (FIFO,
    /// device, socket) is refused. `sender_pane_id` is the pane that requested
    /// the open (the explorer, or the terminal behind it); the text-editor
    /// lands as a split to its right.
    pub(crate) fn open_file_in_view(&mut self, sender_pane_id: PaneId, target: &std::path::Path) {
        if let Some(request) = self.open_request(sender_pane_id, target) {
            let prepared = super::file_open::prepare_open(request);
            self.finish_open(sender_pane_id, prepared);
        }
    }

    /// The host-state half of opening `target` for `sender_pane_id`: its
    /// context's associations and launch directory. Reads no disk, so a
    /// caller may hand the result to `file_open::prepare_open` on a worker
    /// and finish with [`Self::finish_open`]. `None` when file viewers are
    /// blocked.
    pub(crate) fn open_request(
        &mut self,
        sender_pane_id: PaneId,
        target: &std::path::Path,
    ) -> Option<super::file_open::OpenRequest> {
        if self.plugin_blocked("file viewer / text-editor") {
            return None;
        }
        // Resolve the registry view against the REQUESTING pane's own
        // context, not whichever context is currently active (stint 0724
        // Phase B) — `find_pane_in_any_window` finds it via the pane table,
        // never `active_window`/`router.active()`, so a background pane
        // asking to open a file still resolves its own workspace's handlers.
        let context_id = self
            .find_pane_in_any_window(sender_pane_id)
            .map(|(window_index, _)| self.windows[window_index].context_id)
            .unwrap_or_else(|| self.windows[self.active_window].context_id);
        Some(super::file_open::OpenRequest {
            target: target.to_path_buf(),
            associations: self.open_associations(context_id, target),
            cwd: self.launch_cwd(context_id, None),
            editor_open: self.find_open_text_editor_pane_any_window(target).is_some(),
        })
    }

    /// Open a prepared target with the handler its classification picked.
    /// Reads no disk: `prepared` carries every read.
    pub(crate) fn finish_open(
        &mut self,
        sender_pane_id: PaneId,
        prepared: super::file_open::PreparedOpen,
    ) {
        use super::file_open::{handler_arg, FileOpenError, MatchedBy, OpenHandler};
        let super::file_open::PreparedOpen {
            target,
            cwd,
            classified,
            mut fallback,
            mut text,
        } = prepared;
        let target = target.as_path();
        let mut classified = classified;
        loop {
            let chosen = match classified {
                Ok(chosen) => chosen,
                Err(error @ FileOpenError::SpecialFile { .. }) => {
                    log::warn!("open: {target:?} refused ({error})");
                    return;
                }
                Err(error) => {
                    log::info!("open: {target:?} → OS default ({error})");
                    let _logged = shell_open(target, false);
                    return;
                }
            };
            let matched_by = chosen.matched_by.as_str();
            // String-argument handlers need a lossless string form of the path.
            let arg = |handler: &str| match handler_arg(target, handler) {
                Ok(arg) => Some(arg),
                Err(error) => {
                    log::warn!("open: {target:?} refused ({error})");
                    None
                }
            };
            match chosen.handler {
                OpenHandler::System => {
                    log::info!("open: {target:?} → OS default ({matched_by})");
                    let _logged = shell_open(target, false);
                }
                OpenHandler::Explorer => self.open_file_browser_at(target.to_path_buf()),
                OpenHandler::TextEditor => {
                    let Some(path) = arg("text-editor") else {
                        return;
                    };
                    log::info!(
                        "open: '{path}' → builtin text-editor ({matched_by}, split right of pane {sender_pane_id})"
                    );
                    self.open_text_file_in_split(sender_pane_id, &path, text.take(), cwd);
                }
                OpenHandler::View(id) => {
                    let Some(path) = arg(&id) else {
                        return;
                    };
                    if self.at_anchor(sender_pane_id, |app| app.launch_view_with_path(&id, &path)) {
                        log::info!("open: '{path}' → plugin '{id}' ({matched_by})");
                    } else if chosen.matched_by == MatchedBy::Manifest {
                        if let Some(next) = fallback.take() {
                            log::warn!(
                                "open: manifest handler '{id}' failed — continuing resolution for '{path}'"
                            );
                            classified = next;
                            continue;
                        }
                        log::warn!(
                            "open: manifest handler '{id}' failed — OS fallback for '{path}'"
                        );
                        let _logged = shell_open(target, false);
                    } else {
                        log::warn!("open: handler '{id}' unavailable — OS fallback for '{path}'");
                        let _logged = shell_open(target, false);
                    }
                }
            }
            return;
        }
    }

    /// Open `path` in the builtin text-editor as a `split_h` anchored at
    /// `anchor_pane_id`, so the editor lands to the right of the explorer (or
    /// terminal) that requested it. Mirrors the `SpawnPane` plugin-branch: it
    /// resolves the anchor's window from its pane id (never a global-focus
    /// read), retargets the launch to that window/tile, then restores the
    /// original active window. The `context_id` follows implicitly from
    /// anchoring on the pane's own window, so the split stays in the caller's
    /// context. The editor is built from `read`, made before this call.
    fn open_text_file_in_split(
        &mut self,
        anchor_pane_id: PaneId,
        path: &str,
        read: Option<crate::pane_ops::create::TextEditorRead>,
        cwd: std::path::PathBuf,
    ) {
        if let Some(existing_pane_id) =
            self.find_open_text_editor_pane_any_window(std::path::Path::new(path))
        {
            log::info!(
                "open: '{path}' already open in text-editor pane {existing_pane_id}, focusing"
            );
            self.pane_navigate(existing_pane_id);
            return;
        }
        let read = read.unwrap_or_else(|| {
            // The editor that held it when the open was prepared has closed
            // since; read now rather than open an empty document.
            log::info!("open: '{path}' lost its editor while opening; reading it now");
            crate::pane_ops::create::TextEditorRead::read(std::path::Path::new(path), &cwd)
        });
        // Built directly, so no `on_launch` dedup can swap in an editor on
        // another file: opening a specific file must always load THAT file.
        let app = crate::pane_ops::create::text_editor_from_read(path.into(), read);
        let placed = self.at_anchor(anchor_pane_id, |host| {
            host.open_built_text_editor(host.active_window, Some("split_h".to_string()), app, cwd)
        });
        if let Err(e) = placed {
            log::warn!("open: text-editor launch for '{path}' failed — {e}; OS fallback");
            let _logged = shell_open(std::path::Path::new(path), false);
        }
    }

    /// Run a launch from `anchor_pane_id`: its window becomes the launch
    /// target and its tile the split point, whatever is focused now; the
    /// active window is restored afterwards. A missing anchor launches in
    /// the active window.
    fn at_anchor<R>(&mut self, anchor_pane_id: PaneId, launch: impl FnOnce(&mut Self) -> R) -> R {
        let original_active_window = self.active_window;
        match self.find_pane_in_any_window(anchor_pane_id) {
            Some((win_idx, tile)) => {
                self.active_window = win_idx;
                self.set_window_focused_pane(win_idx, tile);
            }
            None => {
                log::warn!("open: anchor pane {anchor_pane_id} not found — using active window")
            }
        }
        let launched = launch(self);
        self.active_window = original_active_window;
        launched
    }

    /// Launch plugin `id` with `path` as its sole arg, using the plugin's natural
    /// placement (`None` layout → manifest `[launch] placement` → `overlay`).
    /// Returns `true` on a successful launch; `false` when the plugin is not
    /// available, so the resolver can fall through to the OS opener.
    fn launch_view_with_path(&mut self, id: &str, path: &str) -> bool {
        let layout = matches!(id, "image-viewer" | "video-player" | "audio-player")
            .then(|| "split_h".to_string());
        match self.launch_view_by_id_with_layout(id, layout, &[path.to_string()], None) {
            Ok(_) => true,
            Err(e) => {
                log::warn!("open: launch of plugin '{id}' for '{path}' failed — {e}");
                false
            }
        }
    }
}

fn linked_terminal_payload_log_message(
    terminal_pane_id: PaneId,
    payload: &str,
    echo: bool,
) -> String {
    format!(
        "RunInLinkedTerminal: pane {terminal_pane_id} payload_len={} (echo={echo})",
        payload.chars().count()
    )
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
pub(crate) fn quote_for_shell(path: &str) -> String {
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
    #[expect(
        clippy::disallowed_methods,
        reason = "detached: the browser must outlive the opener"
    )]
    let result = std::process::Command::new("open").arg(url).spawn();
    #[cfg(target_os = "linux")]
    #[expect(
        clippy::disallowed_methods,
        reason = "detached: the browser must outlive the opener"
    )]
    let result = std::process::Command::new("xdg-open").arg(url).spawn();
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let result: std::io::Result<std::process::Child> =
        Err(std::io::Error::other("URL open is macOS/Linux-only"));
    match result {
        Ok(_) => Ok(()),
        Err(e) => Err(format!("OS opener failed for {url}: {e}")),
    }
}

#[cfg(test)]
thread_local! {
    static TEST_OPENER: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// Replace the OS opener program on this thread, so tests never launch a
/// real application. `None` restores the platform opener.
#[cfg(test)]
pub(crate) fn set_test_opener(program: Option<&str>) {
    TEST_OPENER.with(|opener| *opener.borrow_mut() = program.map(str::to_string));
}

fn opener_program(platform: &str) -> String {
    #[cfg(test)]
    if let Some(program) = TEST_OPENER.with(|opener| opener.borrow().clone()) {
        return program;
    }
    platform.to_string()
}

/// Cross-platform helper: shell out to `open` (macOS) / `xdg-open`
/// (Linux). Other platforms have no opener yet and return an error. A spawn
/// failure is logged and returned so a caller with a receipt can report it.
pub(super) fn shell_open(path: &std::path::Path, reveal: bool) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let (program, args) = {
        let mut args: Vec<&std::ffi::OsStr> = Vec::new();
        if reveal {
            args.push("-R".as_ref());
        }
        args.push(path.as_os_str());
        (opener_program("open"), args)
    };
    #[cfg(target_os = "linux")]
    let (program, args) = {
        // `xdg-open` has no "reveal" flag; open the parent directory instead.
        let target = if reveal {
            path.parent().unwrap_or(path)
        } else {
            path
        };
        (opener_program("xdg-open"), vec![target.as_os_str()])
    };
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = opener_program;
        let error = format!("no OS opener on this platform for {path:?} (reveal={reveal})");
        log::warn!("OpenArtifact: {error}");
        return Err(error);
    }
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        #[expect(
            clippy::disallowed_methods,
            reason = "detached: the opened app must outlive the opener"
        )]
        let spawned = std::process::Command::new(&program).args(&args).spawn();
        match spawned {
            Ok(_) => {
                log::debug!("OpenArtifact: spawned `{program}` for {path:?} reveal={reveal}");
                Ok(())
            }
            Err(e) => {
                let error = format!("`{program}` failed for {path:?}: {e}");
                log::error!("OpenArtifact: {error}");
                Err(error)
            }
        }
    }
}

/// True when an already-canonical `checked` path stays inside the
/// canonicalized `workspace_root`. Complements the lexical `normalize_path`
/// check: a symlink inside the workspace pointing outside it passes the lexical
/// gate but fails here (#2242).
fn within_canonical_root(root: &std::path::Path, checked: &std::path::Path) -> bool {
    let canonical_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    checked.starts_with(&canonical_root)
}

/// Canonicalize the deepest existing ancestor of `path` (resolving any symlinks
/// on the way) and re-append the not-yet-existing tail. Symlinks can only live
/// on the existing prefix, so this catches a workspace-internal symlink that
/// redirects outside the sandbox even when the final target does not exist yet.
/// Falls back to the lexical normalization when nothing on the chain exists.
pub(super) fn canonicalize_existing_prefix(path: &std::path::Path) -> std::path::PathBuf {
    for ancestor in path.ancestors() {
        if let Ok(canonical) = ancestor.canonicalize() {
            return match path.strip_prefix(ancestor) {
                Ok(rest) if rest.as_os_str().is_empty() => canonical,
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
    use super::{
        canonicalize_existing_prefix, linked_terminal_payload_log_message, normalize_path,
        quote_for_shell, within_canonical_root,
    };
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
            !within_canonical_root(&workspace, &canonicalize_existing_prefix(&via_symlink)),
            "canonical check must reject a symlink escaping the workspace"
        );

        // A genuine in-workspace file is still accepted.
        let inside = workspace.join("ok.txt");
        std::fs::write(&inside, b"fine").unwrap();
        assert!(
            within_canonical_root(&workspace, &canonicalize_existing_prefix(&inside)),
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
    fn linked_terminal_payload_log_omits_plaintext() {
        let sentinel = "SENTINEL_SECRET_DO_NOT_LOG";
        let message = linked_terminal_payload_log_message(42, sentinel, true);
        assert!(!message.contains(sentinel), "{message}");
        assert_eq!(
            message,
            "RunInLinkedTerminal: pane 42 payload_len=26 (echo=true)"
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

//! Canvas Terminal Binding Primitives dispatch (#78).
//!
//! The primitives are routed by `process_app::routing::route_command` into
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
    ) {
        let active = self.active_window;

        // Resolve cwd: explicit > sender's workspace_root > home.
        let workspace_root = self
            .windows[active]
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
        let settings = Self::make_backend_settings(new_id, resolved_cwd, &self.colors, ctx_id, &ctx_name, &ctx_desc, ctx_root.as_ref(), ctx_depth);
        let Some(term) = TerminalPane::new(
            new_id,
            self.ctx.clone(),
            self.pty_event_tx.clone(),
            settings,
            self.default_font_size,
        ) else {
            log::error!(
                "RequestLinkedTerminal: failed to create TerminalPane for pane {new_id}"
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
        self.windows[active]
            .panes
            .insert(new_id, Pane::Terminal(Box::new(term)));

        // Split the tree directly, adjacent to sender_tile, without touching focused_pane.
        // Side-by-side (vertical=true) on the right: Canvas app left, terminal right.
        // focused_pane is deliberately NOT updated — the Canvas app retains keyboard focus
        // so the user doesn't lose input mid-flow.
        let share = crate::host::command::ShareRatio::new(1.0, 1.0)
            .expect("1:1 is a valid ShareRatio");
        let _new_tile = crate::pane_ops::insert_split_tile(
            &mut self.windows[active].tree,
            Some(sender_tile),
            new_id,
            true,  // vertical = side-by-side on right
            share,
            false, // new_pane_first = false
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
        log::debug!(
            "RunInLinkedTerminal: pane {terminal_pane_id} ← {payload:?} (echo={echo})"
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
        log::debug!(
            "InsertPathToken: pane {terminal_pane_id} mode={mode:?} path={path:?}"
        );
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
        // Note: validation is lexical (normalize_path collapses ".." without I/O).
        // Symlinks inside the workspace root that point outside are treated as
        // trusted, consistent with OS-level file permission semantics.
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
        }
        log::info!("OpenArtifact: path={path:?} mode={mode:?}");
        match mode {
            ArtifactOpenMode::OpenInPane => {
                // For v3.5: directories open the file browser; files fall
                // through to OpenWithDefault. The proper "open .py in text
                // editor / .png in image viewer" routing wants a content-
                // type registry — out of scope here.
                let p = std::path::PathBuf::from(&path);
                if p.is_dir() {
                    // Reuse the file-browser open path, but anchored at
                    // the requested cwd. The current `open_file_browser`
                    // anchors at the focused pane's cwd; we replicate
                    // its body here with our path.
                    self.open_file_browser_at(p);
                } else {
                    shell_open(&path, false);
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
        let app: Box<dyn App> = self
            .registry
            .launch("file_browser", &cwd, &[])
            .unwrap_or_else(|| Box::new(crate::file_browser::FileBrowserApp::new(cwd.clone())));
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
    let safe = path.chars().all(|c| {
        c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '-' | '_' | ':' | '@' | '+')
    });
    if safe {
        path.to_string()
    } else {
        // POSIX single-quote escape: every existing single quote becomes
        // '\'' (close, escaped quote, reopen).
        let escaped = path.replace('\'', "'\\''");
        format!("'{escaped}'")
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
            Ok(_) => log::debug!("OpenArtifact: spawned `open{}` for {path}", if reveal { " -R" } else { "" }),
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
    use super::{normalize_path, quote_for_shell};
    use std::path::PathBuf;

    #[test]
    fn normalize_path_collapses_parent_dirs() {
        assert_eq!(normalize_path(&PathBuf::from("/a/b/../c")), PathBuf::from("/a/c"));
        assert_eq!(normalize_path(&PathBuf::from("/a/b/c/../../d")), PathBuf::from("/a/d"));
        assert_eq!(normalize_path(&PathBuf::from("/a/./b/../c/.")), PathBuf::from("/a/c"));
    }

    #[test]
    fn normalize_path_cannot_escape_above_root() {
        // Pop at the root component is a no-op — result stays at /etc.
        assert_eq!(normalize_path(&PathBuf::from("/../etc")), PathBuf::from("/etc"));
    }

    #[test]
    fn normalize_path_traversal_is_blocked_by_workspace_check() {
        let root = PathBuf::from("/workspace");
        let escaped = normalize_path(&PathBuf::from("/workspace/../../etc/passwd"));
        assert!(!escaped.starts_with(&root), "traversal should escape workspace: {escaped:?}");
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

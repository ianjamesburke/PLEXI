use crate::media_cache::MediaCache;
use crate::theme::Colors;
use crate::tiling::PaneId;
use std::cell::RefCell;
use std::path::PathBuf;

/// Context passed to an app during rendering.
pub struct AppRenderContext<'a> {
    pub colors: &'a Colors,
    pub is_focused: bool,
    pub linked_terminal: PaneId,
    /// Shared image/video thumbnail cache, owned by `PlexiApp` and reused
    /// across every app on every frame. Borrowed mutably during rendering.
    pub media_cache: &'a RefCell<MediaCache>,
}

/// Commands an app can issue back to the system.
pub enum AppCommand {
    /// Write a shell command to the linked terminal.
    RunInTerminal(String),
    /// Change the terminal's working directory.
    Cd(PathBuf),
    /// Post an ephemeral notification.
    Notify(String),
    /// Descend into a child `.plexi` depth boundary.
    DescendDepth(PathBuf),
}

/// The trait all Plexi apps implement.
///
/// Apps live inside a TerminalPane. The terminal shrinks to a command bar at the
/// bottom of the pane while the app occupies the main area. Escape dismisses the
/// app entirely; Tab toggles the terminal between a command bar and a 50% split.
pub trait App: Send {
    /// Unique stable identifier, e.g. `"file_browser"`. Used for serialisation.
    fn type_id(&self) -> &'static str;

    /// Human-readable display name shown in the pane title.
    fn display_name(&self) -> String;

    /// Render the app into the given Ui region.
    fn ui(&mut self, ui: &mut egui::Ui, ctx: &AppRenderContext<'_>);

    /// Handle raw key input before it reaches the terminal.
    /// Return `true` to consume the event (prevents terminal from seeing it).
    fn handle_key(&mut self, _input: &egui::InputState) -> bool {
        false
    }

    /// Drain any commands the app has queued since the last call.
    /// Apps that navigate internally (e.g. file browser) push commands here
    /// so the host can act on them each frame without changing the trait signature.
    fn take_pending_commands(&mut self) -> Vec<AppCommand> {
        vec![]
    }

    /// Drain any `spawn_app` requests the app has queued since the last call.
    ///
    /// This is a separate channel from `take_pending_commands` so the existing
    /// `AppCommand` enum stays source-compatible with callers that only know
    /// about RunInTerminal / Cd / Notify. The host's spawn dispatcher walks
    /// this queue, validates each request against the registry and target
    /// app's `[app.spawnable]` manifest table, and turns allowed entries into
    /// real panes plus `SpawnRelationships` records.
    ///
    /// Default returns empty — only `ProcessApp` and apps that model
    /// `spawn_app` natively need to override this.
    fn take_pending_spawns(&mut self) -> Vec<crate::app_protocol::PendingSpawn> {
        vec![]
    }

    /// Drain any `pipe_write` commands the app has queued since the last call.
    ///
    /// Returns (channel, value) pairs for the host pipe dispatcher to route
    /// to connected apps (parent and/or children via spawn relationships).
    /// Default returns empty — only `ProcessApp` overrides this.
    fn take_pipe_writes(&mut self) -> Vec<(String, serde_json::Value)> {
        vec![]
    }

    /// Deliver a `PipeData` event from a connected app to this app.
    ///
    /// Called by the host pipe dispatcher after routing a `PipeWrite` from a
    /// connected peer. Default no-ops — only `ProcessApp` overrides this.
    fn send_pipe_data(&mut self, _from_app: &str, _channel: &str, _value: &serde_json::Value) {}

    /// Called when the user submits a command via the terminal command bar.
    /// The app may interpret it and return a command to execute, or return `None`
    /// to let it pass through to the terminal as a normal shell command.
    fn on_command(&mut self, _cmd: &str) -> Option<AppCommand> {
        None
    }

    /// File extensions this app handles (lowercase, no dot). Used for file-type
    /// routing from the file browser. Empty slice means not file-driven.
    fn accepted_extensions(&self) -> &[&str] {
        &[]
    }

    /// Returns true if the app wants to close itself (e.g. after saving).
    fn wants_close(&self) -> bool {
        false
    }

    /// Called when the linked terminal's CWD changes.
    /// Apps that track directories (like the file browser) should update.
    fn sync_cwd(&mut self, _new_cwd: &std::path::Path) {}

    /// Offer dropped files to the app. Return `true` if the app consumed the
    /// drop (one of its declared drop targets contained `local_pos`). When
    /// `true`, the host will NOT fall back to the default terminal-paste
    /// behaviour for these paths.
    ///
    /// `local_pos` is in the app's local coordinate space (same origin as
    /// DrawCommand coordinates — top-left of the app surface).
    fn handle_drop(&mut self, _local_pos: egui::Pos2, _paths: &[PathBuf]) -> bool {
        false
    }

    /// The directory this app is currently focused on, if any. Used by the host
    /// to propagate the final navigation directory back to the underlying terminal
    /// when the app closes (e.g. the file browser's final cwd).
    fn current_dir(&self) -> Option<&std::path::Path> {
        None
    }

    /// Serialise app state to JSON for workspace persistence.
    fn serialize_state(&self) -> Option<serde_json::Value> {
        None
    }

    /// Restore app state from a previously serialised value.
    fn restore_state(&mut self, _state: &serde_json::Value) {}

    /// Trigger undo (pop previous state from undo stack and restore).
    /// Only meaningful for ProcessApps that support the get_state/set_state protocol.
    fn undo(&mut self) {}

    /// Returns true if this app wants to receive mouse-move events.
    /// Default: false (opt-in via manifest or `MouseTracking` draw command).
    fn mouse_tracking_enabled(&self) -> bool {
        false
    }

    /// Forward a mouse-down event. `button` is one of `"left"`, `"right"`, `"middle"`.
    fn send_mouse_down(&mut self, _x: f32, _y: f32, _button: &str) {}

    /// Forward a mouse-up event. `button` is one of `"left"`, `"right"`, `"middle"`.
    fn send_mouse_up(&mut self, _x: f32, _y: f32, _button: &str) {}

    /// Forward a mouse-move event (only called when `mouse_tracking_enabled()` returns true).
    fn send_mouse_move(&mut self, _x: f32, _y: f32) {}

    /// Forward a scroll event. `x/y` is the cursor position; `delta_x/delta_y` is the scroll delta.
    fn send_scroll(&mut self, _x: f32, _y: f32, _delta_x: f32, _delta_y: f32) {}

    /// Trigger redo (pop next state from redo stack and restore).
    fn redo(&mut self) {}
}

/// Whether the app surface or the terminal command bar has keyboard focus.
/// Only relevant when `SurfaceMode::AppActive`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SurfaceLayer {
    /// App receives keyboard input. App is fully opaque.
    App,
    /// Terminal command bar receives keyboard input. App dims to signal background state.
    Terminal,
}

/// Whether an app surface is active on this pane.
#[derive(Clone, Copy, PartialEq)]
pub enum SurfaceMode {
    /// No app active — terminal fills the whole pane.
    FullTerminal,
    /// App active — occupies the entire pane. The app is in a separate tile
    /// from any companion terminal (legacy auto-split behavior).
    AppActive,
    /// App active in the same pane as the existing terminal. The app overlays
    /// the top `1.0 - companion_fraction` of the pane area; the original
    /// terminal continues to render in the bottom `companion_fraction`.
    /// No new TerminalPane is created — the user's existing terminal session
    /// is preserved, including history and running processes.
    AppWithCompanion { companion_fraction: f32 },
}

impl SurfaceMode {
    pub fn has_app(self) -> bool {
        matches!(self, SurfaceMode::AppActive | SurfaceMode::AppWithCompanion { .. })
    }
}

/// Default fraction of the pane height the embedded companion terminal
/// occupies when an app is opened in `AppWithCompanion` mode.
pub const DEFAULT_COMPANION_FRACTION: f32 = 0.25;

/// Opacity applied to the app region when the terminal command bar has focus.
pub const APP_DIM_OPACITY: f32 = 0.45;

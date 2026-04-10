use crate::theme::Colors;
use crate::tiling::PaneId;
use std::path::PathBuf;

/// Context passed to an app during rendering.
pub struct AppRenderContext<'a> {
    pub colors: &'a Colors,
    pub is_focused: bool,
    pub linked_terminal: PaneId,
}

/// Commands an app can issue back to the system.
pub enum AppCommand {
    /// Write a shell command to the linked terminal.
    RunInTerminal(String),
    /// Change the terminal's working directory.
    Cd(PathBuf),
    /// Post an ephemeral notification.
    Notify(String),
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

    /// Serialise app state to JSON for workspace persistence.
    fn serialize_state(&self) -> Option<serde_json::Value> {
        None
    }

    /// Restore app state from a previously serialised value.
    fn restore_state(&mut self, _state: &serde_json::Value) {}
}

/// How much of the pane the terminal occupies when an app is active.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SurfaceMode {
    /// No app active — terminal fills the whole pane.
    FullTerminal,
    /// App fills most of the pane; terminal is a 4-line command bar at the bottom.
    AppWithCommandBar,
    /// App and terminal each take 50% of the pane height.
    AppWithTerminalSplit,
}

impl SurfaceMode {
    /// Fraction of the pane height the terminal occupies.
    pub fn terminal_fraction(self) -> f32 {
        match self {
            SurfaceMode::FullTerminal => 1.0,
            SurfaceMode::AppWithCommandBar => 0.0, // command bar height is fixed, not fractional
            SurfaceMode::AppWithTerminalSplit => 0.5,
        }
    }
}

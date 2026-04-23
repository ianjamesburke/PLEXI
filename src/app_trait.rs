use crate::theme::Colors;

/// Context passed to an app during rendering.
pub struct AppRenderContext<'a> {
    pub colors: &'a Colors,
    pub is_focused: bool,
}

/// Commands an app can issue back to the system.
pub enum AppCommand {
    /// Post an ephemeral notification.
    Notify(String),
    /// Request the host to spawn a new app pane.
    /// `layout`: "split_v" (below, default), "split_h" (right), or "overlay".
    /// `args`: passed as argv to the child process.
    SpawnApp {
        type_id: String,
        layout: Option<String>,
        args: Vec<String>,
    },
    /// Request the host to cd sibling terminals (same split container) to `cwd`.
    CdRequest { cwd: String, sender_pane_id: u64 },
    /// Deliver a JSON pipe message to all peer panes that have the given
    /// pipe_id open with direction In or Duplex. The sender pane is excluded.
    DeliverPipeMessage {
        sender_pane_id: u64,
        pipe_id: String,
        payload: serde_json::Value,
    },
    /// Deliver a RunUpdate event to the pane that originally issued the run,
    /// identified by its type_id.
    DeliverRunUpdate {
        originator_type_id: String,
        event: crate::app_protocol::PlexiEvent,
    },
    /// A notification that carries a notify_id and awaits a user response.
    /// The legacy server-side `NotificationAction` list is handled in
    /// `routing.rs` as side effects (resume_run / open_intent / run_command)
    /// BEFORE this command is emitted — it does not participate in the UI.
    /// User-facing buttons are carried via `kind = "choice"` + `options`.
    ShowNotification {
        notify_id: String,
        sender_pane_id: u64,
        level: String,
        title: String,
        body: String,
        kind: crate::app_protocol::NotifyKind,
        options: Vec<crate::app_protocol::NotifyOption>,
        input_prompt: Option<String>,
        required: bool,
        /// Higher = more urgent. Used to pick the next front-most notification
        /// after dismiss, and to order preview cycling via Cmd+] / Cmd+[.
        /// Insertion order breaks ties (oldest first).
        priority: u32,
    },
    /// Route a NotifyAction event back to the app pane that sent the Notify.
    DeliverNotifyAction {
        pane_id: u64,
        notify_id: String,
        action_label: String,
        value: Option<String>,
    },
}

/// The trait all Plexi apps implement.
///
/// Apps live inside `Pane::App` runtimes.
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

    /// Returns true if this app wants to capture all keyboard input, preventing
    /// host shortcuts (Cmd+HJKL, Cmd+Enter, etc.) from firing while it is focused.
    /// Only `Cmd+Q` and `Cmd+W` remain active when capture is true.
    fn keyboard_capture(&self) -> bool {
        false
    }

    /// Returns true if the app wants to close itself (e.g. after saving).
    fn wants_close(&self) -> bool {
        false
    }

    /// Called when the linked terminal's CWD changes.
    /// Apps that track directories (like the file browser) should update.
    fn sync_cwd(&mut self, _new_cwd: &std::path::Path) {}

    /// Queue a PlexiEvent to be sent to the app on the next flush.
    /// Used to deliver host-originated events (e.g. AppSpawned) back to
    /// external process apps. Built-in apps ignore this by default.
    fn queue_outbound_event(&mut self, _event: crate::app_protocol::PlexiEvent) {}

    /// Serialise app state to JSON for workspace persistence.
    fn serialize_state(&self) -> Option<serde_json::Value> {
        None
    }

    /// Restore app state from a previously serialised value.
    fn restore_state(&mut self, _state: &serde_json::Value) {}
}

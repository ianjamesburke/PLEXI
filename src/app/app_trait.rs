use crate::ui::theme::Colors;

/// The result of an overlay's or app's keyboard handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KeyDisposition {
    /// The handler consumed the event; downstream handlers must not run.
    Consumed,
    /// The handler did not consume the event; pass to the next handler.
    Passthrough,
}

/// Context passed to an app during rendering.
pub struct AppRenderContext<'a> {
    pub colors: &'a Colors,
    /// Whether this pane is the currently focused pane in its window.
    pub is_focused: bool,
}

/// Commands an app can issue back to the system.
pub enum AppCommand {
    /// Post an ephemeral notification.
    Notify(String),
    /// Request the host to spawn a new app pane.
    /// `layout`: "split_h" (right), "split_v" (below, default), or "overlay".
    /// `args`: passed as argv to the child process.
    SpawnApp {
        type_id: String,
        layout: Option<String>,
        args: Vec<String>,
    },
    /// Unified spawn (#592). Mirrors DrawCommand::SpawnPane after capability check.
    SpawnPane {
        type_id: String,
        layout: String,
        args: Vec<String>,
        pipe_id: Option<String>,
        from_pane_id: Option<u64>,
        request_id: Option<String>,
        target_context: Option<u64>,
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
    /// Open a *directed* JSON pipe (#286) — only the sender and the named
    /// target pane subscribe to subsequent `PipeMessage` deliveries.
    /// Routed by the host because the target pane lives outside the sender's
    /// process and the sender has no other way to subscribe peers.
    OpenDirectedPipe {
        sender_pane_id: u64,
        pipe_id: String,
        target_pane_id: u64,
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
        /// Stable context identity the notification originated from.
        source_context_id: u64,
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
        /// Visibility scope. `Global` notifications are always visible;
        /// `Context` notifications are only visible in their source context.
        scope: crate::app_protocol::NotifyScope,
        /// Inline base64-encoded image attachment (#74). Decoded + cached
        /// into a texture on first render. Decoded size > 50 KB triggers a
        /// placeholder badge instead — never crash the host on bad input.
        image_inline: Option<crate::app_protocol::NotificationImage>,
        /// Pipe-referenced image (#74). Drained from the binary ring lazily
        /// when the notification is visible. Layout: `width: u32 LE`,
        /// `height: u32 LE`, then RGBA bytes. Mutually exclusive with
        /// `image_inline` — if both set, inline wins.
        image_pipe_id: Option<String>,
        timeout_secs: Option<u64>,
        on_dismiss: Option<String>,
    },
    /// Route a NotifyAction event back to the app pane that sent the Notify.
    DeliverNotifyAction {
        pane_id: u64,
        notify_id: String,
        action_label: String,
        value: Option<String>,
        /// Path to write the chosen value for host-originated blocking notifications.
        response_file: Option<String>,
        /// Host-side action to execute synchronously. Format: `"action_type:action_arg"`.
        host_action: Option<String>,
    },
    /// Canvas Terminal Binding Primitives (#78). The host opens a fresh
    /// terminal next to `sender_pane_id`, sets the new terminal as the
    /// sender app's `linked_pane_id`, and emits
    /// `PlexiEvent::LinkedTerminalReady { request_id, terminal_pane_id }`
    /// back to the sender. `cwd` falls back to the sender's
    /// `workspace_root` when `None`.
    RequestLinkedTerminal {
        sender_pane_id: u64,
        request_id: String,
        cwd: Option<String>,
        label: Option<String>,
    },
    /// Canvas Terminal Binding Primitives (#78). Inject `command` into the
    /// referenced terminal's PTY. With `echo: true`, the command is
    /// followed by `\n` so the shell executes it; the user sees the typed
    /// command and its output. With `echo: false`, the host still writes
    /// the command + newline (PTY-level echo is shell-controlled — we don't
    /// suppress it from the host side; the flag is preserved on the wire
    /// so a future revision can wire shell-aware silent-execute).
    RunInLinkedTerminal {
        sender_pane_id: u64,
        terminal_pane_id: u64,
        command: String,
        echo: bool,
    },
    /// Canvas Terminal Binding Primitives (#78). Inject a path token at
    /// the referenced terminal's cursor. `Replace` mode prefixes a
    /// Ctrl-W (kill-word) so the shell's readline removes the partial
    /// word before the path is written. Paths containing shell
    /// metacharacters are POSIX-quoted by the host before injection.
    InsertPathToken {
        sender_pane_id: u64,
        terminal_pane_id: u64,
        path: String,
        mode: crate::app_protocol::PathTokenMode,
    },
    /// Canvas Terminal Binding Primitives (#78). Compute a no-execute
    /// preview of `command` for the referenced terminal. Host responds
    /// with `PlexiEvent::CommandPreview { request_id, command,
    /// would_run_in_cwd }` to `sender_pane_id`. `would_run_in_cwd` is the
    /// host's best-effort snapshot of the terminal child's cwd.
    RequestCommandPreview {
        sender_pane_id: u64,
        request_id: String,
        terminal_pane_id: u64,
        command: String,
    },
    /// Canvas Terminal Binding Primitives (#78). Open a workspace
    /// artifact via the host's pane router (OpenInPane → file browser
    /// for dirs, Launch Services for files), or shell out to `open`
    /// with `-R` (RevealInFinder) / no flag (OpenWithDefault) on macOS.
    OpenArtifact {
        sender_pane_id: u64,
        path: String,
        mode: crate::app_protocol::ArtifactOpenMode,
    },
    /// Query rolled-up ContextState for a context (#1518).
    /// Forwarded to the host because only it has the full context tree.
    QueryContextState {
        sender_pane_id: u64,
        context_id: u64,
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
    /// Return `Consumed` to prevent downstream handlers from seeing the event.
    fn handle_key(&mut self, _input: &egui::InputState) -> KeyDisposition {
        KeyDisposition::Passthrough
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

    /// Returns the app's current working directory, if it tracks one.
    /// Used to sync CWD back to the terminal when an overlay app closes.
    fn current_cwd(&self) -> Option<std::path::PathBuf> {
        None
    }

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

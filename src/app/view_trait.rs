use crate::ui::theme::Colors;

/// The result of an overlay's or plugin's keyboard handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KeyDisposition {
    /// The handler consumed the event; downstream handlers must not run.
    Consumed,
    /// The handler did not consume the event; pass to the next handler.
    Passthrough,
}

/// Context passed to an plugin during rendering.
pub struct ViewRenderContext<'a> {
    pub colors: &'a Colors,
    /// Host pane id of the pane being rendered. Plugins use it to register their
    /// text surfaces against `SurfaceKey::Pane(pane_id)` so the post-frame
    /// focus reconciler can project host input ownership onto egui focus.
    pub pane_id: u64,
}

/// Commands an plugin can issue back to the system.
pub enum ViewCommand {
    Fleet(crate::app::agents_view::FleetAction),
    /// A capability-gated effect returned by a WASM component. The plugin model
    /// executes it through the same host services that native plugins use.
    WasmHostEffect {
        pane_id: u64,
        effect: crate::host::wasm_pane::WasmHostEffect,
    },
    /// Register the plugin's v3.7 connector tools with the host broker.
    ExposeTools {
        tools: Vec<crate::protocol::AiTool>,
        pane_id: Option<u64>,
    },
    /// Resolve a pending v3.7 connector tool call.
    ToolResult {
        call_id: String,
        output_json: Option<String>,
        error: Option<String>,
        error_code: Option<String>,
    },
    /// Route one plugin-event request through the host-owned timeline and broker.
    PluginEventRequest {
        request: crate::protocol::HostRequest,
        pane_id: Option<u64>,
    },
    /// Execute an Assistant host tool against live host state without shelling
    /// out to the CLI. The host replies directly to the blocked model worker.
    AssistantHostTool {
        name: String,
        input_json: String,
        origin_pane_id: u64,
        origin_context_id: u64,
        reply: std::sync::mpsc::SyncSender<crate::plexi_ai::tool_dispatch::ToolCallResult>,
    },
    /// Request the host to spawn a new view pane.
    /// `layout`: "split_h" (right), "split_v" (below, default), or "overlay".
    /// `args`: passed as argv to the child process.
    SpawnView {
        type_id: String,
        layout: Option<String>,
        args: Vec<String>,
    },
    /// Unified spawn (#592). Mirrors DrawCommand::SpawnPane after capability check.
    SpawnPane {
        type_id: String,
        layout: String,
        args: Vec<String>,
        from_pane_id: Option<u64>,
        request_id: Option<String>,
        target_context: Option<u64>,
    },
    /// Forward a capability-gated pane read/control `HostRequest` (ListPanes,
    /// FocusPane, SendToPane, ...) into the host's pane-IPC handler — the
    /// same code path CLI requests take over PLEXI_SOCKET (stint 0013/0014).
    /// Emitted by `routing.rs` after the `panes.read` / `panes.control`
    /// capability check passes.
    ForwardPaneRequest {
        request: crate::protocol::HostRequest,
    },
    /// Request the host to cd sibling terminals (same split container) to `cwd`.
    CdRequest { cwd: String, sender_pane_id: u64 },
    /// Deliver a JSON pipe message on a directed pipe (#286): the host routes
    /// it to the non-sender member of the recorded `(sender, target)` pair.
    /// A send on a pipe that was never opened directed is dropped — the legacy
    /// non-directed peer-broadcast fan-out was removed in 0327.
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
        event: crate::protocol::PlexiEvent,
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
        title: String,
        body: String,
        kind: crate::protocol::NotifyKind,
        options: Vec<crate::protocol::NotifyOption>,
        input_prompt: Option<String>,
        required: bool,
        /// Visibility scope. `Global` notifications are always visible;
        /// `Context` notifications are only visible in their source context.
        scope: crate::protocol::NotifyScope,
        /// Inline base64-encoded image attachment (#74). Decoded + cached
        /// into a texture on first render. Decoded size > 50 KB triggers a
        /// placeholder badge instead — never crash the host on bad input.
        image_inline: Option<crate::protocol::NotificationImage>,
        /// Pipe-referenced image (#74). Drained from the binary ring lazily
        /// when the notification is visible. Layout: `width: u32 LE`,
        /// `height: u32 LE`, then RGBA bytes. Mutually exclusive with
        /// `image_inline` — if both set, inline wins.
        image_pipe_id: Option<String>,
        timeout_secs: Option<u64>,
        on_dismiss: Option<String>,
    },
    /// Route a NotifyAction event back to the view pane that sent the Notify.
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
    /// sender plugin's `linked_pane_id`, and emits
    /// `PlexiEvent::LinkedTerminalReady { request_id, terminal_pane_id }`
    /// back to the sender. `cwd` falls back to the sender's
    /// `workspace_root` when `None`.
    RequestLinkedTerminal {
        sender_pane_id: u64,
        request_id: String,
        cwd: Option<String>,
        label: Option<String>,
        /// Where to place the new terminal relative to the requesting plugin
        /// pane. `false` (default) splits side-by-side with the terminal on
        /// the right; `true` stacks it underneath. Process plugins use the
        /// side-by-side default; the CLI renderer requests `true` so its form
        /// sits above its output terminal.
        place_below: bool,
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
        mode: crate::protocol::PathTokenMode,
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
        mode: crate::protocol::ArtifactOpenMode,
    },
    /// Query rolled-up ContextState for a context (#1518).
    /// Forwarded to the host because only it has the full context tree.
    QueryContextState {
        sender_pane_id: u64,
        context_id: u64,
    },
}

/// The trait all Plexi plugins implement.
///
/// Plugins live inside `Pane::App` runtimes.
/// Routing and ledger tags for one external Assistant turn.
///
/// `conversation_id` keeps an external turn off the desktop transcript.
/// `join_desktop` opts into that transcript. `client` and `kind` are the
/// ledger tags from `assistant send --client/--kind`.
pub struct ExternalTurnOpts {
    pub conversation_id: Option<String>,
    pub join_desktop: bool,
    pub status_for: Option<String>,
    pub client: Option<String>,
    pub kind: Option<String>,
}

pub trait View: Send {
    /// Unique stable identifier, e.g. `"file_browser"`. Used for serialisation.
    fn type_id(&self) -> &'static str;

    /// Pending approval this plugin is showing, when it is an approval surface.
    fn approval_request_id(&self) -> Option<String> {
        None
    }

    /// Human-readable display name shown in the pane title.
    fn display_name(&self) -> String;

    /// Render the plugin into the given Ui region. `pending_click` (stint 0469) is
    /// a synthetic pointer click queued by `plexi pane click --node` /
    /// `HostHarness::inject_node_click`, delivered the same frame a real click's
    /// hit-test result would reach. Plugins that expose activatable widgets to
    /// node-targeted clicks match it against their own `egui::Id`s (see File
    /// Explorer in `src/file_browser/mod.rs`); plugins that don't ignore it.
    fn ui(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &ViewRenderContext<'_>,
        pending_click: Option<crate::host::pane::PendingPaneClick>,
    );

    /// Handle raw key input before the plugin's own render pass.
    /// Return `Consumed` to prevent downstream handlers from seeing the event.
    ///
    /// Reads from the frame's [`crate::app::input_router::PlexiInput`] ownership
    /// buffer (via `input.key_pressed(..)` / `input.events()` /
    /// `input.modifiers()`) rather than `ctx.input()`, so an plugin and the global
    /// hotkey allowlist can never both claim the same key.
    fn handle_key(&mut self, _input: &crate::app::input_router::PlexiInput) -> KeyDisposition {
        KeyDisposition::Passthrough
    }

    /// Drain any commands the plugin has queued since the last call.
    /// Plugins that navigate internally (e.g. file browser) push commands here
    /// so the host can act on them each frame without changing the trait signature.
    fn take_pending_commands(&mut self) -> Vec<ViewCommand> {
        vec![]
    }

    /// Bind host-owned runtime identity before any external work is serviced.
    fn bind_host_identity(&mut self, _pane_id: u64, _context_id: u64) {}

    /// Pump asynchronous work while the plugin's context is off-screen.
    fn background_tick(&mut self) {}

    /// Whether `background_tick` can currently make progress.
    fn needs_background_tick(&self) -> bool {
        false
    }

    /// Returns true if this plugin wants to capture all keyboard input, preventing
    /// host shortcuts (Cmd+HJKL, Cmd+Enter, etc.) from firing while it is focused.
    /// Only `Cmd+Q` and `Cmd+W` remain active when capture is true.
    fn keyboard_capture(&self) -> bool {
        false
    }

    /// Whether bare Enter submits the current pane action even while one of its
    /// declarative text inputs owns egui focus. Plugins opt in only where Enter
    /// has no text-editing meaning; the dispatcher delivers that key before
    /// returning the rest of the input buffer to the focused `TextEdit`.
    fn submit_on_focused_text_input_enter(&self) -> bool {
        false
    }

    /// Returns true if the plugin wants to close itself (e.g. after saving).
    fn wants_close(&self) -> bool {
        false
    }

    /// A user-initiated close asks the plugin first. Returning false vetoes the
    /// close: the plugin shows its own prompt (unsaved buffer) and sets
    /// `wants_close` once the user decides. Normal desktop quit also asks;
    /// programmatic closes (CLI, context close, forced host stop) do not.
    fn request_close(&mut self) -> bool {
        true
    }

    /// Off-paint servicing of external state the plugin mirrors (files on disk).
    /// Runs from `App::logic` every pass for every pane, including panes in
    /// hidden windows and inactive contexts, so it must be cheap when idle.
    /// Plugins arm their own watcher lazily with `wake`. Returns true when
    /// visible state changed.
    fn service_external(&mut self, _wake: &std::sync::Arc<dyn crate::app::ui_mailbox::UiWake>) -> bool {
        false
    }

    /// Called when the linked terminal's CWD changes.
    /// Plugins that track directories (like the file browser) should update.
    fn sync_cwd(&mut self, _new_cwd: &std::path::Path) {}

    /// Queue a PlexiEvent to be sent to the plugin on the next flush.
    /// Used to deliver host-originated events (e.g. ViewOpened) back to
    /// external process plugins. Built-in plugins ignore this by default.
    fn queue_outbound_event(&mut self, _event: crate::protocol::PlexiEvent) {}

    /// Host-owned disabled view retains the original plugin record without executing it.
    fn disabled_plugin_record(&self) -> Option<&crate::workspace::SavedPane> {
        None
    }

    /// Serialise plugin state to JSON for workspace persistence.
    fn serialize_state(&self) -> Option<serde_json::Value> {
        None
    }

    /// Restore plugin state from a previously serialised value.
    fn restore_state(&mut self, _state: &serde_json::Value) {}

    /// The plugin's current internal font size in points, if supported.
    #[cfg(test)]
    fn font_size(&self) -> Option<f32> {
        None
    }

    /// Adjust the plugin's internal font size by `delta` points (positive = larger).
    /// Only implemented by plugins with their own font size, e.g. TextEditorView, FileBrowserView.
    fn adjust_font_size(&mut self, _delta: f32) {}

    /// Reset the plugin's internal font size to its default value.
    fn reset_font_size(&mut self) {}

    /// Seed text for the rename-pane overlay. `None` falls back to the pane's
    /// current name. TextEditorView returns the note's frontmatter title so
    /// renaming edits the title rather than the timestamp filename.
    fn rename_seed(&self) -> Option<String> {
        None
    }

    /// Called when the user renames this plugin's pane. TextEditorView persists
    /// the new name into the note's frontmatter `title`.
    fn on_pane_renamed(&mut self, _name: &str) {}

    /// The note this plugin currently has open, if any. The command palette
    /// lists notes that are open in a pane, so it asks every builtin plugin
    /// rather than scanning the notes dir. Plugins that are not note editors —
    /// and TextEditorView on an ordinary file — return `None`.
    fn open_note_path(&self) -> Option<&std::path::Path> {
        None
    }

    /// Read-only, plugin-owned semantic details appended to `plexi pane state`.
    /// Production state is authoritative; callers may only serialize it.
    fn semantic_state(&self) -> Option<serde_json::Value> {
        None
    }

    /// Submit a text turn through a builtin plugin's normal interactive path.
    /// Only the host Assistant implements this; the response path is owned by
    /// the plugin so callers cannot bypass its model or permission loop.
    fn submit_external_turn(
        &mut self,
        _text: String,
        _request_id: String,
        _response_file: String,
        _opts: ExternalTurnOpts,
    ) -> Result<(), String> {
        Err("this plugin does not accept external turns".to_string())
    }

    /// Deliver a host file/URL drop through the plugin's production handler.
    fn drop_file(&mut self, _path_or_url: &str) -> Result<serde_json::Value, String> {
        Err("this plugin does not accept file drops".to_string())
    }

    /// Pull a prepared file edit into this pane. The text editor previews it
    /// and leaves the buffer alone until a pointer click accepts it.
    fn sync_change_set(&mut self) -> bool {
        false
    }

    /// Open-buffer snapshot for change-set conflict checks. Other plugins have none.
    fn editor_buffer(&self) -> Option<crate::host::changes::OpenEditorBuffer> {
        None
    }

    /// Receive the host-owned fleet projection without disk or socket reads.
    fn sync_agents_view(&mut self, _view: &crate::agent::fleet::AgentsView) {}

    /// True once after a builtin editor saves `config.toml` so the host can
    /// immediately reload config and `[keys]` bindings without waiting on a
    /// filesystem watcher or restart.
    fn take_config_saved(&mut self) -> bool {
        false
    }

    /// Switch an open `commands.toml` editor between workspace and global scope.
    fn switch_commands_scope(&mut self) -> bool {
        false
    }

    /// True when this builtin editor currently owns keyboard shortcuts like
    /// `Cmd+/` (toggle line comment) so the host does not intercept them.
    fn captures_editor_shortcuts(&self) -> bool {
        false
    }

    /// Concrete-type access for host tests that assert on plugin-internal state
    /// through `ViewRuntime::Builtin`'s `dyn App`. Test-only: the sole consumer
    /// is `HostHarness` (`src/testing/`), which is `#[cfg(test)]`.
    #[cfg(test)]
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;
}

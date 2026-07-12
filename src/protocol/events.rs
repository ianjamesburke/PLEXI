use super::primitives::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PlexiEvent {
    /// Sent exactly once on startup. App must reply with DrawCommand::Ready.
    Init {
        /// Protocol version string, e.g. "pgap/3". App must refuse unknown versions.
        protocol: String,
        /// Stable identifier for this app instance, e.g. "audio-recorder".
        app_id: String,
        /// The workspace root this app was launched from.
        /// Hard invariant: all SecretGet calls are scoped to this directory.
        workspace_root: PathBuf,
        /// Capabilities granted to this app (declared in manifest or runtime-prompted).
        /// e.g. ["audio.record", "fs.read"]
        capabilities: Vec<String>,
        /// Additive feature flags. Unknown flags are ignored.
        /// e.g. ["media_v1", "pane_groups_v1"]
        feature_flags: Vec<String>,
        /// compact/regular threshold sent to SDK so ctx.size_class matches host behaviour.
        /// Defaults match SDK constants COMPACT_DEFAULT (280) / REGULAR_DEFAULT (480).
        #[serde(default = "default_compact_threshold")]
        compact_threshold: f32,
        #[serde(default = "default_regular_threshold")]
        regular_threshold: f32,
        /// Active host theme as a `role -> #rrggbb` map (see `Colors::to_theme_map`).
        /// Lets app-drawn chrome track the host theme (light/dark + user overrides).
        /// Empty map ⇒ app falls back to its built-in dark constants.
        #[serde(default)]
        theme: std::collections::HashMap<String, String>,
        /// Launch arguments passed via `plexi app open <id> -- <args>` or
        /// programmatic SpawnPane. Also forwarded as subprocess argv for
        /// backward compatibility. Empty when no args were provided.
        #[serde(default)]
        args: Vec<String>,
        /// Pre-seeded app state for headless rendering (--state flag).
        /// When present, the SDK populates self.state before calling on_init.
        /// Omitted in live (non-headless) Init events.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        state: Option<serde_json::Value>,
        /// Initial pane width in logical pixels. Apps use this to size
        /// layout before the first Resize event arrives.
        #[serde(default)]
        width: f32,
        /// Initial pane height in logical pixels.
        #[serde(default)]
        height: f32,
    },
    /// Request a new frame. App replies with DrawCommands terminated by FrameDone.
    Render {
        frame_id: u64,
        /// Current surface rect the app should draw into.
        rect: Rect,
        /// Actual rendered canvas width from the previous frame's component tree.
        /// Zero on the first frame (no prior render). SDK exposes as sdk.canvas_width.
        #[serde(default)]
        canvas_width: f32,
        /// Actual rendered canvas height from the previous frame's component tree.
        /// Zero on the first frame (no prior render). SDK exposes as sdk.canvas_height.
        #[serde(default)]
        canvas_height: f32,
    },
    /// Surface was resized. App should re-layout and request a new frame.
    Resize { width: f32, height: f32 },
    /// User input event.
    Key {
        key: String,
        modifiers: Modifiers,
        pressed: bool,
    },
    /// Mouse click at logical coordinates within the app surface.
    Click {
        x: f32,
        y: f32,
        button: MouseButton,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        region: Option<String>,
    },
    /// Pointer button pressed (fires on the frame the button goes down).
    MouseDown {
        x: f32,
        y: f32,
        button: MouseButton,
        modifiers: Modifiers,
    },
    /// Pointer button released (fires on the frame the button goes up).
    MouseUp {
        x: f32,
        y: f32,
        button: MouseButton,
        modifiers: Modifiers,
    },
    /// Pointer moved over the app surface. Only fires when the app has opted in
    /// via `DrawCommand::SetMouseTracking { enabled: true }`. Pane-local
    /// coordinates; `buttons` lists which buttons are currently held.
    MouseMove {
        x: f32,
        y: f32,
        buttons: Vec<MouseButton>,
        modifiers: Modifiers,
    },
    /// User submitted a command via the command bar.
    Command { text: String },
    /// Semantic action dispatched by `plexi app action <pane_id> <action> [args...]`.
    /// Apps receive this in `on_event` and dispatch on `action` name.
    /// `args` is empty when no extra arguments were provided.
    Action {
        /// Action name, e.g. "refresh", "navigate-to", "add-item".
        action: String,
        /// Optional positional arguments forwarded as-is from the CLI.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        args: Vec<String>,
    },
    /// Response to a runtime CapabilityRequest.
    CapabilityDecision {
        request_id: String,
        capability: String,
        granted: bool,
    },
    /// Secret broker response. value is None when denied.
    SecretValue { key: String, value: Option<String> },
    /// Native WASM app runtime file read result.
    FileReadResult {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        content: Option<Vec<u8>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    /// Native WASM app runtime directory listing result.
    FileListResult {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        entries: Option<Vec<FileListEntry>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    /// Run lifecycle update from the host.
    RunUpdate {
        run_id: String,
        /// One of: "pending" | "running" | "blocked_on_user" | "completed" | "failed"
        status: String,
        payload: serde_json::Value,
    },
    /// Typed pipe message (JSON mode only; binary mode travels on the side channel).
    PipeMessage {
        pipe_id: String,
        payload: serde_json::Value,
    },
    /// Pane group CWD broadcast. Apps in the same group receive this when any
    /// member's CWD changes.
    PathChanged { cwd: PathBuf },
    /// App is being backgrounded (host window losing focus, app no longer visible).
    Suspend,
    /// App is being foregrounded again.
    Resume,
    /// App is being closed. Process must exit within a short timeout.
    Shutdown,
    /// Host theme changed (config hot-reload or macOS system appearance toggle).
    /// App should update its color state; the next render will pick up the new colors.
    Theme {
        /// Updated role → #rrggbb map. Same roles as the Init `theme` field.
        colors: std::collections::HashMap<String, String>,
    },
    /// Confirmation that a SpawnApp request succeeded.
    AppSpawned {
        /// The pane_id of the newly spawned app pane.
        pane_id: u64,
        type_id: String,
    },
    /// Confirmation that a SpawnPane request succeeded (#592).
    PaneSpawned {
        pane_id: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
    },
    /// SpawnPane could not be fulfilled (#592). `reason` is a human-readable error.
    PaneSpawnError {
        reason: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
    },
    /// Response to `AppRequest::QueryContextState` (#1518).
    ContextStateResponse {
        state: crate::host::context_state::ContextState,
    },
    /// Binary pipe opened — app connects to `socket_path` as a unix socket client.
    PipeOpened {
        pipe_id: String,
        socket_path: String,
    },
    /// Binary pipe backpressure — host dropped `dropped_frames` frames from the ring.
    PipeOverrun {
        pipe_id: String,
        dropped_frames: u64,
    },
    /// Drop a JSON payload into the app's `on_inject` hook.
    /// Sent at startup with persisted app state (workspace if available, else global).
    /// Also usable from `pgap_test_harness` to seed deterministic state.
    InjectState { payload: serde_json::Value },
    /// DEPRECATED: superseded by the `state` field on Init.
    /// Kept for backwards compatibility with older SDK versions. The headless
    /// renderer no longer sends this event.
    RenderSeed { payload: serde_json::Value },
    /// Host broker response to a `DrawCommand::HttpRequest`. `error` is present
    /// when the request failed; `body` may still carry a partial response.
    HttpResponse {
        request_id: String,
        status: u16,
        body: String,
        #[serde(default)]
        error: Option<String>,
    },
    /// Sent when the user responds to a notification that included a notify_id.
    ///
    /// - `action_label`: what was clicked — "acknowledge" for the default,
    ///   the option label for a choice, "submit" for input, or "cancel" if
    ///   dismissed with Esc (only possible when `required = false`).
    /// - `value`: option `value` for choice kind, typed text for input kind,
    ///   absent otherwise.
    NotifyAction {
        notify_id: String,
        action_label: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<String>,
    },
    /// Fired when a SetTimer timer expires.
    Timer { timer_id: String },
    /// Fired when a `load_image` request completes (success or failure).
    /// `status` is "ok" or "error". `message` carries the error detail on failure.
    ImageLoaded {
        handle: String,
        status: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    /// Response to a `DrawCommand::MeasureText` request.
    /// `width` and `height` are in logical pixels at the requested font size.
    TextMeasured {
        request_id: String,
        width: f32,
        height: f32,
    },
    /// Response to a `ControlCommand::MeasureTextWrapped` request.
    /// `height` is the pixel height of the text when wrapped at the requested width,
    /// clamped to `max_lines` rows if specified.
    TextWrappedMeasured { request_id: String, height: f32 },
    /// Clipboard paste forwarded into the focused app pane.
    ///
    /// Emitted whenever the host observes `egui::Event::Paste(text)` while an
    /// app pane has keyboard focus. The text is the OS clipboard contents at
    /// paste time, already decoded to UTF-8 by egui. Apps receive this both
    /// for Cmd+V chords and for OS-menu / right-click → Paste actions.
    ///
    /// Pre-#200 apps shelled out to `pbpaste` for this; that workaround is
    /// macOS-only and races with focus changes. This event is the portable
    /// path.
    Paste { text: String },
    /// Response to a `DrawCommand::AiQuery`. Either `content` is `Some` (success)
    /// or `error` is `Some` (failure) — the two are mutually exclusive. Token
    /// counts are zero on error.
    ///
    /// `error` is set when:
    ///   - the app does not declare the `ai.query` capability ("capability denied")
    ///   - the host backend cannot be reached (e.g. missing API key, Ollama not running)
    ///   - the upstream backend returned an error mid-stream
    AiResponse {
        request_id: String,
        content: Option<String>,
        tokens_in: u32,
        tokens_out: u32,
        error: Option<String>,
    },
    /// Incremental token chunk from a streaming ai_query response.
    /// Sent live while the turn runs, before the final `AiResponse`.
    /// The final `AiResponse` is still sent with complete content + token counts.
    AiStreamChunk {
        request_id: String,
        /// Incremental text delta (may be empty for the final chunk before AiResponse)
        delta: String,
        /// Incremental reasoning ("thinking") delta from a reasoning model.
        /// Carried separately from `delta`; a chunk holds one or the other.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reasoning: Option<String>,
        /// True on the last chunk before AiResponse fires
        #[serde(default)]
        done: bool,
    },
    /// Host-to-app tool invocation (#399). The broker calls a tool exposed via
    /// `DrawCommand::ExposeTools` by sending this event to the owning pane.
    /// The app must reply with `DrawCommand::ToolResult { call_id, … }`.
    ToolCall {
        call_id: String,
        name: String,
        input_json: String,
        /// Broker identity of the caller (e.g. `"agent:chess-opponent"` or
        /// an app id). The SDK stamps `caused_by` on events emitted while
        /// this call is being serviced.
        caller_id: String,
    },
    /// External MCP client called a tool declared in `[app.mcp]`. The app must
    /// reply with `DrawCommand::Host(AppRequest::McpToolResult { call_id, … })`.
    McpToolCall {
        call_id: String,
        tool_name: String,
        arguments: serde_json::Value,
    },
    /// Response to a `DrawCommand::ListAudioDevices` request (#277).
    /// Both vectors are always present — empty when enumeration finds no
    /// devices of that direction. `error` is set only when device
    /// enumeration itself failed (host without an audio host driver, etc).
    AudioDevicesListed {
        request_id: String,
        inputs: Vec<AudioDeviceWire>,
        outputs: Vec<AudioDeviceWire>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    /// Sent when a `DrawCommand::AudioCapture` successfully opened the device
    /// and started delivering PCM frames on `pipe_id`. The `sample_rate`,
    /// `channels`, and `buffer_size` are the actual negotiated values — the
    /// app must use these (not its requested values) when interpreting frames.
    AudioCaptureStarted {
        pipe_id: String,
        sample_rate: u32,
        channels: u16,
        buffer_size: u32,
        device_name: String,
    },
    /// Sent when a `DrawCommand::AudioCapture` could not be honoured —
    /// permission denied, bad device id, no devices, cpal failure.
    AudioCaptureError { pipe_id: String, error: String },
    /// Response to a `DrawCommand::ListMidiDevices` request (#320).
    /// Both vectors are always present — empty when CoreMIDI finds no
    /// endpoints of that direction. `error` is set only when MIDI subsystem
    /// itself failed to enumerate (CoreMIDI unavailable, etc).
    MidiDevicesListed {
        request_id: String,
        inputs: Vec<MidiPortWire>,
        outputs: Vec<MidiPortWire>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    /// Sent when a `DrawCommand::OpenMidiInput` successfully opened the port
    /// and started forwarding incoming MIDI byte streams to the binary pipe
    /// at `pipe_id`. The host emitted `PipeOpened { pipe_id, socket_path }`
    /// just before this event so the app can connect to the socket.
    MidiInputOpened {
        pipe_id: String,
        port_id: String,
        port_name: String,
    },
    /// Sent when `DrawCommand::OpenMidiInput` could not be honoured —
    /// permission denied, port not found, CoreMIDI failure.
    MidiInputError { pipe_id: String, error: String },
    /// Sent when `DrawCommand::SendMidi` could not be honoured. Successful
    /// sends produce no event (fire-and-forget); only failures surface.
    MidiSendError { port_id: String, error: String },
    /// Sent when a `DrawCommand::OpenVideo` succeeded (#345). The host has
    /// allocated the binary pipe (look for the preceding `PipeOpened`),
    /// started the decoder, and is now pumping RGBA8 frames into the pipe.
    /// `handle_id` is the opaque handle the app passes to subsequent
    /// `SetVideoState` / `CloseVideo` commands.
    VideoOpenAck {
        request_id: String,
        handle_id: u64,
        width: u32,
        height: u32,
        fps: f32,
        duration_ms: u64,
    },
    /// Sent when `DrawCommand::OpenVideo` could not be honoured (#345) —
    /// capability denied, source not found, decoder error, or the
    /// production stub's `NotImplemented` (until #346 ships).
    VideoOpenError { request_id: String, error: String },
    /// Response to `DrawCommand::RequestLinkedTerminal` (#78). Carries the
    /// pane id of the freshly-opened terminal so subsequent
    /// `RunInLinkedTerminal` / `InsertPathToken` / etc. calls can reference
    /// it. `terminal_pane_id` is the same `PaneId` (`u64`) the host uses
    /// internally — pass it back verbatim.
    ///
    /// Apps without `terminal.bindings` never receive this event; the
    /// request is dropped at the routing layer with a capability-denied
    /// log line.
    LinkedTerminalReady {
        request_id: String,
        terminal_pane_id: u64,
    },
    /// Response to `DrawCommand::RequestCommandPreview` (#78). Returns the
    /// command verbatim plus the linked terminal's current cwd so the app
    /// can render a confirmation modal that says "this would run in
    /// /tmp/foo". `would_run_in_cwd` is the host's best-effort snapshot of
    /// the terminal child's cwd at request time — never an expansion of
    /// shell history or alias.
    CommandPreview {
        request_id: String,
        command: String,
        would_run_in_cwd: String,
    },
    /// Emitted by host to app when Escape is pressed and the app's nav stack
    /// depth is > 0. The app handles this by popping its own internal view
    /// and emitting `DrawCommand::PopNav` to decrement the host counter.
    NavBack { view_id: String },

    /// Response to `DrawCommand::OpenFilePicker`. At least one file was selected.
    /// `paths` contains the absolute paths chosen by the user.
    FilePicked {
        request_id: String,
        paths: Vec<String>,
    },

    /// Response to `DrawCommand::OpenFilePicker` when the user cancelled the
    /// dialog without selecting a file, or the app lacks `fs.pick` capability.
    FilePickCancelled { request_id: String },

    /// Chunk of stdout/stderr bytes from an active `DrawCommand::StreamProcess`
    /// child. `bytes` is a raw byte array (values 0–255); decode with
    /// `bytes(event['bytes'])` in Python. Delivered at up to ~30 Hz.
    StreamChunk {
        correlation_id: String,
        channel: StreamChannel,
        bytes: Vec<u8>,
    },
    /// Terminal event for a `DrawCommand::StreamProcess` child. Sent when the
    /// child exits, on `CancelProcess`, or on capability denial. The SDK
    /// iterator unblocks after this event.
    StreamEnd {
        correlation_id: String,
        exit_code: i32,
    },

    /// Emitted by the host when the scroll offset for a `BeginScroll` region
    /// changes (mouse wheel, drag). The app should re-render using `offset_y`
    /// as the vertical translation applied to all content within that region.
    ///
    /// `id` matches the `id` from the `DrawCommand::BeginScroll` that declared
    /// the region. `offset_y` is always >= 0 and clamped to
    /// `max(0, content_height - viewport_height)`.
    ScrollOffset { id: String, offset_y: f32 },

    /// Emitted when the mouse wheel moves over an app pane that has no
    /// host-managed scroll region or list_view under the cursor.
    /// `delta_y` is the raw `smooth_scroll_delta.y` value from egui (positive =
    /// scroll up). SDK `Scrollable` components call `handle_scroll(delta_y)` from
    /// the app's `on_scroll_delta` handler.
    Scroll { delta_y: f32 },

    /// Emitted when j/k/up/down changes the list selection.
    /// `id` matches the `list_view` id field; `index` is the new selected index.
    ListSelect { id: String, index: usize },

    /// Emitted when Enter is pressed on the selected item.
    /// `id` matches the `list_view` id field; `index` is the activated item index.
    ListActivate { id: String, index: usize },

    /// Fired when a user interacts with a node that has `Interactive` wrapping or
    /// when a Button/Input node is activated.
    ComponentEvent {
        /// Matches the `node_id` on the Interactive/Button/Input node.
        node_id: String,
        /// One of: "click", "hover_enter", "hover_exit", "submit", "change"
        event_type: String,
        /// Optional payload (e.g. current text value for "change" events on Input)
        #[serde(default, skip_serializing_if = "Option::is_none")]
        payload: Option<serde_json::Value>,
    },

    // ── Undo rollback (src/host/app_timeline.rs, Phase B) ─────────────
    /// Host asks the app whether `resource_id` is still at
    /// `expected_revision` before rolling back a checkpoint. The app must
    /// answer with `AppRequest::RollbackVerifyResult` carrying its current
    /// revision; rollback only proceeds on an exact match.
    RollbackVerify {
        checkpoint_id: String,
        resource_id: String,
        expected_revision: String,
    },

    /// Host instructs the app to roll `resource_id` back using the
    /// `rollback_token` the app supplied when it emitted the reversible
    /// event. Sent only after a successful `RollbackVerify` round-trip.
    RollbackApply {
        checkpoint_id: String,
        resource_id: String,
        rollback_token: String,
    },

    /// Response to `AppRequest::SubscribeAppEvents`. Exactly one of
    /// `subscription_id` / `error` is set.
    AppEventsSubscribed {
        request_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subscription_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },

    /// One subscribed app event, delivered to the subscriber pane. Content
    /// beyond the event identity is shaped by the subscription's
    /// `payload_mode`; `trigger_mode` tells the subscriber (Phase C: the
    /// agent runtime) how it is allowed to react.
    AppEvent {
        subscription_id: String,
        /// Publisher app id.
        app_id: String,
        /// Stream name, e.g. `"move.played"`.
        event: String,
        /// Host-assigned timeline id of the underlying event.
        event_id: u64,
        resource_id: String,
        trigger_mode: crate::protocol::commands::TriggerMode,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        summary: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        payload: Option<serde_json::Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        state_ref: Option<String>,
        /// RFC 3339.
        created_at: String,
    },

    /// Response to `AppRequest::ListUndoCheckpoints`: undo checkpoints,
    /// newest first, serialized with the spec's checkpoint metadata fields
    /// plus `status` (`active | verifying | rolled_back | conflict`).
    UndoCheckpoints {
        request_id: String,
        checkpoints: Vec<serde_json::Value>,
    },
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq, Eq)]
pub struct FileListEntry {
    pub name: String,
    pub path: String,
    #[serde(default)]
    pub is_dir: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_event_serializes_pane_dimensions() {
        let event = PlexiEvent::Init {
            protocol: "pgap/3".to_string(),
            app_id: "test".to_string(),
            workspace_root: std::path::PathBuf::from("/tmp"),
            capabilities: vec![],
            feature_flags: vec![],
            compact_threshold: 280.0,
            regular_threshold: 480.0,
            theme: std::collections::HashMap::new(),
            args: vec![],
            state: None,
            width: 800.0,
            height: 450.0,
        };
        let json = serde_json::to_string(&event).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["width"], 800.0);
        assert_eq!(v["height"], 450.0);
    }

    #[test]
    fn init_event_deserializes_without_dimensions() {
        let json = r#"{"type":"init","protocol":"pgap/3","app_id":"test","workspace_root":"/tmp","capabilities":[],"feature_flags":[]}"#;
        let event: PlexiEvent = serde_json::from_str(json).unwrap();
        match event {
            PlexiEvent::Init { width, height, .. } => {
                assert_eq!(width, 0.0);
                assert_eq!(height, 0.0);
            }
            _ => panic!("expected Init"),
        }
    }
}

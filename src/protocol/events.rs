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
    },
    /// Request a new frame. App replies with DrawCommands terminated by FrameDone.
    Render {
        frame_id: u64,
        /// Current surface rect the app should draw into.
        rect: Rect,
    },
    /// Surface was resized. App should re-layout and request a new frame.
    Resize { width: f32, height: f32 },
    /// User input event.
    Key { key: String, modifiers: Modifiers },
    /// Mouse click at logical coordinates within the app surface.
    Click { x: f32, y: f32, button: MouseButton },
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
    CapabilityDecision { request_id: String, granted: bool },
    /// Secret broker response. value is None when denied.
    SecretValue { key: String, value: Option<String> },
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
    /// User pressed Enter inside a `DrawCommand::TextInput` field.
    ///
    /// `id` matches the `id` the app supplied on the `TextInput` command.
    /// `value` is the buffered text at submission time. The host clears its
    /// buffer for `id` after emitting this event so the field is empty for
    /// the next input.
    TextSubmitted { id: String, value: String },
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
}

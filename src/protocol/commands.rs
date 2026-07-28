use super::primitives::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Agent state reported by a hook script.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    Working,
    Blocked,
    Idle,
}

/// App-reported "pip" status — a traffic-light health indicator an app sets for
/// itself via the SDK (`App.set_pip_status`). Optional: when unset the host
/// falls back to derived activity. Distinct from `AgentState` (the hook-script
/// vocabulary) so the SDK surface reads as red/yellow/green.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PipStatus {
    Green,
    Yellow,
    Red,
}

impl PipStatus {
    /// Map the app-reported status onto the host activity-dot vocabulary so the
    /// dot renders the app's intended color. The pip palette (`src/ui/theme.rs`)
    /// is yellow=Working, green=Idle, red=Blocked, so the color-faithful mapping
    /// is yellow→Working, green→Idle, red→Blocked. (Yellow maps to Working, which
    /// pulses — an "attention + active" dot.)
    pub fn as_agent_state(self) -> &'static AgentState {
        match self {
            PipStatus::Green => &AgentState::Idle,
            PipStatus::Yellow => &AgentState::Working,
            PipStatus::Red => &AgentState::Blocked,
        }
    }
}

/// How `plexi context sub` arranges the panes it seeds inside the new
/// sub-context's single window.
#[derive(
    Serialize, Deserialize, JsonSchema, clap::ValueEnum, Debug, Clone, Copy, Default, PartialEq, Eq,
)]
#[serde(rename_all = "snake_case")]
pub enum SubContextLayout {
    /// Near-square grid — the default for an agent squad.
    #[default]
    Tiled,
    /// One full-height column per pane, left to right.
    Columns,
}

/// Per-pane agent state stored on the pane struct.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PaneAgentState {
    pub pane_id: u64,
    pub state: AgentState,
    pub agent: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

/// App-to-host requests — go to `route_command`.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AppRequest {
    /// Request a runtime capability prompt. Host shows modal; responds with CapabilityDecision.
    CapabilityRequest {
        request_id: String,
        /// v3 capability string, e.g. "net.http"
        capability: String,
    },
    /// Request a workspace-scoped secret. Scoped to Init.workspace_root automatically.
    SecretGet { key: String },
    /// Read a file through the native WASM app runtime host. Requires `fs.read` and
    /// the resolved path must stay inside the app's workspace root.
    FileRead { path: String },
    /// List a directory through the native WASM app runtime host. Requires `fs.read`
    /// and the resolved path must stay inside the app's workspace root.
    FileList {
        path: String,
        #[serde(default)]
        extensions: Vec<String>,
    },
    /// Save app state. Host writes to workspace or global JSON file.
    SaveAppState { payload: serde_json::Value },
    /// Request to start a run. Host surfaces in Run palette (Cmd+R).
    RunGet {
        intent: String,
        payload: serde_json::Value,
    },
    /// Signal that a run the app owns has finished.
    RunComplete {
        run_id: String,
        result: serde_json::Value,
    },
    /// Post a notification. All three action_types must dispatch correctly (no TODO).
    Notify {
        /// One of: "info" | "warn" | "error"
        level: String,
        title: String,
        body: String,
        /// The notification shape. Defaults to `message` for back-compat with
        /// existing apps. Determines how the modal renders and interacts.
        #[serde(default)]
        kind: NotifyKind,
        /// Choice options (only meaningful for `kind = "choice"`). The host shows
        /// these as keyboard-navigable buttons; Enter selects the focused one.
        #[serde(default)]
        options: Vec<NotifyOption>,
        /// Placeholder / hint for the text input (only for `kind = "input"`).
        #[serde(default)]
        input_prompt: Option<String>,
        /// If true, the user cannot dismiss with Esc — they must pick an option
        /// or submit input. Intended for decisions the app depends on.
        #[serde(default)]
        required: bool,
        #[serde(default)]
        actions: Vec<NotificationAction>,
        /// If set, host sends PlexiEvent::NotifyAction when the user responds.
        #[serde(default)]
        notify_id: Option<String>,
        /// Higher = more urgent. REQUIRED — no `#[serde(default)]`. Apps must
        /// set this explicitly; omitting it fails deserialisation (forces SDK
        /// upgrade, no silent defaults). Ties broken by arrival order.
        /// Typical values: 0 (background info), 50 (normal), 100 (important),
        /// 200 (critical/required).
        priority: u32,
        /// Inline image attachment (PNG / JPEG, base64-encoded). Rendered above
        /// the action buttons. Decoded size > 50 KB triggers a placeholder.
        /// `Option` is the natural empty/missing wire shape — no
        /// `#[serde(default)]` shim. Mutually exclusive with `image_pipe_id`;
        /// when both set, inline wins and the pipe is ignored (logged warn).
        #[serde(skip_serializing_if = "Option::is_none")]
        image_inline: Option<NotificationImage>,
        /// Pipe-referenced image. Host drains the binary ring lazily when the
        /// notification is visible. Payload format: `width: u32 LE`,
        /// `height: u32 LE`, then `width * height * 4` bytes of RGBA.
        #[serde(skip_serializing_if = "Option::is_none")]
        image_pipe_id: Option<String>,
        /// Auto-dismiss after this many seconds. On expiry, host delivers
        /// `PlexiEvent::NotifyAction` with `value = on_dismiss` (or "timeout" if
        /// `on_dismiss` is None). Omit for notifications that should persist.
        #[serde(default)]
        timeout_secs: Option<u64>,
        /// Payload delivered to the app when the notification is dismissed without
        /// an explicit choice (timeout, Esc, or tombstone dismiss). Defaults to
        /// "timeout" when omitted.
        #[serde(default)]
        on_dismiss: Option<String>,
        /// CLI-only: path to a file the host writes the chosen key into when
        /// the user responds. Used by `plexi notify --choice` to return the
        /// result to the blocking CLI process. Never set by app SDK.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        response_file: Option<String>,
        /// CLI-only: notification visibility scope. Apps never set this — their
        /// scope comes from `manifest.toml::[launch] notification_scope`.
        /// Omit to take `NotifyScope::default()`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        scope: Option<NotifyScope>,
        /// CLI-only: the sender's own context id (`PLEXI_CONTEXT_ID`), stamped
        /// by the CLI so the host never guesses provenance from whichever
        /// context happens to be active at dispatch time. Absent when the
        /// caller runs outside any Plexi pane. A scope the sender cannot
        /// support resolves to the nearest wider one rather than attaching to
        /// a context that never produced it: window scope with a resolvable
        /// context but no live window narrows to context; no resolvable
        /// sender at all escalates to global.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source_context_id: Option<u64>,
        /// CLI-only: the sender's own pane id (`PLEXI_PANE_ID`). When the pane
        /// is still alive its live location is the ground truth for the
        /// notification's window and context — a pane can move between
        /// contexts after its env was stamped.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source_pane_id: Option<u64>,
    },
    /// Report agent state for a pane. Called by hook scripts via `plexi agent report`.
    SetAgentState {
        pane_id: u64,
        state: AgentState,
        agent: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
        #[serde(default)]
        session_id: Option<String>,
    },
    /// Report an app's own pip status (red/yellow/green) for its activity dot.
    /// Fire-and-forget; set by the app process via `App.set_pip_status`. Takes
    /// priority over derived activity until the app reports a new status.
    ///
    /// `pane_id` is stamped by the host with the sending app's own pane (the app
    /// does not know its pane id), so it defaults to 0 on the wire and an app
    /// can only ever set its own pip — never another pane's.
    SetPipStatus {
        #[serde(default)]
        pane_id: u64,
        status: PipStatus,
    },
    /// Get all tracked pane agent states. Writes JSON array to response_file.
    GetAgentStates { response_file: String },
    /// Open a typed pipe.
    /// mode: "json" | "binary"
    /// direction: "in" | "out" | "duplex"
    PipeOpen {
        pipe_id: String,
        /// One of: "json" | "binary"
        mode: String,
        /// One of: "in" | "out" | "duplex"
        direction: String,
    },
    /// Open a *directed* JSON pipe to a specific target pane (#286).
    ///
    /// Mirrors `PipeOpen` but the host scopes `PipeMessage` delivery so only
    /// the caller pane and the pane identified by `target_pane_id` can
    /// receive messages on this pipe — peer apps are NOT subscribed even
    /// if they call `pipe_open` with the same `pipe_id`.
    ///
    /// Created bidirectionally: both panes receive the other's `PipeSend`
    /// payloads as `PlexiEvent::PipeMessage`. Either side can close.
    ///
    /// Capability gate: caller needs `pipe.open` (the same as `PipeOpen`).
    /// The target does NOT need `agents.list` — it just needs to be
    /// addressable. The target also doesn't need to opt in; the host
    /// subscribes it on this pipe id.
    ///
    /// Direction is always `duplex` on directed pipes — there's no use case
    /// for a one-way agent-to-agent channel today, so the field is omitted.
    /// Mode is always `json`; binary directed pipes are out of scope until
    /// a real use case arrives.
    PipeOpenDirected {
        pipe_id: String,
        target_pane_id: u64,
    },
    /// Send a JSON-mode pipe message (not for binary pipes).
    PipeSend {
        pipe_id: String,
        payload: serde_json::Value,
    },
    /// Update the status text shown in the parent pane chrome.
    StatusSummary { text: String },

    /// Write one info-level marker line into the running host's channel log
    /// (`~/.plexi-<channel>/plexi.log`). Sent by `plexi host log` — the
    /// sanctioned way for automated drivers (release gates, scene runners,
    /// CI) to leave start/finish summaries in the installed host's own log
    /// instead of only in their local output. Newlines are flattened so the
    /// marker stays one greppable line.
    /// Host writes `{"ok":true}` to `response_file` when set.
    LogMarker {
        /// Short tool identity prefixed to the line (e.g. "editor_gate").
        source: String,
        message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        response_file: Option<String>,
    },

    /// Request the host to spawn a new app pane. Requires `spawn.app` capability.
    /// `layout`: "split_h" (new pane right), "split_v" (default, new pane below),
    ///           or "overlay" (full pane, no split).
    /// `args`: argv passed to the child process (appended after the binary path).
    /// Host responds with `PlexiEvent::AppSpawned { pane_id }` on success.
    SpawnApp {
        type_id: String,
        #[serde(default)]
        layout: Option<String>,
        #[serde(default)]
        args: Vec<String>,
    },

    /// Unified pane spawn primitive (#592). Supersedes SpawnApp for new apps.
    /// Requires `panes.spawn` capability.
    /// `layout`: one of "split_h", "split_right", "split_v", "split_below", "split_above", "split_left", "overlay",
    ///   "new_window" (terminal only — creates a new spatial grid window to the right
    ///   of the current context row instead of splitting the active pane),
    ///   "tab" (terminal only — adds a new tab alongside the focused pane, wrapping
    ///   both in a Tabs container if needed; use after `pane focus` to target a window).
    ///   "overlay_pane" and "background" are reserved but not yet implemented.
    /// A spawned child that needs to report completion back to its spawner does
    /// so over the event bus: it calls `EmitEvent` with `<app>::completed` and
    /// the spawner reads it via `SubscribeAppEvents`. There is no JSON-pipe
    /// reply channel (the legacy `--pipe=<id>` coupling was removed in 0327).
    /// Host responds: `PlexiEvent::PaneSpawned { pane_id }` on success,
    ///               `PlexiEvent::PaneSpawnError { reason }` on failure.
    SpawnPane {
        type_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        layout: Option<String>,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from_pane_id: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        response_file: Option<String>,
        #[serde(default, skip_serializing_if = "is_false")]
        ephemeral: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cwd: Option<String>,
        #[serde(default, skip_serializing_if = "is_false")]
        no_focus: bool,
        /// When set, the host launches the app directly from this filesystem path
        /// rather than looking it up in the registry by type_id.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        /// When set, use this path as the workspace root for app state scoping
        /// instead of defaulting to the app directory. Sent by `plexi open github:`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workspace_root: Option<String>,
        /// When set, spawn the pane into this child context instead of the
        /// requesting app's own context. The target context must exist and be
        /// a descendant of the requesting app's context (#1518).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target_context: Option<u64>,
        /// Inline pane name — applied immediately after spawn so the pane
        /// starts with a human-readable label instead of the default shell title.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },

    /// Set the title displayed on a terminal pane's tab. Sent by `plexi pane set-title`
    /// over PLEXI_SOCKET.
    SetPaneTitle { pane_id: u64, name: String },

    /// List all open panes. Host writes a JSON array to `response_file`. Sent by `plexi pane list`.
    ListPanes {
        response_file: String,
        /// When Some, only panes in this context are returned.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        context_id: Option<u64>,
    },

    /// List all open contexts. Host writes a JSON array to `response_file`.
    /// Sent by `plexi context list`.
    ListContexts { response_file: String },

    /// Query info for a specific pane by ID. Host writes JSON object to `response_file`.
    /// Sent by `plexi pane info`.
    GetPaneInfo { pane_id: u64, response_file: String },

    /// Query info for the previously focused pane. Host walks `pane_focus_history`
    /// from the end, finds the Nth live entry, and writes the same JSON shape as
    /// `GetPaneInfo` to `response_file`. `steps` defaults to 1 (immediate previous).
    /// Returns `{"error":"..."}` if history has fewer than N valid panes.
    /// Sent by `plexi pane info --previous [N]`.
    GetPreviousPaneInfo {
        response_file: String,
        #[serde(default = "one")]
        steps: u64,
    },

    /// List permission state across apps (stint 0017). Gated on
    /// `permissions.manage` when arriving over PGAP. Host writes a JSON object
    /// to `response_file`:
    ///
    /// ```json
    /// {
    ///   "permissions": [
    ///     {
    ///       "app_id": "...",
    ///       "workspace": "/abs/path",
    ///       "capability": "panes.read",
    ///       "state": "green" | "yellow" | "red",
    ///       "stored": true | false,
    ///       "sensitive": true | false,
    ///       "description": "..."
    ///     }
    ///   ],
    ///   "running": ["app_id", ...]
    /// }
    /// ```
    ///
    /// One row per entry persisted in `permissions.toml` (`stored: true`),
    /// plus one row per live capability of a currently-running app that has
    /// no stored entry (`stored: false` — granted → "green", blocked →
    /// "red"). Live capabilities in the effective yellow state (declared but
    /// neither granted nor blocked) are not retained by the host after
    /// launch and only appear once a decision is stored.
    ListPermissions { response_file: String },

    /// Set the stored permission state for an (app, workspace, capability)
    /// triple (stint 0017). Gated on `permissions.manage` when arriving over
    /// PGAP. `state` is one of "green" | "yellow" | "red". When `workspace`
    /// is omitted, the workspace root of a running app with `app_id` is used;
    /// if no such app is running the request fails. Persists to
    /// `permissions.toml` AND live-updates any running app's permission set,
    /// so a revocation takes effect on the app's next request. Host writes
    /// `{"ok":true}` or `{"error":"..."}` to `response_file`. Unknown
    /// capability or state strings fail closed with an error reply.
    SetPermission {
        app_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workspace: Option<String>,
        capability: String,
        state: String,
        response_file: String,
    },

    /// Write bytes to a named host-managed pane file slot.
    SlotWrite {
        pane_id: u64,
        slot_name: String,
        content: Vec<u8>,
        append: bool,
        replace: bool,
        response_file: String,
    },

    /// Read raw bytes from a named host-managed pane file slot.
    SlotRead {
        pane_id: u64,
        slot_name: String,
        response_file: String,
    },

    /// List named host-managed pane file slots.
    SlotList { pane_id: u64, response_file: String },

    /// Delete a named host-managed pane file slot.
    SlotDelete {
        pane_id: u64,
        slot_name: String,
        response_file: String,
    },

    /// Remove slot files for pane ids that are no longer live in any window.
    WorkspaceCleanSlots {
        dry_run: bool,
        response_file: String,
    },

    /// Move UI focus to a pane by PaneId. Sent by `plexi pane focus`. Fire-and-forget.
    FocusPane { pane_id: u64 },

    /// Close a pane by PaneId. Sent by `plexi pane close`. Fire-and-forget.
    ClosePane { pane_id: u64 },

    /// Write text to a running pane's PTY stdin. Sent by `plexi pane send`.
    /// `\n` in text (literal backslash-n) is interpreted as Enter.
    /// Host writes `{"ok":true}` or `{"error":"..."}` to `response_file` when set.
    SendToPane {
        pane_id: u64,
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        response_file: Option<String>,
    },

    /// Deliver a synthetic key event to any pane. Sent by `plexi pane key`.
    /// For terminal panes, the key is translated to PTY bytes.
    /// For PGAP app panes, a `PlexiEvent::Key` is delivered.
    /// Host writes `{"ok":true}` or `{"error":"..."}` to `response_file` when set.
    KeyPane {
        pane_id: u64,
        /// Key string: single char ("h"), named key ("enter", "escape", "up", "down",
        /// "left", "right", "space", "backspace"), or chord ("ctrl+c", "ctrl+d").
        key: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        response_file: Option<String>,
    },

    /// Deliver a local path or image URL to the pane through the same
    /// production drop handler used by native host drops.
    DropFile {
        pane_id: u64,
        path_or_url: String,
        response_file: String,
    },

    /// Deliver a synthetic pointer click to an app pane, in pane-pixel
    /// coordinates (origin at the pane's top-left). Sent by `plexi pane click`.
    /// The host injects a real `PointerMoved`+`PointerButton` press/release
    /// pair into the live production egui pass, so it exercises the exact
    /// same `canvas_transform` inversion a physical click would — never a
    /// parallel resolver. Terminal panes reject this (no click semantics
    /// defined yet). Host writes `{"ok":true}` or `{"error":"..."}` to
    /// `response_file` when set.
    ClickPane {
        pane_id: u64,
        x: f32,
        y: f32,
        /// "left" (default), "right", or "middle".
        #[serde(default, skip_serializing_if = "Option::is_none")]
        button: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        response_file: Option<String>,
    },

    /// Deliver a synthetic click to an app pane by node id, activating the
    /// Button/TextInput/ListView node the id names — the node-addressed
    /// sibling of `ClickPane` (stint 0414). Sent by
    /// `plexi pane click <pane_id> --node <node_id>`. `node_id` matches
    /// `SemanticPaneNode.id`, the id `plexi pane state` reports for every
    /// node in the pane's tree, so a caller resolves it from `pane state`
    /// output rather than computing pixel geometry. The host validates
    /// `node_id` against the pane's cached semantic tree and fails loudly
    /// (no queued click) when it is absent or not an interactive role;
    /// otherwise it resolves the node's on-screen rect during the next
    /// render pass and delivers the same `PendingPaneClick` honest hit-test
    /// the pixel path uses. Host writes `{"ok":true}` or `{"error":"..."}`
    /// to `response_file` when set.
    ClickPaneNode {
        pane_id: u64,
        node_id: String,
        /// "left" (default), "right", or "middle".
        #[serde(default, skip_serializing_if = "Option::is_none")]
        button: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        response_file: Option<String>,
    },

    /// Deliver a sanctioned pointer drag to an app pane: press at `from`,
    /// `steps` intermediate `PointerMoved` positions, release at `to` —
    /// spread across consecutive frames through the same production input
    /// paths `ClickPane` uses (raw-input merge for builtin egui panes, the
    /// render-pass honest hit-test for canvas/process/WASM panes), never a
    /// parallel resolver. Sent by `plexi pane drag` and
    /// `HostHarness::inject_drag` (stint 0510). Endpoints are addressable by
    /// pane-pixel coordinates (`from`/`to`, origin at the pane's top-left) or
    /// by semantic node id (`from_node`/`to_node`, the ids `plexi pane state`
    /// reports; the drag targets the node rect's center). Exactly one of the
    /// pixel/node forms must be set per endpoint; the host validates node ids
    /// against the pane's semantic tree and fails loudly when one is absent.
    /// Host writes `{"ok":true}` or `{"error":"..."}` to `response_file`
    /// when set.
    DragPane {
        pane_id: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<[f32; 2]>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from_node: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to: Option<[f32; 2]>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to_node: Option<String>,
        /// Intermediate move positions between press and release (default 8,
        /// max 256). Each step is one frame, so scrub/threshold logic in the
        /// target app sees a realistic pointer trajectory.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        steps: Option<u32>,
        /// "left" (default), "right", or "middle".
        #[serde(default, skip_serializing_if = "Option::is_none")]
        button: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        response_file: Option<String>,
    },

    /// Read the last N lines from a terminal pane's PTY scrollback buffer.
    /// Sent by `plexi pane capture`. Host writes a JSON array of strings to `response_file`.
    CapturePane {
        pane_id: u64,
        /// Number of lines to capture from the end of the scrollback.
        lines: usize,
        response_file: String,
        /// When true, preserve trailing empty lines. Defaults to false (strip them).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        full_output: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from_cursor: Option<u64>,
    },

    /// Query the last-rendered UI state of a pane. Sent by `plexi pane state`.
    /// For app panes: host writes a versioned `semantic` tree for every runtime.
    /// Process panes also retain the compatible `frame` RenderCommand array.
    /// For terminal panes: host writes a simple status object.
    /// Host writes `{"error":"..."}` if the pane is not found.
    GetPaneState { pane_id: u64, response_file: String },

    /// Capture the live host window as a PNG through the real render
    /// pipeline (`egui::ViewportCommand::Screenshot` — actual rendered
    /// pixels, not a re-render). With `pane_id`, the image is cropped to
    /// that pane's current screen rect. Sent by `plexi host screenshot`.
    /// Host writes the PNG to `output_path` and
    /// `{"ok":true,"path":...,"width":...,"height":...}` (or
    /// `{"error":"..."}`) to `response_file`.
    Screenshot {
        #[serde(default)]
        pane_id: Option<u64>,
        output_path: String,
        response_file: String,
    },

    /// Dispatch a semantic action to an app pane. Sent by `plexi app action <pane_id> <action> [args...]`.
    /// Host delivers `PlexiEvent::Action { action, args }` to the target app pane.
    /// Writes `{"ok":true}` or `{"error":"..."}` to `response_file`.
    SendAppAction {
        pane_id: u64,
        /// Action name, e.g. "refresh", "navigate-to".
        action: String,
        /// Optional extra arguments forwarded from the CLI.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        args: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        response_file: Option<String>,
    },

    /// Create a new context. Sent by `plexi context new` over PLEXI_SOCKET.
    CreateContext {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        root: Option<std::path::PathBuf>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_name: Option<String>,
        /// Parent context id, sent for bare `--parent` (the caller's
        /// `PLEXI_CONTEXT_ID`). Wins over `parent_name`, which is ambiguous when
        /// two contexts share a name. Absent for an explicit `--parent=<name>`,
        /// where resolving by name is what the caller asked for.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_context_id: Option<u64>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        windows: Vec<String>,
        /// Zoom into the new sub-context after creation. Default false (stay in parent).
        #[serde(default)]
        focus: bool,
        /// Direction the portal tile splits in the parent window: "right" | "down" | "left" | "up".
        #[serde(default, skip_serializing_if = "Option::is_none")]
        portal_direction: Option<String>,
        /// Pane in the parent context to anchor the portal split at. The CLI sends
        /// the caller's pane (--pane or PLEXI_PANE_ID); absent or unknown ids fall
        /// back to the parent's focused pane.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        anchor_pane: Option<u64>,
        /// If set, the host writes a JSON response (context_id, windows) to this path.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        response_file: Option<String>,
    },

    /// Create a sub-context under the caller's context, pre-populated with a
    /// squad of terminal panes in a single window. Sent by `plexi context sub`.
    ///
    /// Unlike `CreateContext { parent_name, windows }` this seeds *exactly*
    /// `panes.len()` terminals — no spare root terminal — and tiles them inside
    /// one window instead of creating sibling pages.
    CreateSubContext {
        /// Name for the new sub-context.
        name: String,
        /// Root path for the new sub-context. The CLI sends the caller's cwd.
        root: std::path::PathBuf,
        /// Parent context id (the caller's `PLEXI_CONTEXT_ID`). Authoritative:
        /// `parent_name` is consulted only when this is absent. An id naming no
        /// live context is an error, not a reason to fall back to the name.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_context_id: Option<u64>,
        /// Parent context name. Used only when `parent_context_id` is absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_name: Option<String>,
        /// One entry per pane to create, in order. `null` launches a plain
        /// shell. Never empty — the CLI expands `--agents`/`--command` into
        /// exactly the requested pane count before sending.
        panes: Vec<Option<String>>,
        /// How the panes are arranged inside the sub-context's single window.
        #[serde(default)]
        layout: SubContextLayout,
        /// Zoom into the new sub-context after creation. Default false.
        #[serde(default)]
        focus: bool,
        /// Pane in the parent context to anchor the portal split at. The CLI
        /// sends the caller's `PLEXI_PANE_ID`; absent or unknown ids fall back
        /// to the parent's focused pane.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        anchor_pane: Option<u64>,
        /// If set, the host writes a JSON response (context_id, windows, panes)
        /// to this path.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        response_file: Option<String>,
    },
    /// Focus existing context by root, or create one. Sent by `plexi context open`.
    FocusContext { root: std::path::PathBuf },
    /// Set/update the root of a context. Sent by `plexi context set-root`.
    /// `context_id` targets the caller's context (PLEXI_CONTEXT_ID); when
    /// absent, the active context is used.
    SetContextRoot {
        root: std::path::PathBuf,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        context_id: Option<u64>,
    },
    /// Set/update the description of a context. Sent by `plexi context describe`.
    /// `context_id` targets the caller's context (PLEXI_CONTEXT_ID); when
    /// absent, the active context is used.
    SetContextDescription {
        description: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        context_id: Option<u64>,
    },
    /// Zoom into a sub-context. Pushes depth stack. Sent by `plexi context zoom`.
    ZoomIntoContext { context_id: u64 },
    /// Zoom out of a sub-context. Pops depth stack. Sent by `plexi context zoom-out`.
    ZoomOutOfContext,
    /// Push a pane into a new sub-context. Sent by `plexi context push`.
    /// `pane_id` targets the caller's pane (PLEXI_PANE_ID); when absent, the
    /// focused pane is pushed.
    PushPaneToSubcontext {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pane_id: Option<u64>,
    },

    /// Query the rolled-up `ContextState` for a context (#1518).
    /// The requesting app must be in an ancestor (or the same) context.
    /// Host responds with `PlexiEvent::ContextStateResponse`.
    QueryContextState { context_id: u64 },

    // ── Media + HTTP primitives ──────────────────────────────────────────
    /// Host-brokered HTTP request. Requires `net.http` capability.
    /// Host replies with `PlexiEvent::HttpResponse { request_id, ... }`.
    HttpRequest {
        request_id: String,
        url: String,
        #[serde(default = "default_http_method")]
        method: String,
        #[serde(default)]
        headers: std::collections::HashMap<String, String>,
        #[serde(default)]
        body: Option<String>,
    },
    /// Open an HTTP(S) URL in the user's default browser.
    ///
    /// Capability: `net.http`. The URL host must pass the same
    /// `allowed_hosts` check used by `HttpRequest`.
    OpenUrl { url: String },
    /// v3.3 brokered AI call. Requires `ai.query` capability.
    ///
    /// The host routes this to the active Plexi AI backend, appends an
    /// `AgentTurn` row to `ai-ledger.jsonl`, and replies with
    /// `PlexiEvent::AiResponse { request_id, content, tokens_in, tokens_out, error }`.
    ///
    /// All fields are required — no `serde(default)`. `tools` may be empty;
    /// non-empty `tools` causes the broker to dispatch through a tool loop
    /// (#399) — the AI can call tools on any pane that exposed them via
    /// `AppRequest::ExposeTools`.
    AiQuery {
        request_id: String,
        model_tier: ModelTier,
        system: String,
        messages: Vec<AiMessage>,
        tools: Vec<AiTool>,
    },
    /// v3.7 tool protocol (#398). App declares its callable tools to the host.
    /// The host registers these in the global tool registry so the broker can
    /// route `PlexiEvent::ToolCall` events to this pane during AI turns that
    /// include tool-use.
    ///
    /// May be sent at any time (including on_init). Replaces any prior
    /// registration for this pane — send the full current set each time.
    ExposeTools { tools: Vec<AiTool> },
    /// v3.7 tool protocol (#399). App returns the result of a `PlexiEvent::ToolCall`
    /// invocation. `call_id` must match the `call_id` from the `ToolCall` event.
    ///
    /// Either `output_json` or `error` must be set — not both.
    ToolResult {
        call_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        output_json: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    /// App returns the result of a `PlexiEvent::McpToolCall` invocation.
    /// `call_id` must match the `call_id` from the `McpToolCall` event.
    /// Either `result` or `error` must be set.
    McpToolResult {
        call_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        result: Option<serde_json::Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    /// Host-owned audio playback via `rodio`.
    AudioPlay {
        #[serde(default)]
        source: Option<String>,
        #[serde(default)]
        pipe_id: Option<String>,
        #[serde(default = "default_volume")]
        volume: f32,
        /// One of: "playing" | "paused" | "stopped".
        state: String,
    },
    /// Host-owned audio capture: mic PCM delivered on a binary pipe.
    /// `device_id` selects which input device (from `ListAudioDevices`); when
    /// `None` the host opens the OS default input. The negotiated parameters
    /// arrive in `PlexiEvent::AudioCaptureStarted`; failures arrive in
    /// `PlexiEvent::AudioCaptureError`.
    AudioCapture {
        pipe_id: String,
        #[serde(default)]
        device_id: Option<String>,
        sample_rate: u32,
        buffer_size: u32,
    },
    /// Request enumeration of audio devices (#277). Host responds with
    /// `PlexiEvent::AudioDevicesListed { request_id, inputs, outputs }`. No
    /// capability gate — enumeration discloses only device names already
    /// visible to any macOS app.
    ListAudioDevices { request_id: String },

    /// Request enumeration of MIDI ports (#320). Host responds with
    /// `PlexiEvent::MidiDevicesListed { request_id, inputs, outputs }`. No
    /// capability gate — enumeration discloses only port names already
    /// visible in Audio MIDI Setup.
    ListMidiDevices { request_id: String },
    /// Open a MIDI input port and forward every incoming message as a binary
    /// pipe frame on `pipe_id`. Each frame is a single MIDI 1.0 byte stream
    /// (1–3 bytes for channel-voice / system real-time). Requires `midi.in`.
    /// Host emits `PlexiEvent::PipeOpened` then `PlexiEvent::MidiInputOpened`
    /// on success, or `PlexiEvent::MidiInputError` on failure (port not found,
    /// capability denied, CoreMIDI error).
    OpenMidiInput { port_id: String, pipe_id: String },

    /// Close the MIDI input previously opened on `port_id`. The host
    /// disconnects from the port and closes the associated binary pipe.
    /// No-op if the port is not currently open. No response event.
    CloseMidiInput { port_id: String },

    /// Send one MIDI 1.0 byte stream to `port_id`. Fire-and-forget — the host
    /// only emits `PlexiEvent::MidiSendError` if the send failed (port not
    /// open, CoreMIDI error). Requires `midi.out`. The host opens the output
    /// port lazily on the first `SendMidi` call and keeps it open until the
    /// app exits.
    SendMidi { port_id: String, bytes: Vec<u8> },

    /// Open a video decoder (#345). The host responds with
    /// `PlexiEvent::VideoOpenAck { request_id, handle_id, width, height,
    /// fps, duration_ms }` on success or `PlexiEvent::VideoOpenError
    /// { request_id, error }` on failure (capability denied, source not
    /// found, decoder error, NotImplemented from the production stub).
    /// Decoded RGBA8 frames flow over the binary pipe at `pipe_id`; one
    /// pipe frame = one video frame, packed `[R,G,B,A,...]` of length
    /// `width * height * 4`.
    ///
    /// Requires `video.playback` capability. All fields required —
    /// no `serde(default)`.
    OpenVideo {
        request_id: String,
        source: String,
        pipe_id: String,
    },
    /// Drive playback state for a previously-opened video handle (#345).
    /// `handle_id` is the value returned in `VideoOpenAck`. State variants:
    /// `play`, `pause`, or `seek` to an absolute position in milliseconds.
    SetVideoState {
        handle_id: u64,
        state: crate::media::video::VideoState,
    },
    /// Close a previously-opened video handle (#345). Tears down the
    /// decoder thread and the associated binary pipe drains. No response
    /// event — fire-and-forget.
    CloseVideo { handle_id: u64 },

    /// Request the host to cd all terminals in the same pane group to `cwd`.
    /// Terminals receive `cd <cwd>\n` written to their PTY.
    CdRequest { cwd: String },

    /// Request a one-shot timer. Requires `timer` capability.
    /// Host fires `PlexiEvent::Timer { timer_id }` after `after_ms` milliseconds.
    SetTimer { timer_id: String, after_ms: u64 },
    /// Cancel a pending timer. No-op if the timer has already fired or doesn't exist.
    CancelTimer { timer_id: String },
    /// Async image fetch brokered through the host. Requires `net.http` capability.
    /// Host fetches `src`, caches under `handle`, and emits `PlexiEvent::ImageLoaded`
    /// when done.
    LoadImage { handle: String, src: String },

    // ── Canvas Terminal Binding Primitives (#78) ─────────────────────────
    //
    // The binding-primitive surface that lets a Canvas app drive a linked
    // terminal pane. All commands require the `terminal.bindings` capability
    // — apps without it get capability-denied responses on each call.
    //
    // The lifecycle: app emits `RequestLinkedTerminal` once at startup; host
    // opens a fresh `Pane::Terminal` next to the app and replies with
    // `PlexiEvent::LinkedTerminalReady { request_id, terminal_pane_id }`.
    // From then on every primitive references the terminal via
    // `terminal_pane_id`.
    /// Ask the host to open a fresh terminal pane next to this Canvas app
    /// and link it. Host responds with `PlexiEvent::LinkedTerminalReady
    /// { request_id, terminal_pane_id }`. `cwd` defaults to the app's
    /// workspace root when `None`. `label` is reserved for future pane-
    /// chrome decoration; currently unused by the host but pinned on the
    /// wire so apps and the host evolve together.
    ///
    /// Capability: `terminal.bindings`. Required fields — no `serde(default)`.
    RequestLinkedTerminal {
        request_id: String,
        cwd: Option<String>,
        label: Option<String>,
    },

    /// Execute `command` in a linked terminal pane.
    ///
    /// `echo: true` — the command is typed into the terminal so the user
    /// sees it (followed by a newline so the shell executes it). This is
    /// the default behaviour for click-driven UIs where the user wants to
    /// know what just ran.
    /// `echo: false` — the command is still written to the PTY, but the
    /// caller is signalling intent that the terminal is being driven
    /// programmatically (the terminal still echoes input by default at
    /// the PTY level — silent execution is best-effort).
    ///
    /// Capability: `terminal.bindings`. All fields required.
    RunInLinkedTerminal {
        terminal_pane_id: u64,
        command: String,
        echo: bool,
    },

    /// Insert `path` into the linked terminal at the cursor position.
    ///
    /// `Replace` mode: write a Ctrl-W (kill-word, shell readline default)
    /// before the path — replaces the partial word the user is typing.
    /// `Append` mode: write the path verbatim — handy for completing
    /// drag-and-drop / file-picker style flows where the user is composing
    /// a command.
    ///
    /// The host wraps the path in single quotes (POSIX) when it contains
    /// shell metacharacters so the shell doesn't expand or split it.
    ///
    /// Capability: `terminal.bindings`. All fields required.
    InsertPathToken {
        terminal_pane_id: u64,
        path: String,
        mode: PathTokenMode,
    },

    /// Ask the host to compute the command that *would* run for a given
    /// command string in the linked terminal. Doesn't execute. Used for
    /// confirmation modals before destructive operations.
    ///
    /// Host responds with `PlexiEvent::CommandPreview { request_id, command,
    /// would_run_in_cwd }`. `would_run_in_cwd` is the terminal's current
    /// working directory at request time — useful for "rm -rf .git in
    /// /tmp/foo" style confirmations.
    ///
    /// Capability: `terminal.bindings`. All fields required.
    RequestCommandPreview {
        request_id: String,
        terminal_pane_id: u64,
        command: String,
    },

    /// Open a workspace artifact (file or directory) via the host.
    ///
    /// Modes:
    ///   - `OpenInPane`        — open the path in a new app pane, e.g. a
    ///                           file browser at a directory or a text
    ///                           editor on a file. Implementation: routes
    ///                           through the file-browser app for
    ///                           directories; falls through to the OS
    ///                           default for files in this PR.
    ///   - `RevealInFinder`    — `open -R <path>` on macOS.
    ///   - `OpenWithDefault`   — `open <path>` on macOS — uses Launch
    ///                           Services to pick the registered app for
    ///                           the file's UTI.
    ///
    /// Capability: `terminal.bindings`. All fields required.
    OpenArtifact {
        path: String,
        mode: ArtifactOpenMode,
    },

    // ── Navigation stack ─────────────────────────────────────────────────
    /// App signals it has pushed a navigation level. The host appends the
    /// entry to its per-pane nav stack. While the stack has entries, the pane
    /// chrome shows a back arrow + the current view's title, and Cmd+[
    /// emits `PlexiEvent::NavBack` to the app instead of cycling tabs.
    ///
    /// `view_id` must be a stable identifier for this view (e.g. `"detail"`);
    /// `title` is shown in the pane chrome while this view is active.
    PushNav { view_id: String, title: String },
    /// App signals it has popped a navigation level. The host removes the top
    /// entry from the per-pane nav stack (saturating — no-op on empty stack).
    PopNav {},

    /// Enable or disable `PlexiEvent::MouseMove` delivery for this pane.
    ///
    /// Off by default to avoid flooding apps that don't need continuous pointer
    /// tracking. Send `{ enabled: true }` after `Ready` to start receiving
    /// `on_mouse_move` callbacks. Send `{ enabled: false }` to stop.
    SetMouseTracking { enabled: bool },

    // ── Process streaming (#358) ──────────────────────────────────────────────
    /// Spawn `command` via `sh -c` and stream its output back to the app.
    ///
    /// The host pipes stdout and stderr, batches available bytes at ~30 Hz,
    /// and delivers each batch as `PlexiEvent::StreamChunk`. When the child
    /// exits (or the app calls `CancelProcess`) the host sends
    /// `PlexiEvent::StreamEnd`. `channel` selects which output stream to
    /// forward; v1 treats `structured` identically to `stdout`.
    ///
    /// `terminal_pane_id` links the stream to an existing linked terminal
    /// for capability gating and future display contexts.
    ///
    /// Capability: `terminal.bindings`. All fields required.
    StreamProcess {
        correlation_id: String,
        terminal_pane_id: u64,
        command: String,
        channel: StreamChannel,
    },

    /// Cancel an in-flight `StreamProcess`. The host sends SIGTERM to the
    /// child, waits up to 1s, then SIGKILL. A `PlexiEvent::StreamEnd` is
    /// always delivered after cancellation.
    ///
    /// Cancelling an already-exited stream is a no-op.
    ///
    /// Capability: `terminal.bindings`. All fields required.
    CancelProcess { correlation_id: String },

    // ── File picker (#514, stint 0508) ───────────────────────────────────────
    /// Show a native file picker dialog. Requires `fs.pick` capability.
    ///
    /// `filter` is a list of file extensions without leading dots
    /// (e.g. `["mp4", "mov"]`). Empty list = accept all files. Only applies
    /// to `open` and `save` modes.
    ///
    /// `multiple` allows selecting more than one file (`open` mode only).
    ///
    /// `mode` selects the dialog kind (`open` / `folder` / `save`); see
    /// `FilePickerMode`. Defaults to `open` when omitted.
    ///
    /// Host responds with `PlexiEvent::FilePicked` (paths) or
    /// `PlexiEvent::FilePickCancelled` (user dismissed / capability denied).
    /// Every picked path is registered as a scoped fs grant for this pane:
    /// subsequent `file_read` / `file_write` calls may name the granted path
    /// (or, for `folder` mode, any path under it) as an absolute path, in
    /// addition to workspace-relative paths. Grants live for the pane's
    /// lifetime and are never persisted.
    OpenFilePicker {
        request_id: String,
        filter: Vec<String>,
        multiple: bool,
        #[serde(default)]
        mode: FilePickerMode,
    },

    // ── App events + undo (src/host/app_timeline.rs, Phase B) ─────────
    /// Declare the named event streams this app may emit on. Event names are
    /// app-defined but MUST be declared (with a JSON-Schema object) before
    /// any `EmitEvent` referencing them is accepted. Re-declaring a stream
    /// replaces its previous declaration.
    DeclareEventStreams { streams: Vec<EventStreamDecl> },

    /// Emit a semantic app event into the host timeline.
    ///
    /// Required: `event` (a declared stream name), `actor`, `summary`,
    /// `resource_id`, `revision_after`. The host validates these and rejects
    /// (with a warn log) any event that is malformed or references an
    /// undeclared stream — rejected events never enter the timeline.
    ///
    /// Supplying `rollback_token` marks the mutation reversible: the host
    /// also creates an undo checkpoint from this event's metadata.
    EmitEvent {
        /// Declared stream name, e.g. `"move.played"`.
        event: String,
        /// Who caused the state change.
        actor: AppEventActor,
        /// Optional stable identity for the actor (e.g. an agent id).
        /// Defaults to the emitting app's id.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        actor_id: Option<String>,
        /// Causal identity: the tool caller whose call produced this event
        /// (e.g. `"agent:chess-opponent"`). The SDK stamps this automatically
        /// for events emitted while servicing a `ToolCall`. The agent runtime
        /// uses it to never trigger an agent from its own actions.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        caused_by: Option<String>,
        /// One-line human-readable description, e.g. `"White played e4"`.
        summary: String,
        /// Document, game, pane, or app-instance id the event is about.
        resource_id: String,
        /// Scope class of `resource_id` (`"document"`, `"game"`, `"pane"`…).
        /// Defaults to `"pane"` (the app instance) when omitted.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resource_scope: Option<String>,
        /// Revision identifier after the change.
        revision_after: String,
        /// Structured payload matching the declared stream schema.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        payload: Option<serde_json::Value>,
        /// Stable reference an authorized subscriber can fetch state from,
        /// e.g. `"chess://game/abc/rev/13"`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        state_ref: Option<String>,
        /// Revision identifier before the change.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        revision_before: Option<String>,
        /// Opaque app token the host returns in `PlexiEvent::RollbackApply`.
        /// Presence makes this event checkpoint-creating (reversible).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rollback_token: Option<String>,
        /// Other resource ids touched by this change.
        #[serde(default)]
        changed_resources: Vec<String>,
        /// App's hint for how subscribers should be triggered. The
        /// subscription's own trigger mode always takes precedence.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        suggested_trigger: Option<TriggerMode>,
    },

    /// App's answer to `PlexiEvent::RollbackVerify`: the current revision of
    /// the queried resource. The host compares it against the checkpoint's
    /// `revision_after` — match → `PlexiEvent::RollbackApply`; mismatch →
    /// rollback blocked, checkpoint marked conflict.
    RollbackVerifyResult {
        checkpoint_id: String,
        current_revision: String,
    },

    /// Subscribe this pane to another app's declared event streams. Gated
    /// through the unified broker (`TargetType::AppEventStream`, one
    /// evaluation per event name) — the host stamps the subscriber identity
    /// from the requesting pane; apps cannot subscribe as someone else.
    /// Phase B wire subscriptions are session-scoped.
    ///
    /// Host responds with `PlexiEvent::AppEventsSubscribed`. Matching events
    /// are then delivered as `PlexiEvent::AppEvent`, shaped per
    /// `payload_mode` and tagged with the subscription's `trigger_mode`.
    SubscribeAppEvents {
        request_id: String,
        /// Publisher app package identity whose streams are subscribed.
        app_id: String,
        /// Declared stream names. Empty = all streams the app declares.
        #[serde(default)]
        event_names: Vec<String>,
        payload_mode: PayloadMode,
        trigger_mode: TriggerMode,
        /// Restrict to one resource id; `None` = any resource.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resource_id: Option<String>,
    },

    /// Remove a subscription previously created by `SubscribeAppEvents`.
    /// Only the subscriber that owns it may remove it.
    UnsubscribeAppEvents {
        request_id: String,
        subscription_id: String,
    },

    /// List undo checkpoints from the host undo timeline, newest first.
    /// `app_id` filters to one app; `None` = this app's own checkpoints.
    /// Host responds with `PlexiEvent::UndoCheckpoints`.
    ListUndoCheckpoints {
        request_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        app_id: Option<String>,
    },

    /// Request rollback of an undo checkpoint. Gated through the unified
    /// broker (`TargetType::UndoCheckpoint`). On allow, the host starts the
    /// revision-verification round-trip with the checkpoint's owning app
    /// (`PlexiEvent::RollbackVerify` → `AppRequest::RollbackVerifyResult` →
    /// `PlexiEvent::RollbackApply`). Denials and conflicts are logged.
    RequestRollback { checkpoint_id: String },

    /// No-op wake. Nudges the (zero-frame-idle) UI thread to run a frame so
    /// queued work — spawn-queue files, pane-IPC channel messages — is
    /// drained promptly. Sent by the CLI after writing a spawn-queue file
    /// when no `PLEXI_SOCKET` is set. Fire-and-forget; the handler does
    /// nothing — the wake effect is the socket listener's repaint request.
    Wake,

    /// Request a clean host shutdown. Sent by `plexi host stop` over a direct
    /// `notify.sock` connection (never over `PLEXI_SOCKET` — `host stop` runs
    /// outside a pane by definition). Fire-and-forget; the handler sets a
    /// close-pending flag consumed at the top of the next frame, which sends
    /// `egui::ViewportCommand::Close`. `host stop` falls back to `SIGTERM` if
    /// this request goes unanswered within its short timeout.
    Shutdown,
}

/// Who caused an app state change (`AppRequest::EmitEvent`).
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AppEventActor {
    User,
    Agent,
    App,
    System,
}

/// How an event subscription triggers its subscriber
/// (src/host/app_timeline.rs). Also the vocabulary for an emitting
/// app's `suggested_trigger` hint.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TriggerMode {
    /// Record the event in the timeline only.
    Never,
    /// Inject into the subscriber's context and trigger a visible turn.
    Conversation,
    /// Run a bounded tool workflow without a visible chat turn.
    Ambient,
    /// Prompt the user before triggering.
    Ask,
}

/// How much of an event a subscription delivers
/// (src/host/app_timeline.rs).
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PayloadMode {
    /// Deliver nothing beyond the event name.
    Off,
    /// Deliver the summary line only.
    Summary,
    /// Deliver the structured payload.
    Full,
    /// Deliver the state ref only — the subscriber fetches state itself.
    StateRef,
}

/// One declared app event stream (`AppRequest::DeclareEventStreams`).
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct EventStreamDecl {
    /// Stream name, e.g. `"move.played"`. Must be non-empty.
    pub name: String,
    /// JSON Schema (object) describing the event payload.
    pub schema: serde_json::Value,
    /// Human-readable description of when this event fires.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Replace-vs-append behaviour for `AppRequest::InsertPathToken`.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PathTokenMode {
    /// Send Ctrl-W (kill-word) before the path so the shell's readline
    /// removes the partial word the user was typing, then write the path.
    Replace,
    /// Write the path verbatim at the cursor position.
    Append,
}

/// Routing target for `AppRequest::OpenArtifact`.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactOpenMode {
    /// Open the path in a new Plexi pane (file browser for directories;
    /// OS default for files in v3.5).
    OpenInPane,
    /// `open -R <path>` — reveal in Finder.
    RevealInFinder,
    /// `open <path>` — Launch Services default app.
    OpenWithDefault,
}

/// Visibility scope for a notification.
///
/// - `Context` — visible only when the source context is the active context.
/// - `Global`  — always visible, regardless of which context is active.
///
/// Host-side enum. Apps do NOT emit this on the wire — scope is a per-app
/// user-facing policy declared in `manifest.toml::[launch] notification_scope`,
/// resolved by the host at dispatch time. Apps never think about it.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum NotifyScope {
    /// Only visible when the source window is the active window.
    /// In the current single-window-per-context model this is equivalent to
    /// `Context`; the distinction matters when multi-window contexts land.
    Window,
    /// Visible whenever the source context is the active context (sidebar
    /// item), regardless of which window page is showing.
    ///
    /// The default for every surface that does not request a scope: a
    /// notification belongs to the context that produced it, and following the
    /// user into unrelated contexts is opt-in. Resolve an unset scope with
    /// `unwrap_or_default()` — never with a locally chosen fallback, or the
    /// surfaces drift apart again. The rule is stated once in `docs/CONFIG.md`.
    #[default]
    Context,
    /// Always visible regardless of which context is active. Deliberate opt-in.
    Global,
}

/// An action attached to a Notify command.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct NotificationAction {
    pub label: String,
    /// One of: "resume_run" | "open_intent" | "run_command"
    pub action_type: String,
    pub payload: serde_json::Value,
}

/// The shape / interaction model of a notification.
///
/// - `Message` — title + body, single Acknowledge button.
/// - `Choice`  — title + body + N options; Enter picks the focused one, ↑↓/j-k
///               cycles, 1-9 direct-selects, optional per-option `shortcut` key.
/// - `Input`   — title + body + a text field; Enter submits.
///
/// Future kinds (image / audio / video / rich) will land here without breaking
/// existing apps — `#[serde(default)]` on the field means missing `kind`
/// deserializes to `Message`.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NotifyKind {
    #[default]
    Message,
    Choice,
    Input,
}

/// Inline image attachment on a Notify command. `mime` is the MIME type
/// (`image/png` or `image/jpeg`), `base64` is the raw image bytes
/// base64-encoded. Decoded size cap (50 KB) is enforced host-side at render
/// time; oversized images render a placeholder badge instead of decoding.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq, Eq)]
pub struct NotificationImage {
    pub mime: String,
    pub base64: String,
}

/// One option in a `kind = "choice"` notification.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct NotifyOption {
    /// Visible label on the button.
    pub label: String,
    /// Value returned to the app in `PlexiEvent::NotifyAction.value`. If empty,
    /// the label is used.
    #[serde(default)]
    pub value: String,
    /// Optional single-char hotkey (e.g. "y", "n"). Case-insensitive.
    /// Reserved keys that conflict with navigation are stripped at ingestion:
    /// `j`, `k`, `h`, `l` (navigation), `1`–`9` (digit-select).
    /// Recommended safe set: letters excluding navigation keys; `y`/`n` for yes/no.
    #[serde(default)]
    pub shortcut: Option<String>,
    /// Optional host-side action to execute synchronously at click time.
    /// Format: `"action_type:action_arg"` (e.g. `"pane_focus:123"`).
    /// If set, the host executes the action before writing the response file.
    /// If not set, behavior is unchanged from before this field existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_action: Option<String>,
}

/// Returns `true` if `key` is reserved by the notification overlay for navigation.
/// Reserved: `j`, `k`, `h`, `l` (navigation) and `1`–`9` (digit-select).
pub fn is_reserved_shortcut(key: &str) -> bool {
    let Some(c) = key.chars().next() else {
        return false;
    };
    if key.chars().count() != 1 {
        return false;
    }
    matches!(c.to_ascii_lowercase(), 'j' | 'k' | 'h' | 'l') || c.is_ascii_digit() && c != '0'
}

fn one() -> u64 {
    1
}

fn is_false(b: &bool) -> bool {
    !*b
}

fn default_http_method() -> String {
    "GET".to_string()
}

fn default_volume() -> f32 {
    1.0
}

#[cfg(test)]
mod tests {
    //! Wire-format round-trip tests for the v3.2 clipboard / paste / selectable
    //! text additions (#200 + #146). These pin the on-the-wire shape — every
    //! field is required and must be present. No `#[serde(default)]` papering
    //! over missing fields.
    use crate::protocol::PlexiEvent;

    use super::*;
    #[test]
    fn paste_event_round_trips_serde() {
        let json = r#"{"type":"paste","text":"hello world"}"#;
        let event: PlexiEvent = serde_json::from_str(json).expect("deserialise");
        match &event {
            PlexiEvent::Paste { text } => assert_eq!(text, "hello world"),
            other => panic!("expected Paste, got {other:?}"),
        }
        let serialised = serde_json::to_string(&event).expect("serialise");
        assert!(
            serialised.contains(r#""type":"paste""#),
            "wire tag missing: {serialised}"
        );
        assert!(
            serialised.contains(r#""text":"hello world""#),
            "text missing: {serialised}"
        );
    }

    #[test]
    fn text_drawcommand_missing_selectable_fails_deserialise() {
        // No `selectable` field — must fail because the field is required
        // (no `#[serde(default)]` on it).
        let json = r##"{"type":"text","x":0.0,"y":0.0,"text":"x","size":14.0,"color":"#fff","max_width":null,"elide":true}"##;
        let result: Result<AppRequest, _> = serde_json::from_str(json);
        assert!(
            result.is_err(),
            "deserialise should fail without `selectable` field"
        );
    }

    // ── v3.3 iq.query wire shape (#284) ──────────────────────────────────
    // Pin the on-the-wire shape for AiQuery / AiResponse. All fields are
    // required — no `serde(default)`.

    #[test]
    fn ai_query_drawcommand_round_trips_serde() {
        let json = r#"{"type":"ai_query","request_id":"req-1","model_tier":"medium","system":"You are helpful.","messages":[{"role":"user","content":"hi"}],"tools":[]}"#;
        let cmd: AppRequest = serde_json::from_str(json).expect("deserialise");
        match &cmd {
            AppRequest::AiQuery {
                request_id,
                model_tier,
                system,
                messages,
                tools,
            } => {
                assert_eq!(request_id, "req-1");
                assert_eq!(*model_tier, ModelTier::Medium);
                assert_eq!(system, "You are helpful.");
                assert_eq!(messages.len(), 1);
                assert_eq!(messages[0].role, "user");
                assert_eq!(messages[0].content, "hi");
                assert!(tools.is_empty());
            }
            other => panic!("expected AiQuery, got {other:?}"),
        }
        let serialised = serde_json::to_string(&cmd).expect("serialise");
        assert!(
            serialised.contains(r#""type":"ai_query""#),
            "wire tag missing: {serialised}"
        );
        assert!(
            serialised.contains(r#""model_tier":"medium""#),
            "model_tier missing: {serialised}"
        );
    }

    #[test]
    fn ai_response_round_trips_serde() {
        let json = r#"{"type":"ai_response","request_id":"req-1","content":"Hello!","tokens_in":12,"tokens_out":4,"error":null}"#;
        let event: PlexiEvent = serde_json::from_str(json).expect("deserialise");
        match &event {
            PlexiEvent::AiResponse {
                request_id,
                content,
                tokens_in,
                tokens_out,
                error,
            } => {
                assert_eq!(request_id, "req-1");
                assert_eq!(content.as_deref(), Some("Hello!"));
                assert_eq!(*tokens_in, 12);
                assert_eq!(*tokens_out, 4);
                assert!(error.is_none());
            }
            other => panic!("expected AiResponse, got {other:?}"),
        }
        let serialised = serde_json::to_string(&event).expect("serialise");
        assert!(
            serialised.contains(r#""type":"ai_response""#),
            "wire tag missing: {serialised}"
        );
    }

    #[test]
    fn ai_response_with_error_serde() {
        let json = r#"{"type":"ai_response","request_id":"req-2","content":null,"tokens_in":0,"tokens_out":0,"error":"capability denied: ai.query not declared in manifest"}"#;
        let event: PlexiEvent = serde_json::from_str(json).expect("deserialise");
        match &event {
            PlexiEvent::AiResponse {
                content,
                error,
                tokens_in,
                tokens_out,
                ..
            } => {
                assert!(content.is_none(), "content must be None on error");
                assert_eq!(
                    error.as_deref(),
                    Some("capability denied: ai.query not declared in manifest")
                );
                assert_eq!(*tokens_in, 0);
                assert_eq!(*tokens_out, 0);
            }
            other => panic!("expected AiResponse, got {other:?}"),
        }
    }

    #[test]
    fn ai_query_missing_required_field_fails_deserialise() {
        // No `tools` field — must fail because the field is required
        // (no `#[serde(default)]` on it).
        let json =
            r#"{"type":"ai_query","request_id":"r","model_tier":"low","system":"","messages":[]}"#;
        let result: Result<AppRequest, _> = serde_json::from_str(json);
        assert!(
            result.is_err(),
            "deserialise should fail without `tools` field"
        );
    }

    // ── v3.4 audio capture wire shape (#277) ─────────────────────────────
    // Pin the on-the-wire shape for ListAudioDevices / AudioDevicesListed /
    // AudioCapture. Required fields fail to deserialise when absent.

    #[test]
    fn list_audio_devices_drawcommand_round_trips_serde() {
        let json = r#"{"type":"list_audio_devices","request_id":"req-9"}"#;
        let cmd: AppRequest = serde_json::from_str(json).expect("deserialise");
        match &cmd {
            AppRequest::ListAudioDevices { request_id } => {
                assert_eq!(request_id, "req-9");
            }
            other => panic!("expected ListAudioDevices, got {other:?}"),
        }
        let serialised = serde_json::to_string(&cmd).expect("serialise");
        assert!(
            serialised.contains(r#""type":"list_audio_devices""#),
            "wire tag missing: {serialised}"
        );
    }

    #[test]
    fn audio_devices_listed_event_round_trips_serde() {
        let json = r#"{"type":"audio_devices_listed","request_id":"req-9","inputs":[{"id":"abc","name":"Built-in Mic","default":true}],"outputs":[{"id":"def","name":"Built-in Speakers","default":true}]}"#;
        let event: PlexiEvent = serde_json::from_str(json).expect("deserialise");
        match &event {
            PlexiEvent::AudioDevicesListed {
                request_id,
                inputs,
                outputs,
                error,
            } => {
                assert_eq!(request_id, "req-9");
                assert_eq!(inputs.len(), 1);
                assert_eq!(inputs[0].id, "abc");
                assert!(inputs[0].default);
                assert_eq!(outputs.len(), 1);
                assert_eq!(outputs[0].name, "Built-in Speakers");
                assert!(error.is_none());
            }
            other => panic!("expected AudioDevicesListed, got {other:?}"),
        }
        let serialised = serde_json::to_string(&event).expect("serialise");
        assert!(
            serialised.contains(r#""type":"audio_devices_listed""#),
            "wire tag missing: {serialised}"
        );
    }

    // ── v3.4 CoreMIDI wire shape (#320) ─────────────────────────────────
    // Pin the on-the-wire shape for ListMidiDevices / MidiDevicesListed /
    // OpenMidiInput / SendMidi. Required fields fail to deserialise when absent.

    #[test]
    fn list_midi_devices_drawcommand_round_trips_serde() {
        let json = r#"{"type":"list_midi_devices","request_id":"req-m1"}"#;
        let cmd: AppRequest = serde_json::from_str(json).expect("deserialise");
        match &cmd {
            AppRequest::ListMidiDevices { request_id } => {
                assert_eq!(request_id, "req-m1");
            }
            other => panic!("expected ListMidiDevices, got {other:?}"),
        }
        let serialised = serde_json::to_string(&cmd).expect("serialise");
        assert!(
            serialised.contains(r#""type":"list_midi_devices""#),
            "wire tag missing: {serialised}"
        );
    }

    #[test]
    fn midi_devices_listed_event_round_trips_serde() {
        let json = r#"{"type":"midi_devices_listed","request_id":"req-m1","inputs":[{"id":"123","name":"Mock Controller","default":true}],"outputs":[{"id":"456","name":"Mock Synth","default":true}]}"#;
        let event: PlexiEvent = serde_json::from_str(json).expect("deserialise");
        match &event {
            PlexiEvent::MidiDevicesListed {
                request_id,
                inputs,
                outputs,
                error,
            } => {
                assert_eq!(request_id, "req-m1");
                assert_eq!(inputs.len(), 1);
                assert_eq!(inputs[0].id, "123");
                assert!(inputs[0].default);
                assert_eq!(outputs.len(), 1);
                assert_eq!(outputs[0].name, "Mock Synth");
                assert!(error.is_none());
            }
            other => panic!("expected MidiDevicesListed, got {other:?}"),
        }
        let serialised = serde_json::to_string(&event).expect("serialise");
        assert!(
            serialised.contains(r#""type":"midi_devices_listed""#),
            "wire tag missing: {serialised}"
        );
    }

    #[test]
    fn send_midi_drawcommand_round_trips() {
        // SendMidi carries a Vec<u8> on the wire. JSON encoding is a JSON
        // array of numbers — the human-readable shape (no base64) keeps the
        // wire debuggable and side-steps SDK plumbing for binary payloads.
        let json = r#"{"type":"send_midi","port_id":"123","bytes":[144,60,100]}"#;
        let cmd: AppRequest = serde_json::from_str(json).expect("deserialise");
        match &cmd {
            AppRequest::SendMidi { port_id, bytes } => {
                assert_eq!(port_id, "123");
                assert_eq!(bytes, &vec![0x90u8, 0x3C, 0x64]);
            }
            other => panic!("expected SendMidi, got {other:?}"),
        }
        let serialised = serde_json::to_string(&cmd).expect("serialise");
        assert!(
            serialised.contains(r#""type":"send_midi""#),
            "wire tag missing: {serialised}"
        );

        // Required-field discipline: dropping `bytes` fails deserialisation.
        let bad = r#"{"type":"send_midi","port_id":"123"}"#;
        assert!(
            serde_json::from_str::<AppRequest>(bad).is_err(),
            "must fail without required `bytes` field"
        );
    }

    #[test]
    fn pipe_open_directed_drawcommand_round_trips_serde() {
        let json =
            r#"{"type":"pipe_open_directed","pipe_id":"coord-to-worker","target_pane_id":42}"#;
        let cmd: AppRequest = serde_json::from_str(json).expect("deserialise");
        match &cmd {
            AppRequest::PipeOpenDirected {
                pipe_id,
                target_pane_id,
            } => {
                assert_eq!(pipe_id, "coord-to-worker");
                assert_eq!(*target_pane_id, 42);
            }
            other => panic!("expected PipeOpenDirected, got {other:?}"),
        }
        let serialised = serde_json::to_string(&cmd).expect("serialise");
        assert!(
            serialised.contains(r#""type":"pipe_open_directed""#),
            "wire tag missing: {serialised}"
        );
        assert!(
            serialised.contains(r#""target_pane_id":42"#),
            "target_pane_id missing: {serialised}"
        );
    }

    #[test]
    fn pipe_open_directed_missing_target_pane_id_fails_deserialise() {
        // No `target_pane_id` — must fail. Required.
        let json = r#"{"type":"pipe_open_directed","pipe_id":"x"}"#;
        let result: Result<AppRequest, _> = serde_json::from_str(json);
        assert!(
            result.is_err(),
            "deserialise should fail without `target_pane_id` field"
        );
    }

    #[test]
    fn audio_capture_drawcommand_required_fields() {
        // `sample_rate` and `buffer_size` are now required (no serde-default).
        // `device_id` is the only optional field.
        let bad = r#"{"type":"audio_capture","pipe_id":"mic","device_id":null}"#;
        assert!(
            serde_json::from_str::<AppRequest>(bad).is_err(),
            "must fail without required sample_rate/buffer_size"
        );
        let good = r#"{"type":"audio_capture","pipe_id":"mic","device_id":null,"sample_rate":48000,"buffer_size":512}"#;
        let cmd: AppRequest = serde_json::from_str(good).expect("deserialise");
        match &cmd {
            AppRequest::AudioCapture {
                pipe_id,
                device_id,
                sample_rate,
                buffer_size,
            } => {
                assert_eq!(pipe_id, "mic");
                assert!(device_id.is_none());
                assert_eq!(*sample_rate, 48_000);
                assert_eq!(*buffer_size, 512);
            }
            other => panic!("expected AudioCapture, got {other:?}"),
        }
    }

    // ── v3.4 video substrate (#345) ────────────────────────────────────────
    // Pin the on-the-wire shape for OpenVideo / SetVideoState / CloseVideo
    // and the matching VideoOpenAck / VideoOpenError events. All fields
    // required — no `serde(default)`.

    #[test]
    fn open_video_drawcommand_round_trips_serde() {
        let json = r#"{"type":"open_video","request_id":"req-1","source":"mock://gradient","pipe_id":"video-stream"}"#;
        let cmd: AppRequest = serde_json::from_str(json).expect("deserialise");
        match &cmd {
            AppRequest::OpenVideo {
                request_id,
                source,
                pipe_id,
            } => {
                assert_eq!(request_id, "req-1");
                assert_eq!(source, "mock://gradient");
                assert_eq!(pipe_id, "video-stream");
            }
            other => panic!("expected OpenVideo, got {other:?}"),
        }
        let serialised = serde_json::to_string(&cmd).expect("serialise");
        assert!(
            serialised.contains(r#""type":"open_video""#),
            "wire tag missing: {serialised}"
        );

        // Required-field discipline — dropping any field fails.
        let bad = r#"{"type":"open_video","source":"mock://gradient","pipe_id":"video-stream"}"#;
        assert!(
            serde_json::from_str::<AppRequest>(bad).is_err(),
            "must fail without required `request_id`"
        );
        let bad = r#"{"type":"open_video","request_id":"r","pipe_id":"p"}"#;
        assert!(
            serde_json::from_str::<AppRequest>(bad).is_err(),
            "must fail without required `source`"
        );
        let bad = r#"{"type":"open_video","request_id":"r","source":"mock://x"}"#;
        assert!(
            serde_json::from_str::<AppRequest>(bad).is_err(),
            "must fail without required `pipe_id`"
        );
    }

    #[test]
    fn set_video_state_drawcommand_round_trips_serde() {
        let play_json = r#"{"type":"set_video_state","handle_id":7,"state":{"kind":"play"}}"#;
        let cmd: AppRequest = serde_json::from_str(play_json).expect("deserialise play");
        match &cmd {
            AppRequest::SetVideoState { handle_id, state } => {
                assert_eq!(*handle_id, 7);
                assert_eq!(*state, crate::media::video::VideoState::Play);
            }
            other => panic!("expected SetVideoState, got {other:?}"),
        }
        let serialised = serde_json::to_string(&cmd).expect("serialise");
        assert!(
            serialised.contains(r#""type":"set_video_state""#),
            "wire tag missing: {serialised}"
        );

        let pause_json = r#"{"type":"set_video_state","handle_id":7,"state":{"kind":"pause"}}"#;
        let cmd: AppRequest = serde_json::from_str(pause_json).expect("deserialise pause");
        if let AppRequest::SetVideoState { state, .. } = &cmd {
            assert_eq!(*state, crate::media::video::VideoState::Pause);
        } else {
            panic!("expected SetVideoState pause, got {cmd:?}");
        }

        let seek_json = r#"{"type":"set_video_state","handle_id":7,"state":{"kind":"seek","position_ms":1500}}"#;
        let cmd: AppRequest = serde_json::from_str(seek_json).expect("deserialise seek");
        if let AppRequest::SetVideoState { state, .. } = &cmd {
            assert_eq!(
                *state,
                crate::media::video::VideoState::Seek { position_ms: 1500 }
            );
        } else {
            panic!("expected SetVideoState seek, got {cmd:?}");
        }
    }

    #[test]
    fn close_video_drawcommand_round_trips_serde() {
        let json = r#"{"type":"close_video","handle_id":42}"#;
        let cmd: AppRequest = serde_json::from_str(json).expect("deserialise");
        match &cmd {
            AppRequest::CloseVideo { handle_id } => assert_eq!(*handle_id, 42),
            other => panic!("expected CloseVideo, got {other:?}"),
        }
        let bad = r#"{"type":"close_video"}"#;
        assert!(
            serde_json::from_str::<AppRequest>(bad).is_err(),
            "must fail without required `handle_id`"
        );
    }

    #[test]
    fn video_open_ack_round_trips_serde() {
        let json = r#"{"type":"video_open_ack","request_id":"req-1","handle_id":3,"width":640,"height":360,"fps":30.0,"duration_ms":12000}"#;
        let event: PlexiEvent = serde_json::from_str(json).expect("deserialise");
        match &event {
            PlexiEvent::VideoOpenAck {
                request_id,
                handle_id,
                width,
                height,
                fps,
                duration_ms,
            } => {
                assert_eq!(request_id, "req-1");
                assert_eq!(*handle_id, 3);
                assert_eq!(*width, 640);
                assert_eq!(*height, 360);
                assert!((*fps - 30.0).abs() < 0.01);
                assert_eq!(*duration_ms, 12_000);
            }
            other => panic!("expected VideoOpenAck, got {other:?}"),
        }
        let serialised = serde_json::to_string(&event).expect("serialise");
        assert!(
            serialised.contains(r#""type":"video_open_ack""#),
            "wire tag missing: {serialised}"
        );

        // Required-field discipline.
        let bad = r#"{"type":"video_open_ack","handle_id":3,"width":1,"height":1,"fps":30.0,"duration_ms":0}"#;
        assert!(
            serde_json::from_str::<PlexiEvent>(bad).is_err(),
            "must fail without required `request_id`"
        );
    }

    // ── v3.5 Canvas Terminal Binding Primitives (#78) ─────────────────────
    // Pin the on-the-wire shape for the new primitives. All fields are
    // required — no `serde(default)`. Enums round-trip as snake_case.

    #[test]
    fn request_linked_terminal_drawcommand_round_trips_serde() {
        let json = r#"{"type":"request_linked_terminal","request_id":"req-1","cwd":"/tmp/foo","label":"bindings demo"}"#;
        let cmd: AppRequest = serde_json::from_str(json).expect("deserialise");
        match &cmd {
            AppRequest::RequestLinkedTerminal {
                request_id,
                cwd,
                label,
            } => {
                assert_eq!(request_id, "req-1");
                assert_eq!(cwd.as_deref(), Some("/tmp/foo"));
                assert_eq!(label.as_deref(), Some("bindings demo"));
            }
            other => panic!("expected RequestLinkedTerminal, got {other:?}"),
        }
        let serialised = serde_json::to_string(&cmd).expect("serialise");
        assert!(
            serialised.contains(r#""type":"request_linked_terminal""#),
            "wire tag missing: {serialised}"
        );

        // Required-field discipline — no `serde(default)` on `request_id`.
        let bad = r#"{"type":"request_linked_terminal","cwd":null,"label":null}"#;
        assert!(
            serde_json::from_str::<AppRequest>(bad).is_err(),
            "must fail without required `request_id`"
        );

        // Optional fields: explicit null deserialises to None.
        let null_json =
            r#"{"type":"request_linked_terminal","request_id":"r2","cwd":null,"label":null}"#;
        let cmd: AppRequest = serde_json::from_str(null_json).expect("deserialise null");
        match &cmd {
            AppRequest::RequestLinkedTerminal { cwd, label, .. } => {
                assert!(cwd.is_none());
                assert!(label.is_none());
            }
            other => panic!("expected RequestLinkedTerminal, got {other:?}"),
        }
    }

    #[test]
    fn linked_terminal_ready_event_round_trips_serde() {
        let json = r#"{"type":"linked_terminal_ready","request_id":"req-1","terminal_pane_id":42}"#;
        let event: PlexiEvent = serde_json::from_str(json).expect("deserialise");
        match &event {
            PlexiEvent::LinkedTerminalReady {
                request_id,
                terminal_pane_id,
            } => {
                assert_eq!(request_id, "req-1");
                assert_eq!(*terminal_pane_id, 42);
            }
            other => panic!("expected LinkedTerminalReady, got {other:?}"),
        }
        let serialised = serde_json::to_string(&event).expect("serialise");
        assert!(
            serialised.contains(r#""type":"linked_terminal_ready""#),
            "wire tag missing: {serialised}"
        );
        let bad = r#"{"type":"linked_terminal_ready","request_id":"r"}"#;
        assert!(
            serde_json::from_str::<PlexiEvent>(bad).is_err(),
            "must fail without required `terminal_pane_id`"
        );
    }

    #[test]
    fn run_in_linked_terminal_round_trips_serde() {
        let json = r#"{"type":"run_in_linked_terminal","terminal_pane_id":42,"command":"ls -la","echo":true}"#;
        let cmd: AppRequest = serde_json::from_str(json).expect("deserialise");
        match &cmd {
            AppRequest::RunInLinkedTerminal {
                terminal_pane_id,
                command,
                echo,
            } => {
                assert_eq!(*terminal_pane_id, 42);
                assert_eq!(command, "ls -la");
                assert!(*echo);
            }
            other => panic!("expected RunInLinkedTerminal, got {other:?}"),
        }
        let serialised = serde_json::to_string(&cmd).expect("serialise");
        assert!(
            serialised.contains(r#""type":"run_in_linked_terminal""#),
            "wire tag missing: {serialised}"
        );

        // Required-field discipline — `echo` has no default.
        let bad = r#"{"type":"run_in_linked_terminal","terminal_pane_id":1,"command":"ls"}"#;
        assert!(
            serde_json::from_str::<AppRequest>(bad).is_err(),
            "must fail without required `echo`"
        );
    }

    #[test]
    fn insert_path_token_mode_enum_serde() {
        let replace_json =
            r#"{"type":"insert_path_token","terminal_pane_id":7,"path":"/tmp/x","mode":"replace"}"#;
        let cmd: AppRequest = serde_json::from_str(replace_json).expect("deserialise replace");
        match &cmd {
            AppRequest::InsertPathToken {
                mode,
                path,
                terminal_pane_id,
            } => {
                assert_eq!(*mode, PathTokenMode::Replace);
                assert_eq!(path, "/tmp/x");
                assert_eq!(*terminal_pane_id, 7);
            }
            other => panic!("expected InsertPathToken, got {other:?}"),
        }

        let append_json =
            r#"{"type":"insert_path_token","terminal_pane_id":7,"path":"/tmp/y","mode":"append"}"#;
        let cmd: AppRequest = serde_json::from_str(append_json).expect("deserialise append");
        if let AppRequest::InsertPathToken { mode, .. } = &cmd {
            assert_eq!(*mode, PathTokenMode::Append);
        } else {
            panic!("expected InsertPathToken, got {cmd:?}");
        }

        // Round-trip serialise → snake_case on the wire.
        let serialised = serde_json::to_string(&cmd).expect("serialise");
        assert!(
            serialised.contains(r#""mode":"append""#),
            "mode must serialise as snake_case: {serialised}"
        );

        // Bad mode rejected loudly.
        let bad =
            r#"{"type":"insert_path_token","terminal_pane_id":1,"path":"/x","mode":"INSERT"}"#;
        assert!(
            serde_json::from_str::<AppRequest>(bad).is_err(),
            "unknown mode must fail to deserialise"
        );
    }

    #[test]
    fn request_command_preview_round_trips_serde() {
        let json = r#"{"type":"request_command_preview","request_id":"req-9","terminal_pane_id":3,"command":"rm -rf .git"}"#;
        let cmd: AppRequest = serde_json::from_str(json).expect("deserialise");
        match &cmd {
            AppRequest::RequestCommandPreview {
                request_id,
                terminal_pane_id,
                command,
            } => {
                assert_eq!(request_id, "req-9");
                assert_eq!(*terminal_pane_id, 3);
                assert_eq!(command, "rm -rf .git");
            }
            other => panic!("expected RequestCommandPreview, got {other:?}"),
        }

        let preview_json = r#"{"type":"command_preview","request_id":"req-9","command":"rm -rf .git","would_run_in_cwd":"/tmp/foo"}"#;
        let event: PlexiEvent = serde_json::from_str(preview_json).expect("deserialise event");
        match &event {
            PlexiEvent::CommandPreview {
                request_id,
                command,
                would_run_in_cwd,
            } => {
                assert_eq!(request_id, "req-9");
                assert_eq!(command, "rm -rf .git");
                assert_eq!(would_run_in_cwd, "/tmp/foo");
            }
            other => panic!("expected CommandPreview, got {other:?}"),
        }
    }

    #[test]
    fn open_artifact_mode_enum_serde() {
        let cases = [
            ("open_in_pane", ArtifactOpenMode::OpenInPane),
            ("reveal_in_finder", ArtifactOpenMode::RevealInFinder),
            ("open_with_default", ArtifactOpenMode::OpenWithDefault),
        ];
        for (wire, expected) in cases {
            let json = format!(r#"{{"type":"open_artifact","path":"/tmp/x","mode":"{wire}"}}"#);
            let cmd: AppRequest = serde_json::from_str(&json).expect("deserialise");
            match &cmd {
                AppRequest::OpenArtifact { path, mode } => {
                    assert_eq!(path, "/tmp/x");
                    assert_eq!(*mode, expected, "wire {wire} → {expected:?}");
                }
                other => panic!("expected OpenArtifact, got {other:?}"),
            }
        }

        // Round-trip serialise → snake_case on the wire.
        let cmd = AppRequest::OpenArtifact {
            path: "/tmp/x".to_string(),
            mode: ArtifactOpenMode::RevealInFinder,
        };
        let serialised = serde_json::to_string(&cmd).expect("serialise");
        assert!(
            serialised.contains(r#""mode":"reveal_in_finder""#),
            "snake_case missing: {serialised}"
        );
    }

    #[test]
    fn video_open_error_round_trips_serde() {
        let json = r#"{"type":"video_open_error","request_id":"req-1","error":"video decoder not implemented"}"#;
        let event: PlexiEvent = serde_json::from_str(json).expect("deserialise");
        match &event {
            PlexiEvent::VideoOpenError { request_id, error } => {
                assert_eq!(request_id, "req-1");
                assert!(error.contains("not implemented"));
            }
            other => panic!("expected VideoOpenError, got {other:?}"),
        }
        let bad = r#"{"type":"video_open_error","error":"x"}"#;
        assert!(
            serde_json::from_str::<PlexiEvent>(bad).is_err(),
            "must fail without required `request_id`"
        );
    }

    // ── v3.5 P2 rich notification panel (#74) ─────────────────────────────
    // Image attachments — inline base64 + pipe reference. Both are optional;
    // missing them deserialises to None via serde's natural Option handling
    // (no `#[serde(default)]` shim). Mutually exclusive at render time.

    #[test]
    fn notify_choice_action_round_trips_serde() {
        // Choice action shape: each NotifyOption carries a label, a value
        // (the structured `payload` from issue #74), and an optional
        // single-char hotkey (`shortcut`, the `key` from issue #74).
        let json = r#"{"type":"notify","level":"info","title":"Pick","body":"choose","kind":"choice","options":[{"label":"A","value":"sidebar","shortcut":"1"},{"label":"B","value":"fullwidth","shortcut":"2"}],"priority":100}"#;
        let cmd: AppRequest = serde_json::from_str(json).expect("deserialise");
        match &cmd {
            AppRequest::Notify {
                kind,
                options,
                priority,
                image_inline,
                image_pipe_id,
                ..
            } => {
                assert_eq!(*kind, NotifyKind::Choice);
                assert_eq!(options.len(), 2);
                assert_eq!(options[0].value, "sidebar");
                assert_eq!(options[0].shortcut.as_deref(), Some("1"));
                assert_eq!(options[1].value, "fullwidth");
                assert_eq!(*priority, 100);
                assert!(
                    image_inline.is_none(),
                    "image_inline should default to None"
                );
                assert!(
                    image_pipe_id.is_none(),
                    "image_pipe_id should default to None"
                );
            }
            other => panic!("expected Notify, got {other:?}"),
        }
        let serialised = serde_json::to_string(&cmd).expect("serialise");
        assert!(
            serialised.contains(r#""kind":"choice""#),
            "kind missing: {serialised}"
        );
        assert!(
            serialised.contains(r#""value":"sidebar""#),
            "payload missing: {serialised}"
        );
    }

    #[test]
    fn notify_with_inline_image_round_trips_serde() {
        // 4-byte base64 payload — well under the 50 KB cap. The host will
        // attempt to decode + render; tiny or invalid images render a
        // placeholder, never crash.
        let json = r#"{"type":"notify","level":"info","title":"Pic","body":"see image","kind":"message","priority":50,"image_inline":{"mime":"image/png","base64":"AAAA"}}"#;
        let cmd: AppRequest = serde_json::from_str(json).expect("deserialise");
        match &cmd {
            AppRequest::Notify {
                image_inline,
                image_pipe_id,
                ..
            } => {
                let img = image_inline.as_ref().expect("inline image present");
                assert_eq!(img.mime, "image/png");
                assert_eq!(img.base64, "AAAA");
                assert!(image_pipe_id.is_none());
            }
            other => panic!("expected Notify, got {other:?}"),
        }
        let serialised = serde_json::to_string(&cmd).expect("serialise");
        assert!(
            serialised.contains(r#""image_inline":{"mime":"image/png","base64":"AAAA"}"#),
            "image_inline missing: {serialised}"
        );
        // Skip-serializing-if-none means image_pipe_id is absent on the wire
        // when None — keeps existing notifications byte-identical.
        assert!(
            !serialised.contains("image_pipe_id"),
            "absent fields should not appear: {serialised}"
        );
    }

    #[test]
    fn notify_with_image_pipe_id_round_trips_serde() {
        let json = r#"{"type":"notify","level":"info","title":"Pic","body":"piped","kind":"message","priority":50,"image_pipe_id":"render-out"}"#;
        let cmd: AppRequest = serde_json::from_str(json).expect("deserialise");
        match &cmd {
            AppRequest::Notify {
                image_pipe_id,
                image_inline,
                ..
            } => {
                assert_eq!(image_pipe_id.as_deref(), Some("render-out"));
                assert!(image_inline.is_none());
            }
            other => panic!("expected Notify, got {other:?}"),
        }
    }

    #[test]
    fn notify_without_image_fields_round_trips_serde() {
        // Existing apps that never set image fields must continue to work.
        // Missing `image_inline` / `image_pipe_id` deserialise as None.
        let json = r#"{"type":"notify","level":"info","title":"Plain","body":"no image","kind":"message","priority":50}"#;
        let cmd: AppRequest = serde_json::from_str(json).expect("deserialise");
        match cmd {
            AppRequest::Notify {
                image_inline,
                image_pipe_id,
                ..
            } => {
                assert!(image_inline.is_none());
                assert!(image_pipe_id.is_none());
            }
            other => panic!("expected Notify, got {other:?}"),
        }
    }

    // ── v3.5 StreamProcess / CancelProcess / StreamChunk / StreamEnd (#358) ──

    #[test]
    fn stream_channel_serializes_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&StreamChannel::Stdout).unwrap(),
            r#""stdout""#
        );
        assert_eq!(
            serde_json::to_string(&StreamChannel::Stderr).unwrap(),
            r#""stderr""#
        );
        assert_eq!(
            serde_json::to_string(&StreamChannel::Structured).unwrap(),
            r#""structured""#
        );
        let parsed: StreamChannel = serde_json::from_str(r#""stdout""#).unwrap();
        assert_eq!(parsed, StreamChannel::Stdout);
    }

    #[test]
    fn stream_process_drawcommand_round_trips_serde() {
        let json = r#"{"type":"stream_process","correlation_id":"cid-1","terminal_pane_id":42,"command":"ls -la","channel":"stdout"}"#;
        let cmd: AppRequest = serde_json::from_str(json).expect("deserialise");
        match &cmd {
            AppRequest::StreamProcess {
                correlation_id,
                terminal_pane_id,
                command,
                channel,
            } => {
                assert_eq!(correlation_id, "cid-1");
                assert_eq!(*terminal_pane_id, 42);
                assert_eq!(command, "ls -la");
                assert_eq!(*channel, StreamChannel::Stdout);
            }
            other => panic!("expected StreamProcess, got {other:?}"),
        }
        let serialised = serde_json::to_string(&cmd).expect("serialise");
        assert!(
            serialised.contains(r#""type":"stream_process""#),
            "wire tag missing: {serialised}"
        );

        let bad =
            r#"{"type":"stream_process","terminal_pane_id":42,"command":"ls","channel":"stdout"}"#;
        assert!(
            serde_json::from_str::<AppRequest>(bad).is_err(),
            "must fail without required `correlation_id`"
        );
    }

    #[test]
    fn cancel_process_drawcommand_round_trips_serde() {
        let json = r#"{"type":"cancel_process","correlation_id":"cid-2"}"#;
        let cmd: AppRequest = serde_json::from_str(json).expect("deserialise");
        match &cmd {
            AppRequest::CancelProcess { correlation_id } => {
                assert_eq!(correlation_id, "cid-2");
            }
            other => panic!("expected CancelProcess, got {other:?}"),
        }
        let serialised = serde_json::to_string(&cmd).expect("serialise");
        assert!(
            serialised.contains(r#""type":"cancel_process""#),
            "wire tag missing: {serialised}"
        );

        let bad = r#"{"type":"cancel_process"}"#;
        assert!(
            serde_json::from_str::<AppRequest>(bad).is_err(),
            "must fail without required `correlation_id`"
        );
    }

    #[test]
    fn stream_chunk_event_round_trips_serde() {
        let json = r#"{"type":"stream_chunk","correlation_id":"cid-1","channel":"stderr","bytes":[72,101,108,108,111]}"#;
        let event: PlexiEvent = serde_json::from_str(json).expect("deserialise");
        match &event {
            PlexiEvent::StreamChunk {
                correlation_id,
                channel,
                bytes,
            } => {
                assert_eq!(correlation_id, "cid-1");
                assert_eq!(*channel, StreamChannel::Stderr);
                assert_eq!(bytes, &[72u8, 101, 108, 108, 111]);
            }
            other => panic!("expected StreamChunk, got {other:?}"),
        }
        let serialised = serde_json::to_string(&event).expect("serialise");
        assert!(
            serialised.contains(r#""type":"stream_chunk""#),
            "wire tag missing: {serialised}"
        );

        let bad = r#"{"type":"stream_chunk","channel":"stdout","bytes":[1]}"#;
        assert!(
            serde_json::from_str::<PlexiEvent>(bad).is_err(),
            "must fail without required `correlation_id`"
        );
    }

    #[test]
    fn stream_end_event_round_trips_serde() {
        let json = r#"{"type":"stream_end","correlation_id":"cid-1","exit_code":0}"#;
        let event: PlexiEvent = serde_json::from_str(json).expect("deserialise");
        match &event {
            PlexiEvent::StreamEnd {
                correlation_id,
                exit_code,
            } => {
                assert_eq!(correlation_id, "cid-1");
                assert_eq!(*exit_code, 0);
            }
            other => panic!("expected StreamEnd, got {other:?}"),
        }
        let serialised = serde_json::to_string(&event).expect("serialise");
        assert!(
            serialised.contains(r#""type":"stream_end""#),
            "wire tag missing: {serialised}"
        );

        let json_nonzero = r#"{"type":"stream_end","correlation_id":"cid-2","exit_code":127}"#;
        let event: PlexiEvent = serde_json::from_str(json_nonzero).expect("deserialise nonzero");
        match &event {
            PlexiEvent::StreamEnd { exit_code, .. } => assert_eq!(*exit_code, 127),
            other => panic!("expected StreamEnd, got {other:?}"),
        }

        let bad = r#"{"type":"stream_end","exit_code":0}"#;
        assert!(
            serde_json::from_str::<PlexiEvent>(bad).is_err(),
            "must fail without required `correlation_id`"
        );
    }

    #[test]
    fn test_notify_timeout_fields() {
        let json = r#"{"type":"notify","level":"info","title":"T","body":"B","priority":50,"timeout_secs":30,"on_dismiss":"user_ignored"}"#;
        let cmd: AppRequest = serde_json::from_str(json).expect("deserialise");
        match cmd {
            AppRequest::Notify {
                timeout_secs,
                on_dismiss,
                ..
            } => {
                assert_eq!(timeout_secs, Some(30));
                assert_eq!(on_dismiss.as_deref(), Some("user_ignored"));
            }
            other => panic!("expected AppRequest::Notify, got {other:?}"),
        }
    }

    #[test]
    fn test_notify_timeout_fields_default() {
        let json = r#"{"type":"notify","level":"info","title":"T","body":"B","priority":50}"#;
        let cmd: AppRequest = serde_json::from_str(json).expect("deserialise");
        match cmd {
            AppRequest::Notify {
                timeout_secs,
                on_dismiss,
                ..
            } => {
                assert!(timeout_secs.is_none());
                assert!(on_dismiss.is_none());
            }
            other => panic!("expected AppRequest::Notify, got {other:?}"),
        }
    }

    #[test]
    fn set_pane_title_deserializes() {
        let json = r#"{"type":"set_pane_title","pane_id":42,"name":"my label"}"#;
        let cmd: AppRequest = serde_json::from_str(json).unwrap();
        assert!(matches!(cmd, AppRequest::SetPaneTitle { pane_id: 42, .. }));
    }

    #[test]
    fn spawn_pane_drawcommand_round_trips_serde() {
        let json = r#"{"type":"spawn_pane","type_id":"snake","layout":"split_v","args":["--foo"]}"#;
        let cmd: AppRequest = serde_json::from_str(json).expect("deserialise");
        match &cmd {
            AppRequest::SpawnPane {
                type_id,
                layout,
                args,
                from_pane_id,
                request_id,
                ..
            } => {
                assert_eq!(type_id, "snake");
                assert_eq!(layout.as_deref(), Some("split_v"));
                assert_eq!(args, &["--foo"]);
                assert!(from_pane_id.is_none());
                assert!(request_id.is_none());
            }
            other => panic!("expected SpawnPane, got {other:?}"),
        }
        let serialised = serde_json::to_string(&cmd).expect("serialise");
        assert!(
            serialised.contains(r#""type":"spawn_pane""#),
            "wire tag missing: {serialised}"
        );

        // defaults: layout is None (absent from wire), args to []
        let minimal = r#"{"type":"spawn_pane","type_id":"snake"}"#;
        let cmd2: AppRequest = serde_json::from_str(minimal).expect("deserialise minimal");
        match &cmd2 {
            AppRequest::SpawnPane {
                layout,
                args,
                from_pane_id,
                request_id,
                ..
            } => {
                assert!(layout.is_none(), "absent layout must deserialise to None");
                assert!(args.is_empty());
                assert!(from_pane_id.is_none());
                assert!(request_id.is_none());
            }
            other => panic!("expected SpawnPane, got {other:?}"),
        }
    }

    #[test]
    fn pane_spawned_event_round_trips_serde() {
        let json = r#"{"type":"pane_spawned","pane_id":99}"#;
        let event: PlexiEvent = serde_json::from_str(json).expect("deserialise");
        match &event {
            PlexiEvent::PaneSpawned { pane_id, .. } => {
                assert_eq!(*pane_id, 99);
            }
            other => panic!("expected PaneSpawned, got {other:?}"),
        }
        // request_id is None when not present
        assert!(matches!(
            event,
            PlexiEvent::PaneSpawned {
                request_id: None,
                ..
            }
        ));
        let serialised = serde_json::to_string(&event).expect("serialise");
        assert!(
            serialised.contains(r#""type":"pane_spawned""#),
            "wire tag missing: {serialised}"
        );

        let bad = r#"{"type":"pane_spawned"}"#;
        assert!(
            serde_json::from_str::<PlexiEvent>(bad).is_err(),
            "must fail without pane_id"
        );
    }

    #[test]
    fn pane_spawn_error_event_round_trips_serde() {
        let json = r#"{"type":"pane_spawn_error","reason":"capability denied"}"#;
        let event: PlexiEvent = serde_json::from_str(json).expect("deserialise");
        match &event {
            PlexiEvent::PaneSpawnError { reason, .. } => assert_eq!(reason, "capability denied"),
            other => panic!("expected PaneSpawnError, got {other:?}"),
        }
        let serialised = serde_json::to_string(&event).expect("serialise");
        assert!(
            serialised.contains(r#""type":"pane_spawn_error""#),
            "wire tag missing: {serialised}"
        );
    }

    #[test]
    fn spawn_pane_with_new_fields_round_trips_serde() {
        let json = r#"{"type":"spawn_pane","type_id":"snake","layout":"split_v","from_pane_id":42,"request_id":"req-1"}"#;
        let cmd: AppRequest = serde_json::from_str(json).expect("deserialise");
        match &cmd {
            AppRequest::SpawnPane {
                from_pane_id,
                request_id,
                ..
            } => {
                assert_eq!(*from_pane_id, Some(42u64));
                assert_eq!(request_id.as_deref(), Some("req-1"));
            }
            other => panic!("expected SpawnPane, got {other:?}"),
        }
        let serialised = serde_json::to_string(&cmd).expect("serialise");
        assert!(
            serialised.contains(r#""from_pane_id":42"#),
            "from_pane_id missing: {serialised}"
        );
        assert!(
            serialised.contains(r#""request_id":"req-1""#),
            "request_id missing: {serialised}"
        );
    }

    #[test]
    fn spawn_pane_no_focus_round_trips_serde() {
        let json = r#"{"type":"spawn_pane","type_id":"terminal","no_focus":true}"#;
        let cmd: AppRequest = serde_json::from_str(json).expect("deserialise");
        match &cmd {
            AppRequest::SpawnPane { no_focus, .. } => {
                assert!(*no_focus, "no_focus should be true");
            }
            other => panic!("expected SpawnPane, got {other:?}"),
        }
        let serialised = serde_json::to_string(&cmd).expect("serialise");
        assert!(
            serialised.contains(r#""no_focus":true"#),
            "no_focus missing: {serialised}"
        );

        // default (false) should be omitted from serialised output
        let json_default = r#"{"type":"spawn_pane","type_id":"terminal"}"#;
        let cmd_default: AppRequest =
            serde_json::from_str(json_default).expect("deserialise default");
        match &cmd_default {
            AppRequest::SpawnPane { no_focus, .. } => {
                assert!(!*no_focus, "no_focus should default to false");
            }
            other => panic!("expected SpawnPane, got {other:?}"),
        }
        let serialised_default = serde_json::to_string(&cmd_default).expect("serialise default");
        assert!(
            !serialised_default.contains("no_focus"),
            "no_focus should be omitted when false: {serialised_default}"
        );
    }

    #[test]
    fn spawn_pane_with_workspace_root_round_trips_serde() {
        let json =
            r#"{"type":"spawn_pane","type_id":"terminal","workspace_root":"/tmp/github-repo"}"#;
        let cmd: AppRequest = serde_json::from_str(json).expect("deserialise");
        match &cmd {
            AppRequest::SpawnPane { workspace_root, .. } => {
                assert_eq!(workspace_root.as_deref(), Some("/tmp/github-repo"));
            }
            other => panic!("expected SpawnPane, got {other:?}"),
        }
        let serialised = serde_json::to_string(&cmd).expect("serialise");
        assert!(
            serialised.contains(r#""workspace_root":"/tmp/github-repo""#),
            "workspace_root missing: {serialised}"
        );

        // None should be omitted from serialised output.
        let json_absent = r#"{"type":"spawn_pane","type_id":"terminal"}"#;
        let cmd_absent: AppRequest = serde_json::from_str(json_absent).expect("deserialise absent");
        match &cmd_absent {
            AppRequest::SpawnPane { workspace_root, .. } => {
                assert!(
                    workspace_root.is_none(),
                    "absent workspace_root must deserialise to None"
                );
            }
            other => panic!("expected SpawnPane, got {other:?}"),
        }
        let serialised_absent = serde_json::to_string(&cmd_absent).expect("serialise absent");
        assert!(
            !serialised_absent.contains("workspace_root"),
            "workspace_root should be omitted when None: {serialised_absent}"
        );
    }

    #[test]
    fn spawn_pane_name_round_trips_serde() {
        let json = r#"{"type":"spawn_pane","type_id":"terminal","name":"dev server"}"#;
        let cmd: AppRequest = serde_json::from_str(json).expect("deserialise");
        match &cmd {
            AppRequest::SpawnPane { name, .. } => {
                assert_eq!(name.as_deref(), Some("dev server"));
            }
            other => panic!("expected SpawnPane, got {other:?}"),
        }
        let serialised = serde_json::to_string(&cmd).expect("serialise");
        assert!(
            serialised.contains(r#""name":"dev server""#),
            "name missing: {serialised}"
        );

        let json_absent = r#"{"type":"spawn_pane","type_id":"terminal"}"#;
        let cmd_absent: AppRequest = serde_json::from_str(json_absent).expect("deserialise absent");
        match &cmd_absent {
            AppRequest::SpawnPane { name, .. } => {
                assert!(name.is_none(), "absent name must deserialise to None");
            }
            other => panic!("expected SpawnPane, got {other:?}"),
        }
        let serialised_absent = serde_json::to_string(&cmd_absent).expect("serialise absent");
        assert!(
            !serialised_absent.contains("name"),
            "name should be omitted when None: {serialised_absent}"
        );
    }

    #[test]
    fn pane_spawned_with_request_id_round_trips_serde() {
        let json = r#"{"type":"pane_spawned","pane_id":99,"request_id":"req-abc"}"#;
        let event: PlexiEvent = serde_json::from_str(json).expect("deserialise");
        match &event {
            PlexiEvent::PaneSpawned {
                pane_id,
                request_id,
            } => {
                assert_eq!(*pane_id, 99);
                assert_eq!(request_id.as_deref(), Some("req-abc"));
            }
            other => panic!("expected PaneSpawned, got {other:?}"),
        }
        let serialised = serde_json::to_string(&event).expect("serialise");
        assert!(
            serialised.contains(r#""request_id":"req-abc""#),
            "request_id missing: {serialised}"
        );
        // Omitting request_id → None
        let no_req: PlexiEvent =
            serde_json::from_str(r#"{"type":"pane_spawned","pane_id":1}"#).unwrap();
        assert!(matches!(
            no_req,
            PlexiEvent::PaneSpawned {
                request_id: None,
                ..
            }
        ));
    }

    #[test]
    fn pane_spawn_error_with_request_id_round_trips_serde() {
        let json = r#"{"type":"pane_spawn_error","reason":"denied","request_id":"req-xyz"}"#;
        let event: PlexiEvent = serde_json::from_str(json).expect("deserialise");
        match &event {
            PlexiEvent::PaneSpawnError { reason, request_id } => {
                assert_eq!(reason, "denied");
                assert_eq!(request_id.as_deref(), Some("req-xyz"));
            }
            other => panic!("expected PaneSpawnError, got {other:?}"),
        }
        let serialised = serde_json::to_string(&event).expect("serialise");
        assert!(
            serialised.contains(r#""request_id":"req-xyz""#),
            "request_id missing: {serialised}"
        );
        // Omitting request_id → None
        let no_req: PlexiEvent =
            serde_json::from_str(r#"{"type":"pane_spawn_error","reason":"x"}"#).unwrap();
        assert!(matches!(
            no_req,
            PlexiEvent::PaneSpawnError {
                request_id: None,
                ..
            }
        ));
    }

    #[test]
    fn is_reserved_shortcut_covers_expected_set() {
        // Navigation keys — reserved
        assert!(is_reserved_shortcut("j"));
        assert!(is_reserved_shortcut("k"));
        assert!(is_reserved_shortcut("h"));
        assert!(is_reserved_shortcut("l"));
        assert!(is_reserved_shortcut("J")); // case-insensitive
                                            // Digit-select keys — reserved
        assert!(is_reserved_shortcut("1"));
        assert!(is_reserved_shortcut("9"));
        // 0 is NOT reserved (1-9 only)
        assert!(!is_reserved_shortcut("0"));
        // Safe keys — not reserved
        assert!(!is_reserved_shortcut("y"));
        assert!(!is_reserved_shortcut("n"));
        assert!(!is_reserved_shortcut("a"));
        // Multi-char is not a valid shortcut — treated as not reserved
        assert!(!is_reserved_shortcut("jk"));
        // Empty is not reserved
        assert!(!is_reserved_shortcut(""));
    }

    #[test]
    fn key_pane_drawcommand_round_trips_serde() {
        let json =
            r#"{"type":"key_pane","pane_id":42,"key":"enter","response_file":"result.json"}"#;
        let cmd: AppRequest = serde_json::from_str(json).expect("deserialise");
        match &cmd {
            AppRequest::KeyPane {
                pane_id,
                key,
                response_file,
            } => {
                assert_eq!(*pane_id, 42);
                assert_eq!(key, "enter");
                assert_eq!(response_file.as_deref(), Some("result.json"));
            }
            other => panic!("expected KeyPane, got {other:?}"),
        }
        let serialised = serde_json::to_string(&cmd).expect("serialise");
        assert!(
            serialised.contains(r#""type":"key_pane""#),
            "wire tag missing: {serialised}"
        );

        // Optional field: response_file absent → None
        let minimal = r#"{"type":"key_pane","pane_id":1,"key":"h"}"#;
        let cmd2: AppRequest = serde_json::from_str(minimal).expect("deserialise minimal");
        match &cmd2 {
            AppRequest::KeyPane { response_file, .. } => {
                assert!(
                    response_file.is_none(),
                    "absent response_file must deserialise to None"
                );
            }
            other => panic!("expected KeyPane, got {other:?}"),
        }

        // Required-field discipline: missing key field must fail
        let bad = r#"{"type":"key_pane","pane_id":1}"#;
        assert!(
            serde_json::from_str::<AppRequest>(bad).is_err(),
            "must fail without required key field"
        );
    }

    #[test]
    fn click_pane_command_round_trips_serde() {
        let json = r#"{"type":"click_pane","pane_id":42,"x":12.5,"y":7.0,"button":"left","response_file":"result.json"}"#;
        let cmd: AppRequest = serde_json::from_str(json).expect("deserialise");
        match &cmd {
            AppRequest::ClickPane {
                pane_id,
                x,
                y,
                button,
                response_file,
            } => {
                assert_eq!(*pane_id, 42);
                assert_eq!(*x, 12.5);
                assert_eq!(*y, 7.0);
                assert_eq!(button.as_deref(), Some("left"));
                assert_eq!(response_file.as_deref(), Some("result.json"));
            }
            other => panic!("expected ClickPane, got {other:?}"),
        }
        let serialised = serde_json::to_string(&cmd).expect("serialise");
        assert!(
            serialised.contains(r#""type":"click_pane""#),
            "wire tag missing: {serialised}"
        );

        // Optional fields: button and response_file absent → None
        let minimal = r#"{"type":"click_pane","pane_id":1,"x":0.0,"y":0.0}"#;
        let cmd2: AppRequest = serde_json::from_str(minimal).expect("deserialise minimal");
        match &cmd2 {
            AppRequest::ClickPane {
                button,
                response_file,
                ..
            } => {
                assert!(button.is_none(), "absent button must deserialise to None");
                assert!(
                    response_file.is_none(),
                    "absent response_file must deserialise to None"
                );
            }
            other => panic!("expected ClickPane, got {other:?}"),
        }

        // Required-field discipline: missing x/y must fail
        let bad = r#"{"type":"click_pane","pane_id":1}"#;
        assert!(
            serde_json::from_str::<AppRequest>(bad).is_err(),
            "must fail without required x/y fields"
        );
    }

    #[test]
    fn click_pane_node_command_round_trips_serde() {
        let json = r#"{"type":"click_pane_node","pane_id":42,"node_id":"5","button":"left","response_file":"result.json"}"#;
        let cmd: AppRequest = serde_json::from_str(json).expect("deserialise");
        match &cmd {
            AppRequest::ClickPaneNode {
                pane_id,
                node_id,
                button,
                response_file,
            } => {
                assert_eq!(*pane_id, 42);
                assert_eq!(node_id, "5");
                assert_eq!(button.as_deref(), Some("left"));
                assert_eq!(response_file.as_deref(), Some("result.json"));
            }
            other => panic!("expected ClickPaneNode, got {other:?}"),
        }
        let serialised = serde_json::to_string(&cmd).expect("serialise");
        assert!(
            serialised.contains(r#""type":"click_pane_node""#),
            "wire tag missing: {serialised}"
        );

        // Optional fields: button and response_file absent → None
        let minimal = r#"{"type":"click_pane_node","pane_id":1,"node_id":"3"}"#;
        let cmd2: AppRequest = serde_json::from_str(minimal).expect("deserialise minimal");
        match &cmd2 {
            AppRequest::ClickPaneNode {
                button,
                response_file,
                ..
            } => {
                assert!(button.is_none(), "absent button must deserialise to None");
                assert!(
                    response_file.is_none(),
                    "absent response_file must deserialise to None"
                );
            }
            other => panic!("expected ClickPaneNode, got {other:?}"),
        }

        // Required-field discipline: missing node_id must fail
        let bad = r#"{"type":"click_pane_node","pane_id":1}"#;
        assert!(
            serde_json::from_str::<AppRequest>(bad).is_err(),
            "must fail without required node_id field"
        );
    }

    #[test]
    fn capture_pane_command_round_trips_serde() {
        let json =
            r#"{"type":"capture_pane","pane_id":7,"lines":20,"response_file":"capture.json"}"#;
        let cmd: AppRequest = serde_json::from_str(json).expect("deserialise");
        match &cmd {
            AppRequest::CapturePane {
                pane_id,
                lines,
                response_file,
                full_output,
                from_cursor,
            } => {
                assert_eq!(*pane_id, 7);
                assert_eq!(*lines, 20);
                assert_eq!(response_file, "capture.json");
                assert!(!full_output, "full_output should default to false");
                assert!(from_cursor.is_none(), "from_cursor should default to None");
            }
            other => panic!("expected CapturePane, got {other:?}"),
        }
        let serialised = serde_json::to_string(&cmd).expect("serialise");
        assert!(
            serialised.contains(r#""type":"capture_pane""#),
            "wire tag missing: {serialised}"
        );
        assert!(
            !serialised.contains("full_output"),
            "full_output=false should be omitted from wire format: {serialised}"
        );

        // full_output=true should round-trip
        let json_full = r#"{"type":"capture_pane","pane_id":7,"lines":20,"response_file":"capture.json","full_output":true}"#;
        let cmd_full: AppRequest =
            serde_json::from_str(json_full).expect("deserialise full_output");
        match &cmd_full {
            AppRequest::CapturePane { full_output, .. } => {
                assert!(*full_output, "full_output should be true when set");
            }
            other => panic!("expected CapturePane, got {other:?}"),
        }

        // from_cursor should round-trip
        let json_cursor = r#"{"type":"capture_pane","pane_id":7,"lines":20,"response_file":"capture.json","from_cursor":42}"#;
        let cmd_cursor: AppRequest =
            serde_json::from_str(json_cursor).expect("deserialise from_cursor");
        match &cmd_cursor {
            AppRequest::CapturePane { from_cursor, .. } => {
                assert_eq!(*from_cursor, Some(42), "from_cursor should be Some(42)");
            }
            other => panic!("expected CapturePane, got {other:?}"),
        }
        // from_cursor absent → None
        let cmd_no_cursor: AppRequest = serde_json::from_str(json).expect("deserialise no cursor");
        match &cmd_no_cursor {
            AppRequest::CapturePane { from_cursor, .. } => {
                assert!(from_cursor.is_none(), "from_cursor should default to None");
            }
            other => panic!("expected CapturePane, got {other:?}"),
        }
        // from_cursor=0 should be omitted from wire format
        let cmd_zero: AppRequest = serde_json::from_str(
            r#"{"type":"capture_pane","pane_id":7,"lines":5,"response_file":"f.json"}"#,
        )
        .unwrap();
        let s = serde_json::to_string(&cmd_zero).unwrap();
        assert!(
            !s.contains("from_cursor"),
            "absent from_cursor must not appear on wire: {s}"
        );

        // Missing required fields must fail
        let bad = r#"{"type":"capture_pane","pane_id":1}"#;
        assert!(
            serde_json::from_str::<AppRequest>(bad).is_err(),
            "must fail without lines and response_file"
        );
    }

    #[test]
    fn set_context_description_round_trips_serde() {
        let json = r#"{"type":"set_context_description","description":"Main project workspace"}"#;
        let cmd: AppRequest = serde_json::from_str(json).expect("deserialise");
        match &cmd {
            AppRequest::SetContextDescription {
                description,
                context_id,
            } => {
                assert_eq!(description, "Main project workspace");
                assert_eq!(*context_id, None, "context_id defaults to None");
            }
            other => panic!("expected SetContextDescription, got {other:?}"),
        }
        let targeted = r#"{"type":"set_context_description","description":"d","context_id":7}"#;
        match serde_json::from_str::<AppRequest>(targeted).expect("deserialise targeted") {
            AppRequest::SetContextDescription { context_id, .. } => {
                assert_eq!(context_id, Some(7));
            }
            other => panic!("expected SetContextDescription, got {other:?}"),
        }
        let serialised = serde_json::to_string(&cmd).expect("serialise");
        assert!(
            serialised.contains(r#""type":"set_context_description""#),
            "wire tag missing: {serialised}"
        );

        let bad = r#"{"type":"set_context_description"}"#;
        assert!(
            serde_json::from_str::<AppRequest>(bad).is_err(),
            "must fail without required description field"
        );
    }

    // ── feed-quality primitives wire shape (#1607) ────────────────────────
    // Pin wire contracts for TextWrappedMeasured, MeasureTextWrapped, Avatar,
    // Skeleton, and Text::max_lines.

    #[test]
    fn text_wrapped_measured_event_round_trips_serde() {
        let json = r#"{"type":"text_wrapped_measured","request_id":"req-wrap-1","height":48.5}"#;
        let event: PlexiEvent = serde_json::from_str(json).expect("deserialise");
        match &event {
            PlexiEvent::TextWrappedMeasured { request_id, height } => {
                assert_eq!(request_id, "req-wrap-1");
                assert!((height - 48.5).abs() < 0.001);
            }
            other => panic!("expected TextWrappedMeasured, got {other:?}"),
        }
        let serialised = serde_json::to_string(&event).expect("serialise");
        assert!(
            serialised.contains(r#""type":"text_wrapped_measured""#),
            "wire tag missing: {serialised}"
        );
        assert!(
            serialised.contains(r#""request_id":"req-wrap-1""#),
            "request_id missing: {serialised}"
        );
        assert!(
            serialised.contains(r#""height":"#),
            "height missing: {serialised}"
        );
    }

    #[test]
    fn text_wrapped_measured_missing_height_fails_deserialise() {
        let json = r#"{"type":"text_wrapped_measured","request_id":"req-wrap-2"}"#;
        let result: Result<PlexiEvent, _> = serde_json::from_str(json);
        assert!(result.is_err(), "must fail without required height field");
    }

    #[test]
    fn measure_text_wrapped_missing_required_field_fails_deserialise() {
        // No `max_width` field — required, must fail.
        let json =
            r#"{"type":"measure_text_wrapped","request_id":"r","text":"hi","font_size":12.0}"#;
        let result: Result<AppRequest, _> = serde_json::from_str(json);
        assert!(
            result.is_err(),
            "must fail without required max_width field"
        );
    }

    #[test]
    fn avatar_missing_required_field_fails_deserialise() {
        // No `radius` — required, must fail.
        let json = r#"{"type":"avatar","src":"h","cx":0.0,"cy":0.0}"#;
        let result: Result<AppRequest, _> = serde_json::from_str(json);
        assert!(result.is_err(), "must fail without required radius field");
    }

    #[test]
    fn list_select_event_round_trips_serde() {
        let json = r#"{"type":"list_select","id":"feed","index":3}"#;
        let event: PlexiEvent = serde_json::from_str(json).expect("deserialise");
        match &event {
            PlexiEvent::ListSelect { id, index } => {
                assert_eq!(id, "feed");
                assert_eq!(*index, 3);
            }
            other => panic!("expected ListSelect, got {other:?}"),
        }
        let serialised = serde_json::to_string(&event).expect("serialise");
        assert!(
            serialised.contains(r#""type":"list_select""#),
            "wire tag missing: {serialised}"
        );
        assert!(
            serialised.contains(r#""index":3"#),
            "index missing: {serialised}"
        );
    }

    #[test]
    fn list_activate_event_round_trips_serde() {
        let json = r#"{"type":"list_activate","id":"issues","index":0}"#;
        let event: PlexiEvent = serde_json::from_str(json).expect("deserialise");
        match &event {
            PlexiEvent::ListActivate { id, index } => {
                assert_eq!(id, "issues");
                assert_eq!(*index, 0);
            }
            other => panic!("expected ListActivate, got {other:?}"),
        }
    }
}

#[cfg(test)]
mod component_event_tests {
    use crate::protocol::events::PlexiEvent;

    #[test]
    fn component_event_serializes_correctly() {
        let event = PlexiEvent::ComponentEvent {
            node_id: "btn1".into(),
            event_type: "click".into(),
            payload: None,
        };
        let json = serde_json::to_string(&event).expect("serialize");
        assert!(
            json.contains(r#""type":"component_event""#),
            "event tag missing: {json}"
        );
        assert!(
            json.contains(r#""node_id":"btn1""#),
            "node_id missing: {json}"
        );
        assert!(
            json.contains(r#""event_type":"click""#),
            "event_type missing: {json}"
        );
        // payload:None should be omitted (skip_serializing_if)
        assert!(
            !json.contains("payload"),
            "payload should be absent: {json}"
        );

        // Some(payload) case — payload key and value must appear.
        let event_with_payload = PlexiEvent::ComponentEvent {
            node_id: "inp1".into(),
            event_type: "change".into(),
            payload: Some(serde_json::json!({"value": "hello"})),
        };
        let json2 = serde_json::to_string(&event_with_payload).unwrap();
        assert!(
            json2.contains(r#""payload""#),
            "payload Some case must include payload key: {json2}"
        );
        assert!(
            json2.contains(r#""value""#),
            "payload Some case must include value: {json2}"
        );
    }
}

#[cfg(test)]
mod ai_stream_chunk_tests {
    use super::*;
    use crate::protocol::events::PlexiEvent;

    /// AiStreamChunk round-trips through serde with all fields present.
    #[test]
    fn ai_stream_chunk_round_trips_serde() {
        let event = PlexiEvent::AiStreamChunk {
            request_id: "req-123".to_string(),
            delta: "Hello, ".to_string(),
            reasoning: None,
            done: false,
        };
        let json = serde_json::to_string(&event).expect("serialize");
        assert!(
            json.contains(r#""type":"ai_stream_chunk""#),
            "type tag missing: {json}"
        );
        assert!(
            json.contains(r#""request_id":"req-123""#),
            "request_id missing: {json}"
        );
        assert!(
            json.contains(r#""delta":"Hello, ""#),
            "delta missing: {json}"
        );

        let round_tripped: PlexiEvent = serde_json::from_str(&json).expect("deserialize");
        match round_tripped {
            PlexiEvent::AiStreamChunk {
                request_id,
                delta,
                reasoning,
                done,
            } => {
                assert_eq!(request_id, "req-123");
                assert_eq!(delta, "Hello, ");
                assert_eq!(reasoning, None);
                assert!(!done);
            }
            other => panic!("expected AiStreamChunk, got {other:?}"),
        }
    }

    /// done=true round-trips correctly.
    #[test]
    fn ai_stream_chunk_done_true_round_trips() {
        let event = PlexiEvent::AiStreamChunk {
            request_id: "req-456".to_string(),
            delta: String::new(),
            reasoning: None,
            done: true,
        };
        let json = serde_json::to_string(&event).expect("serialize");
        let round_tripped: PlexiEvent = serde_json::from_str(&json).expect("deserialize");
        match round_tripped {
            PlexiEvent::AiStreamChunk { done, .. } => assert!(done, "done must be true"),
            other => panic!("expected AiStreamChunk, got {other:?}"),
        }
    }

    /// done defaults to false when absent from JSON (serde(default)).
    #[test]
    fn ai_stream_chunk_done_defaults_to_false() {
        let json = r#"{"type":"ai_stream_chunk","request_id":"r1","delta":"hi"}"#;
        let event: PlexiEvent = serde_json::from_str(json).expect("deserialize");
        match event {
            PlexiEvent::AiStreamChunk {
                done,
                delta,
                reasoning,
                ..
            } => {
                assert!(!done, "done should default to false when absent");
                assert_eq!(delta, "hi");
                assert_eq!(reasoning, None, "reasoning should default to None");
            }
            other => panic!("expected AiStreamChunk, got {other:?}"),
        }
    }

    /// A reasoning-only chunk round-trips and omits the field when None.
    #[test]
    fn ai_stream_chunk_reasoning_round_trips() {
        let event = PlexiEvent::AiStreamChunk {
            request_id: "req-789".to_string(),
            delta: String::new(),
            reasoning: Some("considering options".to_string()),
            done: false,
        };
        let json = serde_json::to_string(&event).expect("serialize");
        assert!(
            json.contains(r#""reasoning":"considering options""#),
            "reasoning missing: {json}"
        );
        let round_tripped: PlexiEvent = serde_json::from_str(&json).expect("deserialize");
        match round_tripped {
            PlexiEvent::AiStreamChunk { reasoning, .. } => {
                assert_eq!(reasoning.as_deref(), Some("considering options"));
            }
            other => panic!("expected AiStreamChunk, got {other:?}"),
        }

        // None is omitted from the wire entirely (skip_serializing_if).
        let text_chunk = PlexiEvent::AiStreamChunk {
            request_id: "r".to_string(),
            delta: "hi".to_string(),
            reasoning: None,
            done: false,
        };
        let json = serde_json::to_string(&text_chunk).expect("serialize");
        assert!(!json.contains("reasoning"), "None must be omitted: {json}");
    }

    /// Wire-format round-trip for GetPreviousPaneInfo.
    #[test]
    fn get_previous_pane_info_round_trips_serde() {
        // Without steps — default should be 1.
        let json = r#"{"type":"get_previous_pane_info","response_file":"/tmp/prev.json"}"#;
        let req: AppRequest = serde_json::from_str(json).expect("deserialise");
        match &req {
            AppRequest::GetPreviousPaneInfo {
                response_file,
                steps,
            } => {
                assert_eq!(response_file, "/tmp/prev.json");
                assert_eq!(*steps, 1, "default steps must be 1");
            }
            other => panic!("expected GetPreviousPaneInfo, got {other:?}"),
        }
        let serialised = serde_json::to_string(&req).expect("serialise");
        assert!(
            serialised.contains(r#""type":"get_previous_pane_info""#),
            "wire tag missing: {serialised}"
        );
        assert!(
            serialised.contains(r#""response_file":"/tmp/prev.json""#),
            "response_file missing: {serialised}"
        );

        // With explicit steps=3.
        let json3 =
            r#"{"type":"get_previous_pane_info","response_file":"/tmp/prev.json","steps":3}"#;
        let req3: AppRequest = serde_json::from_str(json3).expect("deserialise steps=3");
        match &req3 {
            AppRequest::GetPreviousPaneInfo { steps, .. } => {
                assert_eq!(*steps, 3);
            }
            other => panic!("expected GetPreviousPaneInfo with steps=3, got {other:?}"),
        }
    }

    /// Wire-format round-trip for the no-op Wake nudge sent by the CLI
    /// after a spawn-queue write.
    #[test]
    fn wake_round_trips_serde() {
        let req: AppRequest = serde_json::from_str(r#"{"type":"wake"}"#).expect("deserialise");
        assert!(
            matches!(req, AppRequest::Wake),
            "expected Wake, got {req:?}"
        );
        let serialised = serde_json::to_string(&req).expect("serialise");
        assert!(
            serialised.contains(r#""type":"wake""#),
            "wire tag missing: {serialised}"
        );
    }

    /// Missing response_file must fail deserialization.
    #[test]
    fn get_previous_pane_info_missing_response_file_fails() {
        let json = r#"{"type":"get_previous_pane_info"}"#;
        assert!(
            serde_json::from_str::<AppRequest>(json).is_err(),
            "missing required response_file must fail"
        );
    }

    #[test]
    fn set_agent_state_detail_round_trips_serde() {
        let json = r#"{"type":"set_agent_state","pane_id":7,"state":"working","agent":"claude-code","detail":"Bash: cargo test","session_id":"abc"}"#;
        let req: AppRequest = serde_json::from_str(json).expect("deserialise");
        match &req {
            AppRequest::SetAgentState {
                pane_id,
                state,
                agent,
                detail,
                session_id,
            } => {
                assert_eq!(*pane_id, 7);
                assert_eq!(*state, AgentState::Working);
                assert_eq!(agent, "claude-code");
                assert_eq!(detail.as_deref(), Some("Bash: cargo test"));
                assert_eq!(session_id.as_deref(), Some("abc"));
            }
            other => panic!("expected SetAgentState, got {other:?}"),
        }

        let serialised = serde_json::to_string(&req).expect("serialise");
        assert!(
            serialised.contains(r#""detail":"Bash: cargo test""#),
            "detail must be emitted when present: {serialised}"
        );
    }

    #[test]
    fn set_agent_state_missing_detail_defaults_to_none() {
        let json = r#"{"type":"set_agent_state","pane_id":7,"state":"idle","agent":"claude-code"}"#;
        let req: AppRequest = serde_json::from_str(json).expect("deserialise");
        match &req {
            AppRequest::SetAgentState {
                detail, session_id, ..
            } => {
                assert!(detail.is_none());
                assert!(session_id.is_none());
            }
            other => panic!("expected SetAgentState, got {other:?}"),
        }

        let serialised = serde_json::to_string(&req).expect("serialise");
        assert!(
            !serialised.contains("detail"),
            "empty detail should not add wire noise: {serialised}"
        );
    }
}

#[cfg(test)]
mod pip_status_tests {
    use super::*;

    #[test]
    fn pip_status_maps_color_faithfully_to_agent_state() {
        // The pip palette (src/ui/theme.rs) is yellow=Working, green=Idle,
        // red=Blocked, so the dot renders the app's intended color.
        assert_eq!(PipStatus::Green.as_agent_state(), &AgentState::Idle);
        assert_eq!(PipStatus::Yellow.as_agent_state(), &AgentState::Working);
        assert_eq!(PipStatus::Red.as_agent_state(), &AgentState::Blocked);
    }

    #[test]
    fn pip_status_wire_format_is_snake_case_and_pane_id_defaults() {
        // The SDK omits pane_id (host stamps it); it must default to 0.
        let req: AppRequest =
            serde_json::from_str(r#"{"type":"set_pip_status","status":"red"}"#).unwrap();
        match req {
            AppRequest::SetPipStatus { pane_id, status } => {
                assert_eq!(
                    pane_id, 0,
                    "omitted pane_id must default to 0 for host stamping"
                );
                assert_eq!(status, PipStatus::Red);
            }
            other => panic!("expected SetPipStatus, got {other:?}"),
        }
    }
}

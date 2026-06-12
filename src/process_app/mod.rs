//! ProcessApp — runs an external app binary as a subprocess and renders it
//! using the Plexi draw protocol.
//!
//! The subprocess speaks the app protocol over stdin/stdout (newline-delimited JSON).
//! ProcessApp implements the `App` trait so it drops in wherever a built-in app
//! would — the rest of Plexi doesn't know or care that it's an external process.
//!
//! Internal layout:
//! - `mod.rs`          — struct, launch/drop, App trait impl
//! - `transport.rs`    — stdin/stdout/stderr/reaper threads
//! - `runtime_state.rs` — PGAP render transaction state machine
//! - `scheduler.rs`    — host repaint policy
//! - `routing.rs`      — `route_command()`: dispatch DrawCommands to subsystems
//! - `render.rs`       — `render_draw_commands()`: paint committed frames into egui
//! - `prompts.rs`      — `show_prompt_modal()`: capability/secret grant UI

mod host_bridge;
pub(crate) mod image_cache;
mod lifecycle;
pub(crate) mod mcp_server;
pub(crate) mod prompts;
pub(crate) mod render;
mod render_diag;
mod render_session;
mod routing;
mod runtime_state;
mod scheduler;
pub(crate) mod transport;

pub(crate) use lifecycle::{LifecycleState, LifecycleTracker};
use render_diag::RenderDiagnostics;
use render_session::RenderSession;
use runtime_state::{FrameDoneOutcome, PgapRuntime, RenderPoll};
#[cfg(test)]
pub(crate) use scheduler::RENDER_IN_FLIGHT_TIMEOUT;
pub(crate) use transport::StdinItem;

fn channel_from_config_dir(config_dir: &Path) -> Option<String> {
    config_dir
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_prefix(".plexi-"))
        .filter(|channel| !channel.is_empty())
        .map(str::to_string)
}

use crate::app::app_trait::{App, AppCommand, AppRenderContext};
use crate::app::permissions::{AppPermissions, Capability};
use crate::app_protocol::{
    AiMessage, AiTool, AppRequest, ControlCommand, DrawCommand, ModelTier, Modifiers, PlexiEvent,
    RenderCommand,
};
use crate::host::event_log::{self, HostEvent};
use crate::host::runs::RunRegistry;
use crate::host::services::{NetService, UreqNetService};
use crate::host::typed_pipes::TypedPipeRegistry;
use crate::media::audio::{AudioDevice, CaptureSession};
use crate::media::midi::{MidiDevice, MidiInputSession, MidiOutputHandle};
use crate::media::video::{VideoDecoder, VideoHandle};
use crate::plexi_ai::broker::{AiBroker, LiveAiBroker};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc::{self, Receiver, Sender, TryRecvError},
    Arc, Mutex,
};

// ---------------------------------------------------------------------------
// PendingPrompt — capability / secret prompts queued for modal presentation
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum PendingPrompt {
    Capability {
        request_id: String,
        capability: String,
    },
    Secret {
        key: String,
    },
}

// ---------------------------------------------------------------------------
// DeferredAiQuery — an AiQuery held pending first-run consent
// ---------------------------------------------------------------------------

/// A `AiQuery` request deferred because `ai.query` was withheld pending consent.
/// Drained and re-dispatched (or errored) when the user resolves the modal.
#[derive(Debug)]
pub(crate) struct DeferredAiQuery {
    pub(crate) request_id: String,
    pub(crate) model_tier: ModelTier,
    pub(crate) system: String,
    pub(crate) messages: Vec<AiMessage>,
    pub(crate) tools: Vec<AiTool>,
}

// ---------------------------------------------------------------------------
// NavEntry
// ---------------------------------------------------------------------------

/// One entry on a pane's navigation stack, pushed by `DrawCommand::PushNav`.
#[derive(Debug, Clone)]
pub struct NavEntry {
    /// Stable identifier for this view (e.g. `"detail"`). Echoed back to the
    /// app in `PlexiEvent::NavBack { view_id }` when this becomes the active view.
    pub view_id: String,
    /// Human-readable title shown in the pane chrome while this view is active.
    pub title: String,
}

// ---------------------------------------------------------------------------
// StreamHandle — per-correlation-id state for active StreamProcess children
// ---------------------------------------------------------------------------

pub(crate) struct StreamHandle {
    /// Set to `true` to request cancellation. Shared with the reader thread.
    pub(crate) cancel: Arc<AtomicBool>,
    /// OS process ID — used for SIGTERM / SIGKILL escalation on CancelProcess.
    pub(crate) pid: u32,
}

// ---------------------------------------------------------------------------
// ProcessApp
// ---------------------------------------------------------------------------

pub struct ProcessApp {
    pub(crate) type_id: String,
    pub pane_id: u64,
    display_name: String,
    process: Option<Child>,
    /// Channel to the stdin-writer background thread. The writer owns the
    /// actual `ChildStdin` and blocks on writes there — the GUI thread never
    /// touches the pipe directly. Render events are coalesced via
    /// `render_slot` / `render_in_queue` so that a burst of Render events
    /// during startup never fills the channel and starves itself.
    event_tx: Option<Sender<StdinItem>>,
    /// Holds the latest serialised `PlexiEvent::Render` payload. The
    /// stdin-writer thread drains this on `StdinItem::FlushRender`.
    pub(crate) render_slot: Arc<Mutex<Option<String>>>,
    /// True while a `StdinItem::FlushRender` token is already in the channel
    /// so we don't queue duplicates. Reset to false by the writer thread just
    /// before it drains `render_slot`.
    pub(crate) render_in_queue: Arc<AtomicBool>,
    /// Receives draw commands from the subprocess on a background thread.
    draw_rx: Option<Receiver<DrawCommand>>,
    /// True when the stdout reader has queued at least one draw command since
    /// the last `drain_draw_commands` (issue #2021). Set by the reader thread
    /// AFTER each send; cleared by the host BEFORE each drain, so a queued
    /// command can never be left behind with the flag unset. This is what
    /// lets `needs_background_tick` skip idle apps without ever stranding a
    /// spontaneous command (e.g. `emit.notify` outside a frame).
    pub(crate) draw_pending: Arc<AtomicBool>,
    /// The last fully committed frame (commands between two FrameDones).
    pub(crate) frame: Vec<RenderCommand>,
    /// Accumulates draw commands for the frame currently being received.
    pending_frame: Vec<RenderCommand>,
    /// Pending host app commands collected from the subprocess.
    pub(crate) pending_commands: Vec<AppCommand>,
    last_size: egui::Vec2,
    initialized: bool,
    runtime: PgapRuntime,
    scheduler_mode: scheduler::AppSchedulerMode,
    /// Absolute frame clock — `Some` iff `scheduler_mode` is `Continuous`.
    animation_clock: Option<scheduler::AnimationClock>,
    render_diag: RenderDiagnostics,
    pub(crate) pending_async_completions: usize,
    idle_render_poll_logged: bool,
    sdk: Option<String>,
    features_used: Vec<String>,
    /// workspace_root sent in Init — scopes all SecretGet calls.
    pub(crate) workspace_root: PathBuf,
    /// Directory containing the app's entry file — used to resolve relative
    /// asset paths such as image srcs in RenderCommand::Image.
    pub(crate) app_dir: PathBuf,
    /// Granted capabilities for this app instance.
    pub(crate) permissions: AppPermissions,
    /// Persisted three-state permission store. Updated on grant/deny decisions.
    /// Legacy store — kept in lockstep with `grant_store` until every call
    /// site reads through the unified broker.
    pub(crate) permission_store: crate::app::permissions::PermissionStore,
    /// Unified permissions broker store (`grants.toml`) — generalized grant
    /// model from docs/prm/permissions-broker.md. Capability requests
    /// evaluate here; modal decisions are recorded here and mirrored to the
    /// legacy store.
    pub(crate) grant_store: crate::broker::GrantStore,
    /// User-level permission posture from the channel `config.toml`
    /// `[permissions]` table — applied at the actor-default tiers of broker
    /// evaluation. `None` = no posture configured (broker falls back to Ask).
    pub(crate) posture: Option<crate::broker::PermissionPosture>,
    /// Host-owned app event timeline + undo checkpoints + subscriptions
    /// (docs/prm/undo-and-app-events.md). Production panes share the global
    /// instance; tests inject an isolated one.
    pub(crate) app_timeline: Arc<Mutex<crate::host::app_timeline::AppTimeline>>,
    /// Typed pipe registry.
    pub(crate) pipe_registry: Arc<Mutex<TypedPipeRegistry>>,
    pub(crate) run_registry: RunRegistry,
    pub(crate) pending_prompts: VecDeque<PendingPrompt>,
    /// AiQuery requests withheld pending first-run `ai.query` consent.
    /// Drained on consent resolution — dispatched if granted, errored if denied.
    pub(crate) deferred_ai_queries: VecDeque<DeferredAiQuery>,
    /// Capability-gated pane/permission requests withheld pending consent
    /// (yellow-state routing, stint 0017). Each entry pairs the gating
    /// capability with the original request. Drained on consent resolution —
    /// forwarded via `AppCommand::ForwardPaneRequest` if granted, answered
    /// with the standard denial JSON on the request's response_file if denied.
    pub(crate) deferred_gated_requests: Vec<(Capability, AppRequest)>,
    pub(crate) status_summary: Option<String>,
    /// Navigation stack maintained by `DrawCommand::PushNav` / `PopNav`.
    /// Each entry carries a stable `view_id` and a display `title`. When the
    /// stack is non-empty the pane chrome shows a back arrow + the top title,
    /// and Cmd+[ emits `PlexiEvent::NavBack` to the app.
    pub(crate) nav_stack: Vec<NavEntry>,
    pub(crate) outbound_events: VecDeque<PlexiEvent>,
    pub(crate) secret_input_buf: String,
    /// Recent stderr lines from the subprocess. Capped at the last
    /// `STDERR_RING_CAP` entries. Used by the in-pane error fallback that
    /// surfaces when an app emits no draw commands — the user sees the
    /// crash text in the pane instead of a silent blank screen.
    pub(crate) recent_stderr: Arc<Mutex<VecDeque<String>>>,
    keyboard_capture: bool,
    /// Shared HTTP broker. `Arc<dyn NetService>` so production panes all point
    /// at the same `UreqNetService` while tests can inject `MockNetService`.
    pub(crate) net: Arc<dyn NetService>,
    /// Channel for async HTTP responses from background request threads.
    /// `route_command` spawns one thread per `HttpRequest`; threads send their
    /// result here so the UI thread never blocks on network I/O.
    pub(crate) http_tx: Sender<PlexiEvent>,
    pub(crate) http_rx: Receiver<PlexiEvent>,
    /// Channel for async file picker results from background dialog threads.
    /// `route_command` spawns one thread per `OpenFilePicker`; the thread
    /// sends `PlexiEvent::FilePicked` or `FilePickCancelled` here when done.
    pub(crate) file_picker_tx: Sender<PlexiEvent>,
    pub(crate) file_picker_rx: Receiver<PlexiEvent>,
    /// `ai.query` broker (#284). `Arc<dyn AiBroker>` so production panes share
    /// a `LiveAiBroker` while tests can inject a `CannedBroker`. Dispatch runs
    /// on a worker thread so the UI never blocks on the LLM call.
    pub(crate) ai_broker: Arc<dyn AiBroker>,
    /// Audio device backend (#277). `Arc<dyn AudioDevice>` so production
    /// panes share a `CoreAudioDevice` while tests inject `MockAudioDevice`.
    /// Enumeration is synchronous; capture spawns a cpal stream that drives
    /// frames into the binary-pipe ring on the cpal callback thread.
    pub(crate) audio_device: Arc<dyn AudioDevice>,
    /// Live capture sessions, keyed on the binary `pipe_id` they're
    /// streaming into. Dropping a session tears down the cpal stream; we
    /// drop the entry on `pipe_close`, on app shutdown, or when the pipe
    /// allocation fails.
    pub(crate) audio_capture_sessions: HashMap<String, CaptureSession>,
    /// Live playback sessions (#341), keyed on the file path (source string).
    /// Dropping an entry stops playback. State transitions (pause/resume/stop)
    /// mutate the session in-place via `PlaybackSession::pause` etc.
    pub(crate) audio_playback_sessions: HashMap<String, crate::media::audio::PlaybackSession>,
    /// Per-pipe peak amplitude tracking for AudioMeter rendering (#341).
    /// Written by the cpal callback thread via the Arc<Mutex> clone stored in
    /// the frame sink; read by the UI thread in `ui()` before calling
    /// `render_draw_commands`. Keys are binary `pipe_id` strings.
    pub(crate) audio_peak_meters: std::sync::Arc<std::sync::Mutex<HashMap<String, f32>>>,
    /// MIDI device backend (#320). `Arc<dyn MidiDevice>` so production panes
    /// share a `CoreMidiDevice` while tests inject `MockMidiDevice`.
    pub(crate) midi_device: Arc<dyn MidiDevice>,
    /// Live MIDI input sessions, keyed on `port_id`. Dropping a session
    /// disconnects the CoreMIDI source; we drop on `CloseMidiInput`, on
    /// app shutdown, or on a connect failure.
    pub(crate) midi_input_sessions: HashMap<String, MidiInputSession>,
    /// Lazily-opened MIDI output handles, keyed on `port_id`. The first
    /// `SendMidi` to a given port_id creates the handle; subsequent sends
    /// reuse it. Dropped on app shutdown.
    pub(crate) midi_output_handles: HashMap<String, MidiOutputHandle>,
    /// Video decoder backend (#345). `Arc<dyn VideoDecoder>` so production
    /// panes share an `AvfVideoDecoder` while tests inject `MockVideoDecoder`.
    /// The factory selects `MockVideoDecoder` when `PLEXI_VIDEO=mock://...`
    /// is set, so the POC `examples/video-player/` app can exercise the
    /// substrate without AVFoundation. Production `AvfVideoDecoder::open`
    /// returns `Err(NotImplemented)` until #346 lands real backing.
    pub(crate) video_device: Arc<dyn VideoDecoder>,
    /// Live video handles, keyed on `handle_id` returned in `VideoOpenAck`.
    /// Dropping a handle tears down the decoder thread (mock) and closes
    /// the underlying binary pipe. The map is drained on app shutdown.
    pub(crate) video_handles: HashMap<u64, VideoHandle>,
    /// Per-handle pipe id, so `CloseVideo { handle_id }` can locate the
    /// pipe to close in the registry. Populated on `OpenVideo` success;
    /// drained alongside `video_handles`.
    pub(crate) video_pipe_ids: HashMap<u64, String>,
    /// Cancel flags for pending timers. Key = timer_id, value = Arc<AtomicBool> set to true to cancel.
    pub(crate) pending_timers: HashMap<String, std::sync::Arc<std::sync::atomic::AtomicBool>>,
    /// Observable lifecycle state (issue #316). Written by the stdout/stderr
    /// reader threads and the UI thread's per-frame `try_wait` poll; read
    /// by the UI thread to render the in-pane pill.
    pub(crate) lifecycle: Arc<LifecycleTracker>,
    /// When true, the click-to-reveal stderr overlay is displayed in the
    /// pane. Toggled by clicking a non-Running lifecycle pill.
    show_stderr_overlay: bool,
    /// SystemTime when the app first entered a crash/hung/protocol-error state.
    /// Stamped on first detection; cleared if state recovers to Running/Booting.
    crashed_at: Option<std::time::SystemTime>,
    /// Deadline for showing "✓ copied" overlay feedback. None = "C — copy report".
    copied_feedback_until: Option<std::time::Instant>,
    /// Count of pending notifications originating from this pane. Updated
    /// each frame by `PlexiApp` before tiling renders. Zero when no
    /// notifications are pending.
    pub(crate) pending_notification_count: usize,
    /// When true, `PlexiEvent::MouseMove` is delivered to the app every frame
    /// that the pointer moves over the pane. Controlled by
    /// `DrawCommand::SetMouseTracking { enabled }`. Off by default to avoid
    /// flooding apps that don't need continuous pointer tracking.
    mouse_tracking_enabled: bool,
    /// Minimum pane width from manifest [launch] section. Default: 120.0.
    pub(crate) manifest_min_width: f32,
    /// Minimum pane height from manifest [launch] section. Default: 80.0.
    pub(crate) manifest_min_height: f32,
    /// compact/regular threshold from manifest (or default 280.0).
    pub(crate) compact_threshold: f32,
    /// regular/full threshold from manifest (or default 480.0).
    pub(crate) regular_threshold: f32,
    /// Live minimum size override from DrawCommand::SetMinSize. Supersedes manifest when set.
    live_min_size: Option<(f32, f32)>,
    /// True while the pane is in "too small" state. Guards the transition log.
    too_small: bool,
    /// Cached state for `egui_commonmark` markdown rendering. Persists across
    /// frames so the parser doesn't re-allocate on every repaint.
    commonmark_cache: egui_commonmark::CommonMarkCache,
    /// Image load cache for `RenderCommand::Image` (#1144).
    pub(crate) image_cache: image_cache::ImageCache,
    /// Per-frame rendering state — TextInput buffers, scroll offsets, and
    /// accumulated outbound events from widget passes. Extracted from
    /// `ProcessApp` so each concern has a clear owner.
    pub(crate) render_session: RenderSession,
    /// Tools exposed by this pane via `DrawCommand::ExposeTools` (#398).
    /// Updated each time a new `ExposeTools` command arrives. The routing
    /// layer re-registers these in the global tool registry on each update.
    pub(crate) exposed_tools: Vec<crate::app_protocol::AiTool>,
    /// Active `StreamProcess` children, keyed on `correlation_id` (#358).
    /// Dropping an entry cancels nothing automatically — the cancel flag
    /// must be set and a signal sent before removing the handle.
    pub(crate) stream_handles: HashMap<String, StreamHandle>,
    /// Count of currently active `StreamProcess` reader threads. Incremented
    /// before spawn, decremented when the thread exits. Capped at
    /// `MAX_STREAM_THREADS` to bound peak stack memory.
    pub(crate) active_stream_threads: Arc<AtomicUsize>,
    /// MCP server handle — `Some` when the app has `[app.mcp]` in its manifest.
    pub(crate) mcp_server: Option<mcp_server::McpServerHandle>,
    /// Pending MCP tool call responses awaiting `AppRequest::McpToolResult`.
    /// Key = call_id, value = channel to the blocked HTTP handler thread.
    pub(crate) mcp_pending:
        std::collections::HashMap<String, std::sync::mpsc::SyncSender<mcp_server::McpToolResponse>>,
    /// Last egui context seen by `ui()`. Background stdout/stderr/reaper
    /// threads use this to wake the host when they enqueue work or flip
    /// lifecycle state.
    repaint_ctx: Arc<Mutex<Option<egui::Context>>>,
    /// Set to true when the app emits ControlCommand::CloseSelf. The host
    /// checks wants_close() each frame and calls close_pane gracefully,
    /// avoiding the crash-restart path that sys.exit() would trigger.
    wants_close_self: bool,
    /// True between a click event and the next FrameDone. Keeps the repaint
    /// cadence at 0ms (immediate) so the selection update renders without
    /// falling into the 100ms idle branch.
    click_awaiting_frame: bool,
    /// Launch arguments passed via CLI or SpawnPane. Forwarded in PlexiEvent::Init
    /// so the SDK can expose them as ctx.args.
    pub(crate) launch_args: Vec<String>,
    /// Number of `background_tick()` calls — observable proof for the #2021
    /// gating tests that idle apps are skipped and pending apps are ticked.
    #[cfg(test)]
    pub(crate) background_tick_count: usize,
}

impl ProcessApp {
    /// Spawn an app binary at `bin_path`.
    ///
    /// `workspace_root` must be an absolute existing directory — validated here.
    #[allow(clippy::too_many_arguments)]
    pub fn launch(
        type_id: impl Into<String>,
        display_name: impl Into<String>,
        bin_path: &PathBuf,
        cwd: &PathBuf,
        args: &[String],
        workspace_root: PathBuf,
        capabilities: std::collections::HashSet<Capability>,
        keyboard_capture: bool,
        mcp: Option<&crate::app::registry::McpSection>,
    ) -> Result<Self, std::io::Error> {
        let type_id: String = type_id.into();
        let display_name: String = display_name.into();
        // (broker construction below reads IqConfig from PlexiConfig — no captures needed)

        if workspace_root.as_os_str().is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "workspace_root must be non-empty",
            ));
        }
        if !workspace_root.is_absolute() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "workspace_root must be absolute: {}",
                    workspace_root.display()
                ),
            ));
        }
        if !workspace_root.is_dir() {
            log::warn!(
                "ProcessApp: workspace_root '{}' does not exist yet; proceeding",
                workspace_root.display()
            );
        }

        // Canonicalize to resolve symlinks and platform aliases (e.g. macOS /var → /private/var).
        // This ensures permission keys are stable across equivalent path spellings.
        let workspace_root = match workspace_root.canonicalize() {
            Ok(canonical) if canonical != workspace_root => {
                log::info!(
                    "ProcessApp: workspace_root canonicalized '{}' → '{}'",
                    workspace_root.display(),
                    canonical.display()
                );
                canonical
            }
            _ => workspace_root,
        };

        // STEP-9: environment isolation (spec invariant I-6).
        // Clear the inherited environment and whitelist only vars the app
        // legitimately needs. Strips OPENROUTER_API_KEY and every other
        // host credential — apps must use the iq.query / llm broker, never direct API access.
        const ENV_WHITELIST: &[&str] = &["HOME", "PATH", "LANG", "LC_ALL", "TERM", "USER", "SHELL"];

        // Resolve the bundled Python interpreter path — used both to build the
        // python3 command for .py entries and to prepend to PATH for PYTHONPATH setup.
        let bundle_contents = std::env::current_exe().ok().and_then(|exe| {
            exe.parent()
                .and_then(|p| p.parent())
                .map(|p| p.to_path_buf())
        });
        let bundled_py_bin = bundle_contents.as_ref().map(|c| {
            c.join("Resources")
                .join("assets")
                .join("python")
                .join("bin")
        });

        // .py entries are launched via python3 directly — no shebang or executable bit required.
        let is_python = bin_path.extension().and_then(|e| e.to_str()) == Some("py");
        let py_exe: Option<std::ffi::OsString> = if is_python {
            // Prefer the per-app venv Python when it exists (D2: per-app venv via uv).
            let venv_python = bin_path
                .parent()
                .map(|app_dir| app_dir.join(".venv").join("bin").join("python"))
                .filter(|p| p.exists());
            if let Some(ref vp) = venv_python {
                log::info!(
                    "ProcessApp[{type_id}]: using per-app venv Python at {}",
                    vp.display()
                );
            }
            Some(
                venv_python
                    .map(std::ffi::OsString::from)
                    .or_else(|| {
                        bundled_py_bin
                            .as_ref()
                            .map(|b| b.join("python3"))
                            .filter(|p| p.exists())
                            .map(std::ffi::OsString::from)
                    })
                    .unwrap_or_else(|| std::ffi::OsString::from("python3")),
            )
        } else {
            None
        };
        let mut cmd = if let Some(ref py) = py_exe {
            log::info!("ProcessApp[{type_id}]: launching .py entry via {:?}", py);
            let mut c = std::process::Command::new(py);
            c.arg(bin_path);
            c
        } else {
            std::process::Command::new(bin_path)
        };
        cmd.args(args)
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env_clear();
        for var in ENV_WHITELIST {
            if let Ok(v) = std::env::var(var) {
                cmd.env(var, v);
            }
        }
        // Pass through every PLEXI_* var (harness knobs, mock-device selectors)
        // EXCEPT PLEXI_SOCKET: app processes must not inherit socket routing.
        // Pane read/control flows through capability-gated PGAP requests
        // (panes.read / panes.control) instead of ambient CLI access.
        for (k, v) in std::env::vars() {
            if k.starts_with("PLEXI_") && k != "PLEXI_SOCKET" {
                cmd.env(k, v);
            }
        }
        let active_config_dir = crate::config::config_dir();
        let active_channel = channel_from_config_dir(&active_config_dir);
        cmd.env("PLEXI_CONFIG_DIR", &active_config_dir);
        if let Some(channel) = active_channel.as_deref() {
            cmd.env("PLEXI_CHANNEL", channel);
        } else {
            cmd.env_remove("PLEXI_CHANNEL");
        }
        log::info!(
            "ProcessApp[{type_id}]: environment profile dir={} channel={}",
            active_config_dir.display(),
            active_channel.as_deref().unwrap_or("main")
        );

        // Shared egui-context slot, populated on the first `ui()` frame.
        // Background threads (stdout/stderr readers, reaper, MCP connections,
        // async workers) use it to wake the host when work arrives.
        let repaint_ctx: Arc<Mutex<Option<egui::Context>>> = Arc::new(Mutex::new(None));

        // Start the MCP server when the manifest declares [app.mcp].
        let mcp_server_handle = mcp
            .map(|section| {
                match mcp_server::start_mcp_server(section.tools.clone(), Arc::clone(&repaint_ctx))
                {
                    Ok(handle) => {
                        cmd.env("PLEXI_MCP_PORT", handle.port.to_string());
                        cmd.env("PLEXI_MCP_TOKEN", &handle.token);
                        Some(handle)
                    }
                    Err(e) => {
                        log::error!("ProcessApp[{type_id}]: failed to start MCP server: {e}");
                        None
                    }
                }
            })
            .flatten();
        // Prepend the bundled Python interpreter's bin/ dir to PATH so that
        // dev-mode .py entries without the bundle still resolve python3 correctly.
        // Falls back silently to host PATH if the bundle runtime isn't present.
        if let Some(ref py_bin) = bundled_py_bin {
            if py_bin.exists() {
                let host_path = std::env::var("PATH").unwrap_or_default();
                cmd.env("PATH", format!("{}:{}", py_bin.display(), host_path));
            }
        }

        let bundle_sdk = bundle_contents
            .as_ref()
            .map(|p| p.join("Resources").join("sdk").join("python"));
        let pythonpath = crate::config::build_pythonpath(bundle_sdk.as_deref());
        log::info!("process_app[{type_id}]: PYTHONPATH={pythonpath}");

        // Static capability validation — runs before the real spawn.
        // For Python apps only; non-Python apps skip. Subprocess failures log warn and proceed.
        if let Some(ref py) = py_exe {
            let path_env = {
                let host_path = std::env::var("PATH").unwrap_or_default();
                if let Some(ref py_bin) = bundled_py_bin {
                    if py_bin.exists() {
                        format!("{}:{}", py_bin.display(), host_path)
                    } else {
                        host_path
                    }
                } else {
                    host_path
                }
            };
            if let Err(e) = static_capability_check(
                &type_id,
                bin_path,
                py.as_ref(),
                &pythonpath,
                &path_env,
                &capabilities,
            ) {
                return Err(e);
            }
        }

        cmd.env("PYTHONPATH", pythonpath);
        let mut child = cmd.spawn()?;

        let stdin = child.stdin.take().expect("stdin piped");
        let stdout = child.stdout.take().expect("stdout piped");
        let stderr = child.stderr.take().expect("stderr piped");

        // Stdin writer thread — owns the pipe and blocks on write_all.
        // The GUI thread sends StdinItem values via an unbounded channel; this
        // thread drains them. Render events are coalesced: render_slot holds
        // the latest serialised payload and FlushRender tokens are deduplicated
        // so a burst of Render events during startup never fills the queue and
        // silently drops itself.
        let render_slot: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let render_in_queue: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));
        let (event_tx, event_rx) = mpsc::channel::<StdinItem>();
        transport::spawn_stdin_writer(
            type_id.clone(),
            stdin,
            event_rx,
            Arc::clone(&render_slot),
            Arc::clone(&render_in_queue),
        );

        // Background thread: forward subprocess stderr to Plexi's logger,
        // capture into the recent-stderr ring buffer used by the in-pane
        // error fallback, AND scan each line for `Traceback` / `PANIC` /
        // `panicked at` so the lifecycle pill flips to Crashed without
        // waiting for `try_wait` to observe the eventual exit.
        let recent_stderr_capture = Arc::new(Mutex::new(VecDeque::<String>::new()));
        let lifecycle_tracker = Arc::new(LifecycleTracker::new());
        transport::spawn_stderr_reader(
            type_id.clone(),
            stderr,
            Arc::clone(&recent_stderr_capture),
            Arc::clone(&lifecycle_tracker),
            Arc::clone(&repaint_ctx),
        );

        // Background thread: read draw commands line-by-line and forward via channel.
        // Also feeds the lifecycle tracker:
        //   - Malformed JSON → on_parse_error() (counts toward ProtocolError).
        //   - Stdout EOF / read error → on_stdout_closed() (sticky Crashed).
        let (draw_tx, draw_rx) = mpsc::channel::<DrawCommand>();
        let draw_pending: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));
        transport::spawn_stdout_reader(
            type_id.clone(),
            stdout,
            draw_tx,
            Arc::clone(&draw_pending),
            Arc::clone(&lifecycle_tracker),
            Arc::clone(&repaint_ctx),
        );

        // Background reaper: blocks on waitpid so the UI thread never polls try_wait.
        // Fires on_process_exited() exactly once when the child exits — replaces the
        // per-frame try_wait() poll that was causing 600 syscalls/sec with 10 panes open.
        transport::spawn_reaper(
            type_id.clone(),
            child.id(),
            Arc::clone(&lifecycle_tracker),
            Arc::clone(&repaint_ctx),
        );

        let config_dir = crate::config::config_dir();
        let store = crate::app::permissions::PermissionStore::load_or_default(&config_dir);
        let (mut granted_caps, mut blocked_caps) =
            store.build_permission_sets(&type_id, &workspace_root, &capabilities);
        // Unified broker store (grants.toml) — load (which also migrates any
        // legacy entries) and overlay its records so broker-recorded
        // decisions take effect at launch. Deny wins over allow.
        let grant_store = crate::broker::GrantStore::load_or_default(&config_dir);
        let posture = crate::broker::PermissionPosture::load_from_config(&config_dir);
        log::info!(
            "broker: {} grant records loaded for '{}' capability overlay (posture: {})",
            grant_store.records().len(),
            type_id,
            posture.is_some()
        );
        let (broker_allowed, broker_denied) =
            grant_store.app_capability_sets(&type_id, &workspace_root);
        granted_caps.extend(broker_allowed);
        blocked_caps.extend(broker_denied.iter().copied());
        granted_caps.retain(|c| !blocked_caps.contains(c));
        let permissions = AppPermissions {
            capabilities: granted_caps,
            blocked: blocked_caps,
            is_builtin: false,
            allowed_hosts: vec![],
        };
        let (http_tx, http_rx) = mpsc::channel::<PlexiEvent>();
        let (file_picker_tx, file_picker_rx) = mpsc::channel::<PlexiEvent>();

        event_log::emit(HostEvent::AppSpawned {
            app_id: type_id.clone(),
            type_id: type_id.clone(),
            pane_id: 0,
            timestamp: event_log::now_timestamp(),
        });
        log::info!("app::{}: === SESSION START ===", type_id);

        Ok(Self {
            type_id,
            pane_id: 0,
            display_name,
            process: Some(child),
            event_tx: Some(event_tx),
            render_slot,
            render_in_queue,
            draw_rx: Some(draw_rx),
            draw_pending,
            frame: Vec::new(),
            pending_frame: Vec::new(),
            pending_commands: Vec::new(),
            last_size: egui::Vec2::ZERO,
            initialized: false,
            runtime: PgapRuntime::spawned_with_initial_render(),
            scheduler_mode: scheduler::AppSchedulerMode::default(),
            animation_clock: None,
            render_diag: RenderDiagnostics::new(),
            pending_async_completions: 0,
            idle_render_poll_logged: false,
            sdk: None,
            features_used: Vec::new(),
            workspace_root,
            app_dir: bin_path
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| std::env::temp_dir()),
            permissions,
            permission_store: store,
            grant_store,
            posture,
            app_timeline: crate::host::app_timeline::global(),
            pipe_registry: Arc::new(Mutex::new(TypedPipeRegistry::new(
                crate::config::config_dir().join("pipes"),
            ))),
            run_registry: RunRegistry::new(),
            pending_prompts: VecDeque::new(),
            deferred_ai_queries: VecDeque::new(),
            deferred_gated_requests: Vec::new(),
            status_summary: None,
            nav_stack: Vec::new(),
            outbound_events: VecDeque::new(),
            secret_input_buf: String::new(),
            recent_stderr: Arc::clone(&recent_stderr_capture),
            keyboard_capture,
            net: Arc::new(UreqNetService::new()),
            http_tx,
            http_rx,
            file_picker_tx,
            file_picker_rx,
            ai_broker: Arc::new(default_live_broker()),
            audio_device: default_audio_device(),
            audio_capture_sessions: HashMap::new(),
            audio_playback_sessions: HashMap::new(),
            audio_peak_meters: std::sync::Arc::new(std::sync::Mutex::new(HashMap::new())),
            midi_device: default_midi_device(),
            midi_input_sessions: HashMap::new(),
            midi_output_handles: HashMap::new(),
            video_device: crate::media::video::default_video_device(),
            video_handles: HashMap::new(),
            video_pipe_ids: HashMap::new(),
            pending_timers: HashMap::new(),
            lifecycle: lifecycle_tracker,
            show_stderr_overlay: false,
            crashed_at: None,
            copied_feedback_until: None,
            pending_notification_count: 0,
            mouse_tracking_enabled: false,
            manifest_min_width: 120.0,
            manifest_min_height: 80.0,
            compact_threshold: 280.0,
            regular_threshold: 480.0,
            live_min_size: None,
            too_small: false,
            commonmark_cache: egui_commonmark::CommonMarkCache::default(),
            image_cache: image_cache::ImageCache::new(),
            render_session: RenderSession::new(),
            exposed_tools: Vec::new(),
            stream_handles: HashMap::new(),
            active_stream_threads: Arc::new(AtomicUsize::new(0)),
            mcp_server: mcp_server_handle,
            mcp_pending: std::collections::HashMap::new(),
            repaint_ctx,
            wants_close_self: false,
            click_awaiting_frame: false,
            launch_args: args.to_vec(),
            #[cfg(test)]
            background_tick_count: 0,
        })
    }

    /// Transfer the last committed frame from a previous instance so the pane
    /// doesn't flicker blank during a hot-reload cycle. The transferred frame
    /// is displayed until the new subprocess sends its own FrameDone.
    pub fn transfer_frame_from(&mut self, old: &Self) {
        if !old.frame.is_empty() {
            self.frame = old.frame.clone();
            log::info!(
                "ProcessApp[{}]: transferred {} draw commands from previous instance",
                self.type_id,
                self.frame.len()
            );
        }
    }

    /// Set the pane ID for this app instance. Called by `open_process_app_pane`
    /// before the process is moved into the pane so that pipe peer routing can
    /// exclude the sending pane.
    pub fn set_pane_id(&mut self, id: u64) {
        self.pane_id = id;
        // ExposeTools may arrive before set_pane_id (deferred from routing to
        // avoid registering under the default pane_id=0). Flush them now.
        if !self.exposed_tools.is_empty() {
            if let Some(sender) = self.make_app_event_sender() {
                crate::plexi_ai::tool_dispatch::register(
                    id,
                    self.exposed_tools.clone(),
                    sender,
                    self.workspace_root.clone(),
                );
            }
        }
    }

    /// Create a `ProcessApp` suitable for host-harness tests. No subprocess is
    /// spawned. The returned `Sender<DrawCommand>` feeds the harness's `inject()`
    /// calls directly into `drain_draw_commands()` so the full `route_command`
    /// path executes without a real Python app.
    #[cfg(test)]
    pub fn new_for_test(
        pane_id: u64,
        permissions: crate::app::permissions::AppPermissions,
    ) -> (Self, Sender<DrawCommand>) {
        use crate::media::audio::MockAudioDevice;
        use crate::media::midi::MockMidiDevice;
        use crate::media::video::{MockVideoDecoder, MockVideoDecoderConfig};
        use crate::plexi_ai::broker::{AiBrokerRequest, AiBrokerResponse};

        struct NoopBroker;
        impl AiBroker for NoopBroker {
            fn dispatch(
                &self,
                _req: AiBrokerRequest,
                _on_delta: &mut dyn FnMut(crate::plexi_ai::turn_loop::TurnDelta<'_>),
            ) -> AiBrokerResponse {
                AiBrokerResponse::ok("noop".to_string(), 0, 0)
            }
        }

        struct NoopNet;
        impl NetService for NoopNet {
            fn http(
                &self,
                _method: &str,
                _url: &str,
                _headers: &std::collections::HashMap<String, String>,
                _body: Option<&str>,
            ) -> crate::host::services::HttpResponse {
                crate::host::services::HttpResponse {
                    status: 0,
                    body: String::new(),
                    error: Some("no network in tests".to_string()),
                    response_headers: std::collections::HashMap::<String, Vec<String>>::new(),
                }
            }
        }

        let (draw_tx, draw_rx) = mpsc::channel::<DrawCommand>();
        let (http_tx, http_rx) = mpsc::channel::<PlexiEvent>();
        let (file_picker_tx, file_picker_rx) = mpsc::channel::<PlexiEvent>();
        let lifecycle = Arc::new(LifecycleTracker::new());
        let app = Self {
            type_id: "test".to_string(),
            pane_id,
            display_name: "Test App".to_string(),
            process: None,
            event_tx: None,
            render_slot: Arc::new(Mutex::new(None)),
            render_in_queue: Arc::new(AtomicBool::new(false)),
            draw_rx: Some(draw_rx),
            draw_pending: Arc::new(AtomicBool::new(false)),
            frame: Vec::new(),
            pending_frame: Vec::new(),
            pending_commands: Vec::new(),
            last_size: egui::Vec2::ZERO,
            initialized: true,
            runtime: PgapRuntime::ready_for_test_with_initial_render(),
            scheduler_mode: scheduler::AppSchedulerMode::default(),
            animation_clock: None,
            render_diag: RenderDiagnostics::new(),
            pending_async_completions: 0,
            idle_render_poll_logged: false,
            sdk: None,
            features_used: Vec::new(),
            workspace_root: std::env::temp_dir(),
            app_dir: std::env::temp_dir(),
            permissions,
            permission_store: crate::app::permissions::PermissionStore::default(),
            grant_store: crate::broker::GrantStore::default(),
            posture: None,
            app_timeline: Arc::new(Mutex::new(
                crate::host::app_timeline::AppTimeline::default(),
            )),
            pipe_registry: Arc::new(Mutex::new(TypedPipeRegistry::new(
                std::env::temp_dir().join(format!("plexi-pipes-{}", uuid::Uuid::new_v4())),
            ))),
            run_registry: RunRegistry::new(),
            pending_prompts: VecDeque::new(),
            deferred_ai_queries: VecDeque::new(),
            deferred_gated_requests: Vec::new(),
            status_summary: None,
            nav_stack: Vec::new(),
            outbound_events: VecDeque::new(),
            secret_input_buf: String::new(),
            recent_stderr: Arc::new(Mutex::new(VecDeque::new())),
            keyboard_capture: false,
            net: Arc::new(NoopNet),
            http_tx,
            http_rx,
            ai_broker: Arc::new(NoopBroker),
            audio_device: Arc::new(MockAudioDevice::new()),
            audio_capture_sessions: HashMap::new(),
            audio_playback_sessions: HashMap::new(),
            audio_peak_meters: std::sync::Arc::new(std::sync::Mutex::new(HashMap::new())),
            midi_device: Arc::new(MockMidiDevice::new()),
            midi_input_sessions: HashMap::new(),
            midi_output_handles: HashMap::new(),
            video_device: Arc::new(MockVideoDecoder::new(MockVideoDecoderConfig::default())),
            video_handles: HashMap::new(),
            video_pipe_ids: HashMap::new(),
            pending_timers: HashMap::new(),
            lifecycle,
            show_stderr_overlay: false,
            crashed_at: None,
            copied_feedback_until: None,
            pending_notification_count: 0,
            mouse_tracking_enabled: false,
            manifest_min_width: 120.0,
            manifest_min_height: 80.0,
            compact_threshold: 280.0,
            regular_threshold: 480.0,
            live_min_size: None,
            too_small: false,
            commonmark_cache: egui_commonmark::CommonMarkCache::default(),
            image_cache: image_cache::ImageCache::new(),
            render_session: RenderSession::new(),
            exposed_tools: Vec::new(),
            stream_handles: HashMap::new(),
            active_stream_threads: Arc::new(AtomicUsize::new(0)),
            file_picker_tx,
            file_picker_rx,
            mcp_server: None,
            mcp_pending: std::collections::HashMap::new(),
            repaint_ctx: Arc::new(Mutex::new(None)),
            wants_close_self: false,
            click_awaiting_frame: false,
            launch_args: Vec::new(),
            background_tick_count: 0,
        };
        (app, draw_tx)
    }

    /// Current nav stack depth as tracked by `PushNav`/`PopNav` commands.
    /// Returns 0 for apps that have never pushed a view.
    pub fn nav_stack_depth(&self) -> usize {
        self.nav_stack.len()
    }

    /// The title of the current top-of-stack view, or `None` when the stack
    /// is empty (root view — no back navigation available).
    pub fn nav_top_title(&self) -> Option<&str> {
        self.nav_stack.last().map(|e| e.title.as_str())
    }

    /// The `view_id` the app should navigate *back to* — the entry below the
    /// current top, or empty string when the stack would return to root.
    pub fn nav_back_view_id(&self) -> String {
        let len = self.nav_stack.len();
        if len >= 2 {
            self.nav_stack[len - 2].view_id.clone()
        } else {
            String::new()
        }
    }

    // ── App events + undo (docs/prm/undo-and-app-events.md, Phase B) ────────

    /// Subscribe an actor to `publisher_app_id`'s event streams. Gated
    /// through the unified broker: one `TargetType::AppEventStream`
    /// evaluation per event name, with target id `"<app_id>::<event>"`
    /// (`"<app_id>::*"` when subscribing to all streams). Every evaluation
    /// must come back `Allow`; otherwise the strictest non-allow decision is
    /// returned and nothing is recorded. Returns the new subscription id on
    /// success.
    #[allow(clippy::too_many_arguments)]
    pub fn subscribe_event_stream(
        &mut self,
        publisher_app_id: &str,
        subscriber_type: crate::broker::ActorType,
        subscriber_id: &str,
        event_names: Vec<String>,
        payload_mode: crate::app_protocol::PayloadMode,
        trigger_mode: crate::app_protocol::TriggerMode,
        resource_id: Option<String>,
        duration: crate::broker::GrantDuration,
    ) -> Result<String, crate::broker::Decision> {
        use crate::broker::{Decision, PermissionRequest, TargetType};
        let targets: Vec<String> = if event_names.is_empty() {
            vec![format!("{publisher_app_id}::*")]
        } else {
            event_names
                .iter()
                .map(|n| format!("{publisher_app_id}::{n}"))
                .collect()
        };
        let mut strictest = Decision::Allow;
        for target in &targets {
            let req = PermissionRequest::new(
                subscriber_type,
                subscriber_id,
                TargetType::AppEventStream,
                target,
                Some(&self.workspace_root),
            );
            match self.grant_store.evaluate(&req, self.posture.as_ref()) {
                Decision::Allow => {}
                Decision::Deny => strictest = Decision::Deny,
                Decision::Ask => {
                    if strictest != Decision::Deny {
                        strictest = Decision::Ask;
                    }
                }
            }
        }
        if strictest != Decision::Allow {
            log::info!(
                "ProcessApp[{}]: subscription to '{publisher_app_id}' events for \
                 {subscriber_type:?} '{subscriber_id}' blocked by broker ({})",
                self.type_id,
                strictest.as_str()
            );
            return Err(strictest);
        }
        let subscription_id = format!("sub-{}", uuid::Uuid::new_v4());
        let record = crate::host::app_timeline::SubscriptionRecord {
            subscription_id: subscription_id.clone(),
            subscriber_type,
            subscriber_id: subscriber_id.to_string(),
            app_id: publisher_app_id.to_string(),
            event_names,
            payload_mode,
            trigger_mode,
            resource_id,
            duration,
            created_at: event_log::now_timestamp(),
        };
        self.app_timeline.lock().unwrap().add_subscription(record);
        Ok(subscription_id)
    }

    /// Request rollback of an undo checkpoint. Gated through the unified
    /// broker (`TargetType::UndoCheckpoint`, target id = the checkpoint's
    /// owning app id). On `Allow` the host starts the revision-verification
    /// round-trip by sending `PlexiEvent::RollbackVerify`; the rollback is
    /// only applied after `AppRequest::RollbackVerifyResult` confirms the
    /// app's current revision matches the checkpoint's `revision_after`.
    ///
    /// Checkpoints owned by this pane's app are verified through this pane's
    /// own outbound channel. Cross-pane checkpoints (Phase C) are delivered
    /// to the owning pane through the tool registry sender — the owning app
    /// must be running and have exposed tools to be reachable.
    pub fn request_rollback(
        &mut self,
        actor_type: crate::broker::ActorType,
        actor_id: &str,
        checkpoint_id: &str,
    ) -> Result<(), String> {
        use crate::broker::{Decision, PermissionRequest, TargetType};
        let (owning_app, owning_pane) = self
            .app_timeline
            .lock()
            .unwrap()
            .checkpoints()
            .iter()
            .find(|c| c.checkpoint_id == checkpoint_id)
            .map(|c| (c.app_id.clone(), c.pane_id))
            .ok_or_else(|| format!("unknown checkpoint '{checkpoint_id}'"))?;
        let req = PermissionRequest::new(
            actor_type,
            actor_id,
            TargetType::UndoCheckpoint,
            &owning_app,
            Some(&self.workspace_root),
        );
        match self.grant_store.evaluate(&req, self.posture.as_ref()) {
            Decision::Allow => {}
            decision => {
                let msg = format!(
                    "rollback of '{checkpoint_id}' blocked by broker for {actor_type:?} \
                     '{actor_id}': {}",
                    decision.as_str()
                );
                log::info!("ProcessApp[{}]: {msg}", self.type_id);
                return Err(msg);
            }
        }
        // Cross-pane checkpoints must be deliverable before mutating the
        // checkpoint state — fail fast when the owning pane is unreachable.
        let cross_pane = owning_app != self.type_id;
        if cross_pane && !crate::plexi_ai::tool_dispatch::pane_reachable(owning_pane) {
            return Err(format!(
                "checkpoint '{checkpoint_id}' belongs to app '{owning_app}' in pane \
                 {owning_pane}, which is not reachable — the owning app must be running \
                 and have exposed tools"
            ));
        }
        let verify = self
            .app_timeline
            .lock()
            .unwrap()
            .begin_rollback(checkpoint_id)
            .map_err(|e| e.to_string())?;
        log::info!(
            "ProcessApp[{}]: rollback verification dispatched for {checkpoint_id} \
             (resource '{}', expect rev '{}', cross_pane={cross_pane})",
            self.type_id,
            verify.resource_id,
            verify.expected_revision
        );
        let event = PlexiEvent::RollbackVerify {
            checkpoint_id: verify.checkpoint_id,
            resource_id: verify.resource_id,
            expected_revision: verify.expected_revision,
        };
        if cross_pane {
            if !crate::plexi_ai::tool_dispatch::send_event_to_pane(owning_pane, &event) {
                return Err(format!(
                    "checkpoint '{checkpoint_id}': pane {owning_pane} went away before \
                     the verification could be delivered"
                ));
            }
        } else {
            self.outbound_events.push_back(event);
        }
        Ok(())
    }

    /// Move queued event deliveries addressed to this pane's app (wire
    /// subscriptions are stamped `ActorType::App` + the subscriber's app id)
    /// into the outbound channel as `PlexiEvent::AppEvent`. Runs once per
    /// frame, before the outbound flush. Deliveries addressed to agent
    /// actors stay queued for the Phase C agent runtime.
    pub(crate) fn deliver_subscribed_events(&mut self) {
        let deliveries = {
            let mut timeline = self.app_timeline.lock().unwrap();
            if timeline.pending_delivery_count() == 0 {
                return;
            }
            timeline.take_deliveries_for(crate::broker::ActorType::App, &self.type_id)
        };
        for d in deliveries {
            log::info!(
                "ProcessApp[{}]: delivering app event '{}' (id {}) from '{}' \
                 (subscription {}, trigger={:?})",
                self.type_id,
                d.event,
                d.event_id,
                d.app_id,
                d.subscription_id,
                d.trigger_mode
            );
            self.outbound_events.push_back(PlexiEvent::AppEvent {
                subscription_id: d.subscription_id,
                app_id: d.app_id,
                event: d.event,
                event_id: d.event_id,
                resource_id: d.resource_id,
                trigger_mode: d.trigger_mode,
                summary: d.summary,
                payload: d.payload,
                state_ref: d.state_ref,
                created_at: d.created_at,
            });
        }
    }

    fn effective_min_size(&self) -> (f32, f32) {
        self.live_min_size
            .unwrap_or((self.manifest_min_width, self.manifest_min_height))
    }

    fn render_too_small_placeholder(
        &self,
        ui: &mut egui::Ui,
        pane_rect: egui::Rect,
        ctx: &AppRenderContext<'_>,
    ) {
        let painter = ui.painter();
        let cx = pane_rect.center();
        let name_font = egui::FontId::proportional(crate::ui::style::TEXT_CAPTION);
        let (eff_w, eff_h) = self.effective_min_size();
        painter.text(
            cx - egui::vec2(0.0, 10.0),
            egui::Align2::CENTER_CENTER,
            &self.display_name,
            name_font,
            ctx.colors.text_dim,
        );
        let hint_font = egui::FontId::proportional(crate::ui::style::TEXT_HINT);
        painter.text(
            cx + egui::vec2(0.0, 10.0),
            egui::Align2::CENTER_CENTER,
            &format!("needs {eff_w:.0} × {eff_h:.0} px"),
            hint_font,
            ctx.colors.text_dim,
        );
    }

    pub(crate) fn send_event(&mut self, event: &PlexiEvent) {
        match serde_json::to_string(event) {
            Ok(mut line) => {
                line.push('\n');
                match event {
                    PlexiEvent::Render { .. } => {
                        // Coalesce: store the latest payload and enqueue a
                        // FlushRender token only once. If a token is already
                        // queued the writer will pick up the new payload from
                        // the slot when it drains — no extra token needed.
                        *self.render_slot.lock().unwrap() = Some(line);
                        if !self.render_in_queue.swap(true, Ordering::Relaxed) {
                            if let Some(tx) = &self.event_tx {
                                if tx.send(StdinItem::FlushRender).is_err() {
                                    log::debug!(
                                        "ProcessApp[{}]: stdin writer thread exited",
                                        self.type_id
                                    );
                                    self.event_tx = None;
                                }
                            }
                        }
                    }
                    _ => {
                        if let Some(tx) = &self.event_tx {
                            if tx.send(StdinItem::Event(line)).is_err() {
                                log::debug!(
                                    "ProcessApp[{}]: stdin writer thread exited",
                                    self.type_id
                                );
                                self.event_tx = None;
                            }
                        }
                    }
                }
            }
            Err(e) => log::error!("ProcessApp: failed to serialize event: {e}"),
        }
    }

    fn mark_render_needed(&mut self, reason: &'static str) {
        if self.runtime.request_render_now() {
            log::info!("ProcessApp[{}]: render requested ({reason})", self.type_id);
        }
    }

    fn mark_render_needed_after(&mut self, reason: &'static str, delay: std::time::Duration) {
        if self.runtime.request_render_after(delay) {
            if reason != "schedule_render" {
                log::info!(
                    "ProcessApp[{}]: render requested ({reason}, after {}ms)",
                    self.type_id,
                    delay.as_millis()
                );
            }
        }
    }

    fn send_render_if_needed(&mut self, size: egui::Vec2) -> Option<u64> {
        let now = std::time::Instant::now();
        match self.runtime.poll_render(now) {
            RenderPoll::Send { frame_id } => {
                self.send_event(&PlexiEvent::Render {
                    frame_id,
                    rect: crate::app_protocol::Rect {
                        x: 0.0,
                        y: 0.0,
                        w: size.x,
                        h: size.y,
                    },
                });
                self.render_diag.record_render_sent(
                    frame_id,
                    self.scheduler_mode.next_frame_delay(),
                    now,
                );
                if let Some(clock) = self.animation_clock.as_mut() {
                    self.runtime
                        .request_render_at(clock.deadline_after_send(now));
                }
                Some(frame_id)
            }
            RenderPoll::Waiting { .. }
            | RenderPoll::InFlight
            | RenderPoll::NotReady
            | RenderPoll::Idle
            | RenderPoll::Terminal => None,
        }
    }

    /// Abandon a Render that has been in flight longer than
    /// `RENDER_IN_FLIGHT_TIMEOUT` with no FrameDone (issue #2208). Clears the
    /// in-flight state so the repaint scheduler stops polling, and surfaces
    /// the app through the existing hung lifecycle path. Called once per
    /// frame from both `ui()` and `background_tick()`.
    fn check_render_timeout(&mut self) {
        if let Some(frame_id) = self.runtime.abandon_render_if_stalled(
            std::time::Instant::now(),
            scheduler::RENDER_IN_FLIGHT_TIMEOUT,
        ) {
            log::warn!(
                "ProcessApp[{}]: no FrameDone for render frame {frame_id} within {:?}; \
                 marking app hung and stopping render polling",
                self.type_id,
                scheduler::RENDER_IN_FLIGHT_TIMEOUT,
            );
            self.lifecycle.on_render_timeout();
        }
    }

    fn arm_async_completion_wake(&mut self, reason: &'static str) {
        self.pending_async_completions = self.pending_async_completions.saturating_add(1);
        log::info!(
            "ProcessApp[{}]: async wake armed ({reason}); pending={}",
            self.type_id,
            self.pending_async_completions
        );
    }

    fn complete_async_wake(&mut self) {
        self.pending_async_completions = self.pending_async_completions.saturating_sub(1);
    }

    /// An `http_tx` clone that wakes the host after every send, so worker
    /// thread completions surface on the next frame instead of waiting for
    /// the bounded async-wake poll (#2021).
    pub(crate) fn waking_http_tx(&self, reason: &'static str) -> transport::WakingEventSender {
        transport::WakingEventSender::new(
            self.http_tx.clone(),
            Arc::clone(&self.repaint_ctx),
            reason,
        )
    }

    /// `file_picker_tx` counterpart of [`Self::waking_http_tx`].
    fn waking_file_picker_tx(&self, reason: &'static str) -> transport::WakingEventSender {
        transport::WakingEventSender::new(
            self.file_picker_tx.clone(),
            Arc::clone(&self.repaint_ctx),
            reason,
        )
    }

    fn drain_async_events(&mut self) {
        while let Ok(event) = self.http_rx.try_recv() {
            match &event {
                PlexiEvent::Timer { timer_id } => {
                    self.pending_timers.remove(timer_id);
                }
                PlexiEvent::AiStreamChunk { .. } | PlexiEvent::StreamChunk { .. } => {}
                PlexiEvent::StreamEnd { .. } => self.complete_async_wake(),
                _ => self.complete_async_wake(),
            }
            self.outbound_events.push_back(event);
            self.mark_render_needed("async_completion");
        }
        while let Ok(event) = self.file_picker_rx.try_recv() {
            self.complete_async_wake();
            self.outbound_events.push_back(event);
            self.mark_render_needed("async_completion");
        }
    }

    fn needs_async_wake_poll(&self) -> bool {
        self.pending_async_completions > 0
            || !self.pending_timers.is_empty()
            || self.active_stream_threads.load(Ordering::Relaxed) > 0
            || self.has_pending_mcp_work()
            || self.image_cache.has_pending()
    }

    /// An MCP server only counts as pending work while a tool call is queued
    /// (awaiting delivery to the app) or delivered-but-unanswered. A merely
    /// *present* MCP server must not pin the host in a permanent wake loop
    /// (#2021) — the MCP connection thread wakes the host when a call arrives.
    fn has_pending_mcp_work(&self) -> bool {
        if !self.mcp_pending.is_empty() {
            return true;
        }
        self.mcp_server
            .as_ref()
            .is_some_and(|mcp| mcp.call_queue.lock().map_or(true, |q| !q.is_empty()))
    }

    fn needs_headless_wake_poll(&self) -> bool {
        self.runtime.is_rendering() || self.needs_async_wake_poll()
    }

    /// Does this app have any background work that `background_tick()` would
    /// make progress on? Used by the host to skip idle background/parked apps
    /// on every frame (#2021). Every source of background work must be
    /// represented here:
    /// - queued draw commands from the app process → `draw_pending`
    ///   (set by the stdout reader, which also wakes the host)
    /// - host-queued events awaiting flush to the app → `outbound_events`
    /// - timer/HTTP/file-picker/AI/audio/MIDI worker completions →
    ///   `pending_async_completions` / `pending_timers` / stream threads
    /// - MCP tool calls → `has_pending_mcp_work`
    /// - async image loads → `image_cache.has_pending`
    /// - an in-flight render that may need the hung-timeout check →
    ///   `is_rendering` (via `needs_headless_wake_poll`)
    pub(crate) fn needs_background_tick(&self) -> bool {
        self.draw_pending.load(Ordering::Acquire)
            || !self.outbound_events.is_empty()
            || self.needs_headless_wake_poll()
    }

    /// Drain the MCP call queue and forward each request to the app as a
    /// `PlexiEvent::McpToolCall`. Called each frame (both `ui()` and
    /// `background_tick()`) so tool calls are processed even for background apps.
    pub(crate) fn poll_mcp_calls(&mut self) {
        let Some(mcp) = &self.mcp_server else { return };
        let requests: Vec<_> = mcp.call_queue.lock().unwrap().drain(..).collect();
        for req in requests {
            let call_id = req.call_id.clone();
            log::info!(
                "ProcessApp[{}]: dispatching McpToolCall call_id={call_id} tool={}",
                self.type_id,
                req.tool_name,
            );
            self.mcp_pending.insert(call_id.clone(), req.response_tx);
            self.outbound_events.push_back(PlexiEvent::McpToolCall {
                call_id,
                tool_name: req.tool_name,
                arguments: req.arguments,
            });
        }
    }

    /// Build an `AppEventSender` that can deliver `PlexiEvent`s to this pane
    /// from outside the `ProcessApp` (e.g. the tool dispatcher). Returns `None`
    /// when the stdin writer thread has already exited.
    pub(crate) fn make_app_event_sender(
        &self,
    ) -> Option<crate::plexi_ai::tool_dispatch::AppEventSender> {
        self.event_tx
            .as_ref()
            .map(|tx| crate::plexi_ai::tool_dispatch::AppEventSender { tx: tx.clone() })
    }

    fn flush_outbound_events(&mut self) {
        let mut flushed = false;
        while let Some(event) = self.outbound_events.pop_front() {
            self.send_event(&event);
            flushed = true;
        }
        if flushed {
            self.mark_render_needed("outbound_event");
        }
    }

    fn drain_draw_commands(&mut self) -> Vec<DrawCommand> {
        // Clear BEFORE draining (paired with the stdout reader's send-then-set)
        // so a command sent after this drain always leaves the flag set for
        // the next frame (#2021).
        self.draw_pending.store(false, Ordering::Release);
        let Some(rx) = self.draw_rx.as_ref() else {
            return vec![];
        };
        let mut cmds = Vec::new();
        loop {
            match rx.try_recv() {
                Ok(cmd) => cmds.push(cmd),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    let reason = self.runtime.mark_stdout_closed();
                    log::error!(
                        "ProcessApp[{}]: subprocess stdout closed — {reason:?}",
                        self.type_id,
                    );
                    self.draw_rx = None;
                    break;
                }
            }
        }
        cmds
    }

    fn record_fatal_error(&mut self, message: String, traceback: String) {
        log::error!(
            "ProcessApp[{}]: fatal SDK error: {message}\n{traceback}",
            self.type_id
        );
        self.runtime.mark_fatal_error();
        self.lifecycle.on_process_exited();
        if let Ok(mut buf) = self.recent_stderr.lock() {
            const STDERR_RING_CAP: usize = 32;
            if buf.len() >= STDERR_RING_CAP {
                buf.pop_front();
            }
            buf.push_back(format!("fatal_error: {message}"));
            for line in traceback.lines().take(STDERR_RING_CAP) {
                if buf.len() >= STDERR_RING_CAP {
                    buf.pop_front();
                }
                buf.push_back(line.to_string());
            }
        }
    }

    fn set_scheduler_mode(&mut self, mode: &str, fps: Option<u32>) {
        match scheduler::AppSchedulerMode::from_wire(mode, fps) {
            Ok(parsed) => {
                self.scheduler_mode = parsed;
                self.animation_clock = match parsed {
                    scheduler::AppSchedulerMode::Continuous { interval } => {
                        // Anchor the clock and kick the cadence — switching to
                        // continuous while Idle must start animating without
                        // waiting for an unrelated render request.
                        self.mark_render_needed("continuous_mode");
                        Some(scheduler::AnimationClock::new(
                            interval,
                            std::time::Instant::now(),
                        ))
                    }
                    scheduler::AppSchedulerMode::Idle | scheduler::AppSchedulerMode::Scheduled => {
                        None
                    }
                };
                log::info!(
                    "ProcessApp[{}]: scheduler mode set to {parsed:?}",
                    self.type_id
                );
            }
            Err(e) => {
                log::error!("ProcessApp[{}]: {e}", self.type_id);
                self.runtime.mark_protocol_error(e);
                self.lifecycle.on_parse_error();
            }
        }
    }

    /// Draw the lifecycle pill in the top-right corner of `pane_rect` and
    /// return its interaction response. Returns `None` for `Running` —
    /// the healthy state is intentionally invisible.
    ///
    /// Colour rules:
    /// - Booting        — faint blue/grey
    /// - Hung           — yellow
    /// - Crashed        — red
    /// - ProtocolError  — red
    fn draw_lifecycle_pill(
        &self,
        ui: &mut egui::Ui,
        pane_rect: egui::Rect,
        state: LifecycleState,
    ) -> Option<egui::Response> {
        let (label, fill_hex, fg_hex) = match state {
            LifecycleState::Running => return None,
            LifecycleState::Booting => ("starting", "#3a4a5a", "#cfd6e0"),
            LifecycleState::Hung => ("hung", "#d4a017", "#1e1e2e"),
            LifecycleState::Crashed => ("crashed", "#cc3838", "#ffffff"),
            LifecycleState::ProtocolError => ("protocol error", "#cc3838", "#ffffff"),
        };

        // Anchor the pill to the top-right corner with a small inset so it
        // doesn't sit flush against the pane edge. We measure the label
        // width with the same font metrics `render_badge` uses, then
        // position the pill's *left edge* such that its right edge lands
        // at `pane_rect.max.x - inset`.
        let font_size = 11.0;
        let radius = crate::ui::style::RADIUS_BADGE;
        let inset = 8.0;
        let font_id = egui::FontId::proportional(font_size);
        let galley =
            ui.fonts(|f| f.layout_no_wrap(label.to_string(), font_id, egui::Color32::BLACK));
        let text_w = galley.size().x;
        let text_h = galley.size().y;
        let pill_w =
            (text_w + crate::ui::style::BADGE_PAD_H * 2.0).max(crate::ui::style::BADGE_MIN_W);
        let pill_h = text_h + crate::ui::style::BADGE_PAD_V * 2.0;
        let pill_x_abs = pane_rect.max.x - inset - pill_w;
        let pill_y_center_abs = pane_rect.min.y + inset + pill_h / 2.0;

        // render_badge expects pane-relative `x` (left edge) and `y_center`,
        // and adds `origin` itself. Pass `pane_rect.min` as origin so the
        // helper paints inside the pane.
        let origin = pane_rect.min;
        let x_rel = pill_x_abs - origin.x;
        let y_center_rel = pill_y_center_abs - origin.y;
        // Clip to the pane rect so a pill near a tight pane edge doesn't
        // bleed into a neighbouring pane.
        render::render_badge(
            ui,
            origin,
            pane_rect,
            x_rel,
            y_center_rel,
            label,
            fill_hex,
            fg_hex,
            font_size,
            radius,
        );

        let pill_rect = egui::Rect::from_min_size(
            egui::pos2(pill_x_abs, pill_y_center_abs - pill_h / 2.0),
            egui::vec2(pill_w, pill_h),
        );
        // Stable id so toggling state across frames doesn't rebuild the
        // interaction widget. type_id is unique-per-pane in practice.
        let id = ui.id().with(("lifecycle_pill", &self.type_id));
        Some(ui.interact(pill_rect, id, egui::Sense::click()))
    }

    /// Draw a small notification badge in the top-left corner of `pane_rect`
    /// when there are pending notifications from this pane. Uses the theme
    /// accent color to visually match the sidebar badge style. Shows count if
    /// > 1, "1" if exactly one.
    fn draw_notification_indicator(
        &self,
        ui: &mut egui::Ui,
        pane_rect: egui::Rect,
        count: usize,
        colors: &crate::ui::theme::Colors,
    ) {
        let label = if count > 9 {
            "9+".to_string()
        } else {
            count.to_string()
        };
        let font_size = 11.0;
        let radius = crate::ui::style::RADIUS_BADGE;
        let inset = 8.0;
        let font_id = egui::FontId::proportional(font_size);
        let fg_color = egui::Color32::from_rgb(0x1e, 0x1e, 0x2e);
        let galley = ui.fonts(|f| f.layout_no_wrap(label.clone(), font_id, fg_color));
        let text_w = galley.size().x;
        let text_h = galley.size().y;
        let pill_w =
            (text_w + crate::ui::style::BADGE_PAD_H * 2.0).max(crate::ui::style::BADGE_MIN_W);
        let pill_h = text_h + crate::ui::style::BADGE_PAD_V * 2.0;
        let pill_rect = egui::Rect::from_min_size(
            egui::pos2(pane_rect.min.x + inset, pane_rect.min.y + inset),
            egui::vec2(pill_w, pill_h),
        );
        let painter = ui.painter().with_clip_rect(pane_rect);
        painter.rect_filled(pill_rect, radius, colors.accent);
        let text_x = pill_rect.center().x - text_w / 2.0;
        let text_y = pill_rect.center().y - text_h / 2.0;
        painter.galley(egui::pos2(text_x, text_y), galley, fg_color);
    }

    /// Drain the buffer for `id` and queue a `TextSubmitted` event. Default
    /// UX is "field clears on submit" — the next TextInput emit with the
    /// same id starts empty. Public to the crate for unit tests that
    /// don't go through egui rendering.
    #[cfg(test)]
    pub(crate) fn submit_text_input(&mut self, id: &str) {
        let ev = self.render_session.submit_text_input(id);
        self.outbound_events.push_back(ev);
    }

    /// Test hook: shift the in-flight render's start time into the past so
    /// the render-in-flight timeout can be exercised without sleeping.
    #[cfg(test)]
    pub(crate) fn backdate_in_flight_render(&mut self, by: std::time::Duration) {
        self.runtime.backdate_in_flight_render(by);
    }

    /// Test hook: is a Render transaction currently in flight?
    #[cfg(test)]
    pub(crate) fn render_in_flight_for_test(&self) -> bool {
        self.runtime.is_rendering()
    }

    /// Pump event I/O for a pane that is not currently rendered.
    ///
    /// Active-context panes are fully updated by `ui()` each frame. Non-active
    /// panes never get `ui()` called, so `http_rx` (where timer events land) is
    /// never drained and the Python process never receives those events — timers
    /// stall and notifications never fire.
    ///
    /// This does the minimal work to keep background apps alive:
    /// 1. Drain `http_rx` → `outbound_events`
    /// 2. Flush `outbound_events` → Python stdin
    /// 3. Drain `draw_rx` → route control commands (timers, notifications, etc.)
    ///
    /// Visual draw commands are discarded — there is no pane to render into.
    pub(crate) fn background_tick(&mut self) {
        #[cfg(test)]
        {
            self.background_tick_count += 1;
        }
        self.drain_async_events();
        self.poll_mcp_calls();
        self.flush_outbound_events();
        // A hung in-flight render must not keep the headless wake poll alive
        // forever either (`needs_headless_wake_poll` checks `is_rendering`).
        self.check_render_timeout();
        for cmd in self.drain_draw_commands() {
            match cmd {
                DrawCommand::Host(h) => self.route_command(h),
                DrawCommand::Control(ControlCommand::Log { level, message }) => {
                    let target = format!("app::{}", self.type_id);
                    match level.as_str() {
                        "error" => log::error!(target: &target, "{message}"),
                        "warn" => log::warn!(target: &target, "{message}"),
                        "debug" => log::debug!(target: &target, "{message}"),
                        _ => log::info!(target: &target, "{message}"),
                    }
                }
                DrawCommand::Control(ControlCommand::FatalError { message, traceback }) => {
                    self.record_fatal_error(message, traceback);
                }
                DrawCommand::Control(ControlCommand::FrameDone { frame_id }) => {
                    match self.runtime.complete_frame(frame_id) {
                        FrameDoneOutcome::Matched => {}
                        FrameDoneOutcome::Unexpected { expected, got } => {
                            log::error!(
                                "ProcessApp[{}]: background FrameDone frame_id={got} expected={expected:?}",
                                self.type_id,
                            );
                            self.lifecycle.on_parse_error();
                        }
                    }
                    self.pending_frame.clear();
                    self.click_awaiting_frame = false;
                    self.lifecycle.on_frame_done();
                }
                DrawCommand::Control(ControlCommand::Ready { sdk, features_used }) => {
                    self.sdk = Some(sdk);
                    self.features_used = features_used;
                    self.runtime.mark_ready();
                }
                DrawCommand::Control(ControlCommand::SetSchedulerMode { mode, fps }) => {
                    self.set_scheduler_mode(&mode, fps);
                }
                DrawCommand::Control(_) => {} // Ready/etc. irrelevant without a pane
                DrawCommand::Render(_) => {}  // No pane to render into
            }
        }
    }
}

impl App for ProcessApp {
    fn type_id(&self) -> &'static str {
        Box::leak(self.type_id.clone().into_boxed_str())
    }

    fn display_name(&self) -> String {
        self.display_name.clone()
    }

    fn keyboard_capture(&self) -> bool {
        self.keyboard_capture
    }

    fn wants_close(&self) -> bool {
        self.wants_close_self
    }

    fn queue_outbound_event(&mut self, event: crate::app_protocol::PlexiEvent) {
        self.outbound_events.push_back(event);
        self.mark_render_needed("queued_outbound_event");
    }

    fn ui(&mut self, ui: &mut egui::Ui, ctx: &AppRenderContext<'_>) {
        let size = ui.available_size();
        if let Ok(mut repaint_ctx) = self.repaint_ctx.lock() {
            *repaint_ctx = Some(ui.ctx().clone());
        }

        self.poll_mcp_calls();
        self.flush_outbound_events();

        // Lifecycle: track user-input recency on this pane. Only required
        // for the Hung detector — we just need a "did the user touch this
        // window in the last N seconds" signal, not a per-event log. egui's
        // per-frame input snapshot exposes that directly.
        let had_input = ui.input(|i| {
            !i.events.is_empty()
                || i.pointer.any_pressed()
                || i.pointer.any_down()
                || i.pointer.is_moving()
        });
        if had_input {
            self.lifecycle.on_user_input();
        }

        // Lifecycle: drive the time-based Hung check once per frame.
        self.lifecycle.tick_check_hung();

        // Render-in-flight timeout: an app that never sends FrameDone must
        // not keep the host repaint-polling forever (issue #2208).
        self.check_render_timeout();

        if !self.initialized {
            self.initialized = true;
            self.last_size = size;
            let cap_strings: Vec<String> = self
                .permissions
                .capabilities
                .iter()
                .map(|c| c.to_string())
                .collect();
            log::info!(
                "ProcessApp[{}]: sending Init with {} launch arg(s)",
                self.type_id,
                self.launch_args.len()
            );
            self.send_event(&PlexiEvent::Init {
                protocol: "pgap/3".to_string(),
                app_id: self.type_id.clone(),
                workspace_root: self.workspace_root.clone(),
                capabilities: cap_strings,
                feature_flags: vec!["pane_groups_v1".into()],
                compact_threshold: self.compact_threshold,
                regular_threshold: self.regular_threshold,
                theme: ctx.colors.to_theme_map(),
                args: self.launch_args.clone(),
                state: None,
            });
            // Inject persisted state before first render so on_inject runs with data.
            let state = load_app_state(&self.type_id, &self.workspace_root);
            self.outbound_events
                .push_back(PlexiEvent::InjectState { payload: state });
            log::info!(
                "ProcessApp[{}]: injected persisted state at startup",
                self.type_id
            );
        }

        if (size - self.last_size).length() > 1.0 {
            self.last_size = size;
            self.send_event(&PlexiEvent::Resize {
                width: size.x,
                height: size.y,
            });
            self.mark_render_needed("resize");
        }

        // Size-class guard: if the pane is below the effective minimum, render a
        // host-owned placeholder and skip routing the render event to the app.
        let (eff_min_w, eff_min_h) = self.effective_min_size();
        let now_too_small = size.x < eff_min_w || size.y < eff_min_h;
        if now_too_small != self.too_small {
            self.too_small = now_too_small;
            if now_too_small {
                log::info!(
                    "ProcessApp[{}]: pane too small ({:.0}×{:.0} < {:.0}×{:.0}) — placeholder",
                    self.type_id,
                    size.x,
                    size.y,
                    eff_min_w,
                    eff_min_h
                );
            } else {
                log::info!(
                    "ProcessApp[{}]: pane restored ({:.0}×{:.0}) — resuming",
                    self.type_id,
                    size.x,
                    size.y
                );
            }
        }
        if now_too_small {
            let pane_rect = ui.available_rect_before_wrap();
            ui.painter()
                .rect_filled(pane_rect, 0.0, ctx.colors.terminal_bg);
            self.render_too_small_placeholder(ui, pane_rect, ctx);
            return;
        }

        self.send_render_if_needed(size);

        self.drain_async_events();

        // Per-frame: detect audio capture pipes whose drain thread exited due
        // to a write error (e.g. Broken pipe when the app-side socket closed
        // unexpectedly). Emit AudioCaptureError immediately so the app is
        // notified within one render frame rather than waiting for the next
        // start_audio_capture retry to discover the stale session.
        let failed_audio_pipes: Vec<String> = self
            .audio_capture_sessions
            .keys()
            .filter(|pipe_id| {
                self.pipe_registry
                    .lock()
                    .map(|r| r.drain_failed(pipe_id))
                    .unwrap_or(false)
            })
            .cloned()
            .collect();
        for pipe_id in failed_audio_pipes {
            log::warn!(
                "ProcessApp[{}]: AudioCapture pipe_id={pipe_id} drain failed — cleaning up",
                self.type_id
            );
            self.audio_capture_sessions.remove(&pipe_id);
            self.pipe_registry
                .lock()
                .expect("pipe_registry poisoned")
                .close(&pipe_id);
            if let Ok(mut m) = self.audio_peak_meters.lock() {
                m.remove(&pipe_id);
            }
            self.outbound_events
                .push_back(PlexiEvent::AudioCaptureError {
                    pipe_id,
                    error: "pipe drain failed (broken pipe)".to_owned(),
                });
        }

        let new_cmds = self.drain_draw_commands();

        for cmd in new_cmds {
            match cmd {
                DrawCommand::Control(c) => self.handle_control_command(ui, c),
                DrawCommand::Host(h) => self.route_command(h),
                DrawCommand::Render(r) => self.pending_frame.push(r),
            }
        }

        // A continuous app often completes its render transaction during this
        // drain. Send the next due Render immediately instead of waiting for a
        // separate host repaint; otherwise a 60Hz host loop can only drive
        // one app frame every two host frames.
        self.send_render_if_needed(size);

        // Render the current committed frame.
        //
        // Paint the background directly over the pane's available rect instead
        // of wrapping in an `egui::Frame` — `render_draw_commands` uses
        // `ui.painter()` for every primitive, which paints but never allocates
        // UI space. A Frame wrapper sizes to its *allocated* content and would
        // collapse to a tiny rect in the top-left, leaving a visible grey
        // square on top of the intended background.
        // Compute pane_rect ONCE here and hand it to every downstream
        // consumer (background paint, draw-command renderer, click region).
        // No downstream function derives geometry on its own — single
        // source of truth. The earlier two-sources bug (renderer used
        // ui.min_rect(), caller used available_rect_before_wrap()) was a
        // silent disagreement that clipped every draw to an empty rect.
        let pane_rect = ui.available_rect_before_wrap();
        ui.painter()
            .rect_filled(pane_rect, 0.0, ctx.colors.terminal_bg);
        let audio_peaks: HashMap<String, f32> = self
            .audio_peak_meters
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default();
        for (handle, result) in self.image_cache.poll(ui.ctx()) {
            let (status, message) = match result {
                Ok(()) => ("ok".to_string(), None),
                Err(e) => ("error".to_string(), Some(e)),
            };
            self.outbound_events.push_back(PlexiEvent::ImageLoaded {
                handle,
                status,
                message,
            });
        }
        let net_http_granted = self.permissions.is_builtin
            || self.permissions.capabilities.contains(&Capability::NetHttp);
        self.render_session.render(
            ui,
            pane_rect,
            &self.frame,
            ctx.colors,
            &mut self.commonmark_cache,
            &audio_peaks,
            self.pane_id,
            &mut self.image_cache,
            &self.app_dir,
            net_http_granted,
            ctx.is_focused,
        );
        self.outbound_events
            .extend(self.render_session.drain_events());

        // ── Error fallback ──────────────────────────────────────────────────
        // Surface recent stderr in the pane when:
        //   1. The app emitted no draw commands at all (still booting / never
        //      started rendering), OR
        //   2. The lifecycle says Crashed or Hung — overlays even if the app
        //      had previously committed a frame, so a kill -9 of an app
        //      mid-run shows the failure rather than a frozen last frame.
        //   3. The user clicked the lifecycle pill (show_stderr_overlay).
        let lifecycle_state = self.lifecycle.state();
        if matches!(
            lifecycle_state,
            LifecycleState::Crashed | LifecycleState::Hung | LifecycleState::ProtocolError
        ) {
            if self.crashed_at.is_none() {
                self.crashed_at = Some(std::time::SystemTime::now());
            }
        } else {
            self.crashed_at = None;
        }
        let stderr_overlay_active = self.show_stderr_overlay
            || self.frame.is_empty()
            || matches!(
                lifecycle_state,
                LifecycleState::Crashed | LifecycleState::Hung | LifecycleState::ProtocolError
            );
        if stderr_overlay_active {
            let stderr_lines: Vec<String> = self
                .recent_stderr
                .lock()
                .map(|b| b.iter().rev().take(8).cloned().collect::<Vec<_>>())
                .unwrap_or_default();
            if !stderr_lines.is_empty() {
                let painter = ui.painter();
                let title_pos = pane_rect.min + egui::vec2(16.0, 16.0);
                let header = match lifecycle_state {
                    LifecycleState::Crashed => {
                        format!("⚠  {} crashed — recent stderr:", self.type_id)
                    }
                    LifecycleState::Hung => format!("⚠  {} hung — recent stderr:", self.type_id),
                    LifecycleState::ProtocolError => {
                        format!("⚠  {} protocol error — recent stderr:", self.type_id)
                    }
                    _ => format!("⚠  {} emitted no frames — recent stderr:", self.type_id),
                };
                painter.text(
                    title_pos,
                    egui::Align2::LEFT_TOP,
                    header,
                    egui::FontId::proportional(13.0),
                    egui::Color32::from_rgb(0xff, 0x55, 0x55),
                );
                let mut y = title_pos.y + 24.0;
                let available_width = pane_rect.max.x - title_pos.x - 16.0;
                let font = egui::FontId::monospace(11.0);

                for line in stderr_lines.iter().rev() {
                    let mut layout_job = egui::text::LayoutJob::default();
                    layout_job.append(
                        line,
                        0.0,
                        egui::TextFormat {
                            font_id: font.clone(),
                            color: ctx.colors.text_dim,
                            ..Default::default()
                        },
                    );
                    layout_job.wrap = egui::text::TextWrapping {
                        max_width: available_width,
                        ..Default::default()
                    };

                    let galley = ui.ctx().fonts(|f| f.layout_job(layout_job));
                    painter.galley(
                        egui::pos2(title_pos.x, y),
                        galley.clone(),
                        ctx.colors.text_dim,
                    );

                    let line_height = galley.rows.len() as f32 * 14.0;
                    y += line_height;

                    if y > pane_rect.max.y - 16.0 {
                        break;
                    }
                }

                // C key: copy crash report to clipboard
                if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::C)) {
                    let state_label = match lifecycle_state {
                        LifecycleState::Crashed => "crashed",
                        LifecycleState::Hung => "hung",
                        LifecycleState::ProtocolError => "protocol error",
                        _ => "error",
                    };
                    let time_str = self
                        .crashed_at
                        .map(|t| {
                            chrono::DateTime::<chrono::Utc>::from(t)
                                .format("%Y-%m-%dT%H:%M:%SZ")
                                .to_string()
                        })
                        .unwrap_or_else(event_log::now_timestamp);
                    let report = format!(
                        "=== Plexi Crash Report ===\nApp:   {}\nState: {}\nTime:  {}\n\nRecent stderr ({} lines):\n{}",
                        self.type_id,
                        state_label,
                        time_str,
                        stderr_lines.len(),
                        stderr_lines.iter().map(String::as_str).collect::<Vec<_>>().join("\n"),
                    );
                    ui.ctx().copy_text(report);
                    self.copied_feedback_until =
                        Some(std::time::Instant::now() + std::time::Duration::from_secs(2));
                    log::info!(
                        "crash_overlay: copied report for '{}' ({} lines)",
                        self.type_id,
                        stderr_lines.len()
                    );
                }

                // Hint label in bottom-left of overlay
                let hint = if self
                    .copied_feedback_until
                    .map_or(false, |t| t > std::time::Instant::now())
                {
                    "✓ copied"
                } else {
                    "C — copy report"
                };
                ui.painter().text(
                    egui::pos2(pane_rect.min.x + 16.0, pane_rect.max.y - 20.0),
                    egui::Align2::LEFT_BOTTOM,
                    hint,
                    egui::FontId::proportional(11.0),
                    ctx.colors.text_dim,
                );
            }
        }

        // ── Lifecycle pill ──────────────────────────────────────────────────
        // Top-right corner. Hidden in Running. Click toggles the stderr
        // overlay (in addition to the auto-overlay rules above).
        // Notification badge: top-left corner of pane chrome. Only visible
        // when this pane has pending choice notifications awaiting input.
        if self.pending_notification_count > 0 {
            log::debug!(
                "pane::{}: notification_indicator count={}",
                self.type_id,
                self.pending_notification_count
            );
            self.draw_notification_indicator(
                ui,
                pane_rect,
                self.pending_notification_count,
                ctx.colors,
            );
        }

        let pill_response = self.draw_lifecycle_pill(ui, pane_rect, lifecycle_state);
        let pill_consumed_click = if let Some(response) = pill_response {
            if response.clicked() {
                self.show_stderr_overlay = !self.show_stderr_overlay;
            }
            // Even hover/down over the pill suppresses the pane click —
            // we don't want a click that targeted the pill to ALSO get
            // forwarded as a PlexiEvent::Click into the app behind it.
            response.clicked() || response.is_pointer_button_down_on()
        } else {
            false
        };

        // Detect pointer interactions over the pane rect and forward as
        // PlexiEvent::{Click,MouseDown,MouseUp,MouseMove}.
        //
        // Sense::click_and_drag() is required here — Sense::click() alone only
        // fires on button-release and does not track press or motion.
        let mouse_response = ui.interact(pane_rect, ui.id(), egui::Sense::click_and_drag());
        let mut needs_tracking_repaint = false;
        let mut needs_click_repaint = false;
        if !pill_consumed_click {
            let origin = pane_rect.min;

            // Read frame-level modifier state once; shared by all mouse events
            // this frame so each event carries the same consistent snapshot.
            let frame_mods = ui.input(|i| {
                let m = i.modifiers;
                Modifiers {
                    shift: m.shift,
                    ctrl: m.ctrl,
                    alt: m.alt,
                    cmd: m.command,
                }
            });

            // MouseDown — fires on the frame the primary or secondary button goes down.
            if let Some(pos) = mouse_response.interact_pointer_pos() {
                let is_primary_down = ui.input(|i| {
                    i.pointer.button_pressed(egui::PointerButton::Primary)
                        && pane_rect.contains(i.pointer.interact_pos().unwrap_or(pos))
                });
                let is_secondary_down = ui.input(|i| {
                    i.pointer.button_pressed(egui::PointerButton::Secondary)
                        && pane_rect.contains(i.pointer.interact_pos().unwrap_or(pos))
                });
                if is_primary_down {
                    log::info!(
                        "app::{}: mouse_down primary ({:.0},{:.0}) shift={} cmd={}",
                        self.type_id,
                        pos.x - origin.x,
                        pos.y - origin.y,
                        frame_mods.shift,
                        frame_mods.cmd
                    );
                    self.send_event(&PlexiEvent::MouseDown {
                        x: pos.x - origin.x,
                        y: pos.y - origin.y,
                        button: crate::app_protocol::MouseButton::Primary,
                        modifiers: frame_mods.clone(),
                    });
                    self.mark_render_needed("mouse_down");
                    needs_click_repaint = true;
                }
                if is_secondary_down {
                    self.send_event(&PlexiEvent::MouseDown {
                        x: pos.x - origin.x,
                        y: pos.y - origin.y,
                        button: crate::app_protocol::MouseButton::Secondary,
                        modifiers: frame_mods.clone(),
                    });
                    self.mark_render_needed("mouse_down");
                    needs_click_repaint = true;
                }
            }

            // MouseUp — fires on the frame any button is released, including
            // after a drag. clicked() only fires for clean clicks (no significant
            // drag); drag_released() / drag_released_by() cover the drag-then-
            // release case. Both paths also emit the legacy Click for compat.
            if let Some(pos) = mouse_response.interact_pointer_pos() {
                let x = pos.x - origin.x;
                let y = pos.y - origin.y;
                let primary_up = mouse_response.clicked() || mouse_response.drag_stopped();
                let secondary_up = mouse_response.secondary_clicked()
                    || mouse_response.drag_stopped_by(egui::PointerButton::Secondary);
                if primary_up {
                    self.send_event(&PlexiEvent::MouseUp {
                        x,
                        y,
                        button: crate::app_protocol::MouseButton::Primary,
                        modifiers: frame_mods.clone(),
                    });
                    self.send_event(&PlexiEvent::Click {
                        x,
                        y,
                        button: crate::app_protocol::MouseButton::Primary,
                    });
                    self.mark_render_needed("mouse_up");
                    needs_click_repaint = true;
                }
                if secondary_up {
                    self.send_event(&PlexiEvent::MouseUp {
                        x,
                        y,
                        button: crate::app_protocol::MouseButton::Secondary,
                        modifiers: frame_mods.clone(),
                    });
                    self.send_event(&PlexiEvent::Click {
                        x,
                        y,
                        button: crate::app_protocol::MouseButton::Secondary,
                    });
                    self.mark_render_needed("mouse_up");
                    needs_click_repaint = true;
                }
            }

            // MouseMove — only delivered when the app opts in via
            // DrawCommand::SetMouseTracking { enabled: true }.
            // During a drag the pointer may leave the pane; we continue sending
            // events while any button is held so the app can track drag-to-outside.
            if self.mouse_tracking_enabled {
                let pointer_state = ui.input(|i| i.pointer.clone());
                if let Some(pos) = pointer_state.latest_pos() {
                    let is_dragging = pointer_state.button_down(egui::PointerButton::Primary)
                        || pointer_state.button_down(egui::PointerButton::Secondary);
                    let is_moving = pointer_state.is_moving();
                    if (pane_rect.contains(pos) || is_dragging) && is_moving {
                        let mut buttons = Vec::new();
                        if pointer_state.button_down(egui::PointerButton::Primary) {
                            buttons.push(crate::app_protocol::MouseButton::Primary);
                        }
                        if pointer_state.button_down(egui::PointerButton::Secondary) {
                            buttons.push(crate::app_protocol::MouseButton::Secondary);
                        }
                        self.send_event(&PlexiEvent::MouseMove {
                            x: pos.x - origin.x,
                            y: pos.y - origin.y,
                            buttons,
                            modifiers: frame_mods,
                        });
                        self.mark_render_needed("mouse_move");
                        needs_tracking_repaint = true;
                    }
                }
            }
        }

        // Deliver any subscribed app events queued for this pane, then flush
        // events accumulated during this frame (broker AiResponse,
        // TextSubmitted, ScrollOffset, etc.) so apps receive them without
        // waiting for the next frame's start-of-ui flush.
        self.deliver_subscribed_events();
        self.flush_outbound_events();

        // Idle polling for async HTTP responses. Apps that need faster repaints
        // (games, animations) emit DrawCommand::ScheduleRender { after_ms } each frame.
        //
        // Pointer-tracking apps are a special case: while the pointer is actively
        // moving we keep the repaint cadence near 60 FPS so host->app hover state
        // does not feel sticky.
        if needs_click_repaint {
            self.click_awaiting_frame = true;
        }

        match scheduler::decide_repaint(scheduler::RepaintInputs {
            click_now: needs_click_repaint,
            click_awaiting_frame: self.click_awaiting_frame,
            pointer_tracking: needs_tracking_repaint,
            render_delay: self.runtime.pending_repaint_delay(),
            render_in_flight: self.runtime.is_rendering(),
            async_wake: self.needs_async_wake_poll(),
        }) {
            scheduler::RepaintDecision::Now => ui.ctx().request_repaint(),
            scheduler::RepaintDecision::After(delay) => ui.ctx().request_repaint_after(delay),
            scheduler::RepaintDecision::None => {
                if !self.idle_render_poll_logged {
                    log::info!(
                        "ProcessApp[{}]: idle render polling disabled; waiting for input, dirty state, or ScheduleRender",
                        self.type_id
                    );
                    self.idle_render_poll_logged = true;
                }
            }
        }
    }

    fn handle_key(&mut self, input: &egui::InputState) -> crate::app::app_trait::KeyDisposition {
        use crate::app::app_trait::KeyDisposition;
        // When a TextInput widget has focus, egui owns the keyboard — all
        // text and key events are consumed by the TextEdit widget. Don't
        // forward them to the app's on_key handler (typing "h" in the chat
        // input shouldn't trigger a tier change, for example).
        if self.render_session.text_input_has_focus {
            return KeyDisposition::Passthrough;
        }
        let mut consumed = false;
        for event in &input.events {
            match event {
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => {
                    // Keys that generate egui::Event::Text at the OS level —
                    // digits, letters, and punctuation. For these, Arm 2 (Text)
                    // delivers the OS-resolved character (including shift-layer),
                    // so Arm 1 (Key) must be suppressed to avoid a double call.
                    // Ctrl/Cmd-modified chords suppress Text generation at the OS
                    // level, so they still need Arm 1 to reach the app.
                    let is_printable_key = matches!(
                        key,
                        egui::Key::A
                            | egui::Key::B
                            | egui::Key::C
                            | egui::Key::D
                            | egui::Key::E
                            | egui::Key::F
                            | egui::Key::G
                            | egui::Key::H
                            | egui::Key::I
                            | egui::Key::J
                            | egui::Key::K
                            | egui::Key::L
                            | egui::Key::M
                            | egui::Key::N
                            | egui::Key::O
                            | egui::Key::P
                            | egui::Key::Q
                            | egui::Key::R
                            | egui::Key::S
                            | egui::Key::T
                            | egui::Key::U
                            | egui::Key::V
                            | egui::Key::W
                            | egui::Key::X
                            | egui::Key::Y
                            | egui::Key::Z
                            | egui::Key::Num0
                            | egui::Key::Num1
                            | egui::Key::Num2
                            | egui::Key::Num3
                            | egui::Key::Num4
                            | egui::Key::Num5
                            | egui::Key::Num6
                            | egui::Key::Num7
                            | egui::Key::Num8
                            | egui::Key::Num9
                            | egui::Key::Minus
                            | egui::Key::Equals
                            | egui::Key::OpenBracket
                            | egui::Key::CloseBracket
                            | egui::Key::Backslash
                            | egui::Key::Semicolon
                            | egui::Key::Quote
                            | egui::Key::Backtick
                            | egui::Key::Comma
                            | egui::Key::Period
                            | egui::Key::Slash
                            | egui::Key::Space
                            | egui::Key::Plus
                    );
                    // When a ListView is active, suppress bare j/k/up/down/enter
                    // forwarding — the list_view pass already handled these host-side.
                    if self.render_session.list_view_intercepts_nav
                        && !modifiers.ctrl
                        && !modifiers.command
                    {
                        let is_nav = matches!(
                            key,
                            egui::Key::J
                                | egui::Key::K
                                | egui::Key::ArrowDown
                                | egui::Key::ArrowUp
                                | egui::Key::Enter
                        );
                        if is_nav {
                            consumed = true;
                            continue;
                        }
                    }

                    // Cmd-modified chords are reserved for host shortcuts
                    // (Cmd+Enter zoom, Cmd+P palette, Cmd+Shift+A notifications,
                    // etc.). Apps can't shadow a host keybind; they use bare
                    // letters or non-Cmd modifiers instead.
                    if (!is_printable_key || modifiers.ctrl) && !modifiers.command {
                        self.send_event(&PlexiEvent::Key {
                            key: format!("{key:?}"),
                            modifiers: Modifiers {
                                shift: modifiers.shift,
                                ctrl: modifiers.ctrl,
                                alt: modifiers.alt,
                                cmd: modifiers.command,
                            },
                        });
                    }
                    consumed = true;
                }
                egui::Event::Text(text) => {
                    for ch in text.chars() {
                        if ch.is_control() {
                            continue;
                        }
                        // Suppress j/k text events when a ListView is active
                        if self.render_session.list_view_intercepts_nav
                            && matches!(ch, 'j' | 'k' | 'J' | 'K')
                        {
                            continue;
                        }
                        self.send_event(&PlexiEvent::Key {
                            key: ch.to_string(),
                            modifiers: Modifiers::default(),
                        });
                    }
                    consumed = true;
                }
                egui::Event::Paste(text) => {
                    // Forward OS-clipboard paste to the focused app pane.
                    // egui emits this for both Cmd+V chords and the
                    // OS-menu / right-click → Paste action; both paths
                    // land here as a single event with the decoded UTF-8
                    // payload. Apps subscribe via `on_paste` in the SDK
                    // (or by checking `t == "paste"` directly).
                    //
                    // Queue rather than `send_event` so the event order
                    // matches frame boundaries — `flush_outbound_events`
                    // drains on the next `ui()` tick and writes the
                    // Paste line ahead of the Render that follows.
                    self.outbound_events
                        .push_back(PlexiEvent::Paste { text: text.clone() });
                    consumed = true;
                }
                _ => {}
            }
        }
        if consumed {
            self.mark_render_needed("keyboard_input");
            KeyDisposition::Consumed
        } else {
            KeyDisposition::Passthrough
        }
    }

    fn take_pending_commands(&mut self) -> Vec<AppCommand> {
        std::mem::take(&mut self.pending_commands)
    }

    fn sync_cwd(&mut self, new_cwd: &std::path::Path) {
        self.outbound_events.push_back(PlexiEvent::PathChanged {
            cwd: new_cwd.to_path_buf(),
        });
        self.mark_render_needed("path_changed");
    }

    fn serialize_state(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "type_id": self.type_id,
        }))
    }
}

impl ProcessApp {
    pub(crate) fn update_ai_config(&mut self, ai_config: Option<crate::config::AiConfig>) {
        self.ai_broker = Arc::new(LiveAiBroker::new(ai_config));
    }
}

/// Build a `LiveAiBroker` from the `[ai]` section of the current config.
/// If the section is absent, the broker is constructed with `None` and will
/// fail fast at dispatch time with a clear error directing the user to add
/// the section to config.toml.
fn default_live_broker() -> LiveAiBroker {
    let ai_config = crate::config::PlexiConfig::load().ai;
    LiveAiBroker::new(ai_config)
}

/// Build the production audio device. cpal in non-test builds; the mock
/// device under `cfg(test)` since cpal pulls in real CoreAudio APIs that
/// are unsuitable for unit tests. Tests that exercise the audio routing
/// path inject `Arc::new(MockAudioDevice::new())` directly into
/// `ProcessApp::audio_device`.
#[cfg(not(test))]
fn default_audio_device() -> Arc<dyn AudioDevice> {
    Arc::new(crate::media::audio::CoreAudioDevice::new())
}

#[cfg(test)]
fn default_audio_device() -> Arc<dyn AudioDevice> {
    Arc::new(crate::media::audio::MockAudioDevice::new())
}

/// Build the production MIDI device. CoreMIDI on non-test mac builds; an
/// empty stub on non-mac (CoreMidiDevice impl returns empty port lists and
/// PortNotFound for every open). Mock under `cfg(test)`.
///
/// Tests that exercise the MIDI routing path inject
/// `Arc::new(MockMidiDevice::new())` directly into `ProcessApp::midi_device`.
#[cfg(not(test))]
fn default_midi_device() -> Arc<dyn MidiDevice> {
    Arc::new(crate::media::midi::CoreMidiDevice::new())
}

#[cfg(test)]
fn default_midi_device() -> Arc<dyn MidiDevice> {
    Arc::new(crate::media::midi::MockMidiDevice::new())
}

/// Migrate a single `app_state/` directory to `app_states/` if the old path exists and the new
/// one does not yet. Logs a warning so operators know migration occurred.
fn migrate_app_state_dir(old_dir: &std::path::Path, new_dir: &std::path::Path) {
    if old_dir.exists() && !new_dir.exists() {
        match std::fs::rename(old_dir, new_dir) {
            Ok(()) => log::warn!(
                "load_app_state: migrated {} → {} (one-time rename)",
                old_dir.display(),
                new_dir.display()
            ),
            Err(e) => log::warn!(
                "load_app_state: could not migrate {} → {}: {e}",
                old_dir.display(),
                new_dir.display()
            ),
        }
    }
}

fn load_app_state(type_id: &str, workspace_root: &std::path::Path) -> serde_json::Value {
    let filename = format!("{type_id}.json");

    // Migrate old app_state/ → app_states/ on first access (workspace).
    let channel_dir = crate::config::workspace_channel_dir();
    let ws_old = workspace_root.join(&channel_dir).join("app_state");
    let ws_new = workspace_root.join(&channel_dir).join("app_states");
    migrate_app_state_dir(&ws_old, &ws_new);

    let workspace_path = ws_new.join(&filename);
    // Fallback: if migration failed (e.g. permission error), still read from old location
    let workspace_path_legacy = ws_old.join(&filename);
    if workspace_path.exists() {
        match std::fs::read(&workspace_path) {
            Err(e) => {
                log::warn!(
                    "load_app_state[{type_id}]: could not read workspace state {}: {e}",
                    workspace_path.display()
                );
            }
            Ok(bytes) => match serde_json::from_slice::<serde_json::Value>(&bytes) {
                Err(e) => {
                    log::warn!(
                        "load_app_state[{type_id}]: could not parse workspace state {}: {e}",
                        workspace_path.display()
                    );
                }
                Ok(val) => {
                    log::info!(
                        "load_app_state[{type_id}]: loaded workspace state from {}",
                        workspace_path.display()
                    );
                    return val;
                }
            },
        }
    } else if workspace_path_legacy.exists() {
        match std::fs::read(&workspace_path_legacy) {
            Err(e) => {
                log::warn!(
                    "load_app_state[{type_id}]: could not read legacy workspace state {}: {e}",
                    workspace_path_legacy.display()
                );
            }
            Ok(bytes) => {
                match serde_json::from_slice::<serde_json::Value>(&bytes) {
                    Err(e) => {
                        log::warn!("load_app_state[{type_id}]: could not parse legacy workspace state {}: {e}", workspace_path_legacy.display());
                    }
                    Ok(val) => {
                        log::info!(
                            "load_app_state[{type_id}]: loaded workspace state from legacy path {}",
                            workspace_path_legacy.display()
                        );
                        return val;
                    }
                }
            }
        }
    }

    // Migrate old app_state/ → app_states/ on first access (global).
    let global_old = crate::config::config_dir().join("app_state");
    let global_new = crate::config::config_dir().join("app_states");
    migrate_app_state_dir(&global_old, &global_new);

    let global_path = global_new.join(&filename);
    // Fallback: if migration failed (e.g. permission error), still read from old location
    let global_path_legacy = global_old.join(&filename);
    if global_path.exists() {
        match std::fs::read(&global_path) {
            Err(e) => {
                log::warn!(
                    "load_app_state[{type_id}]: could not read global state {}: {e}",
                    global_path.display()
                );
            }
            Ok(bytes) => match serde_json::from_slice::<serde_json::Value>(&bytes) {
                Err(e) => {
                    log::warn!(
                        "load_app_state[{type_id}]: could not parse global state {}: {e}",
                        global_path.display()
                    );
                }
                Ok(val) => {
                    log::info!(
                        "load_app_state[{type_id}]: loaded global state from {}",
                        global_path.display()
                    );
                    return val;
                }
            },
        }
    } else if global_path_legacy.exists() {
        match std::fs::read(&global_path_legacy) {
            Err(e) => {
                log::warn!(
                    "load_app_state[{type_id}]: could not read legacy global state {}: {e}",
                    global_path_legacy.display()
                );
            }
            Ok(bytes) => {
                match serde_json::from_slice::<serde_json::Value>(&bytes) {
                    Err(e) => {
                        log::warn!("load_app_state[{type_id}]: could not parse legacy global state {}: {e}", global_path_legacy.display());
                    }
                    Ok(val) => {
                        log::info!(
                            "load_app_state[{type_id}]: loaded global state from legacy path {}",
                            global_path_legacy.display()
                        );
                        return val;
                    }
                }
            }
        }
    }
    log::debug!("load_app_state[{type_id}]: no usable state file found, starting empty");
    serde_json::Value::Object(serde_json::Map::new())
}

fn cap_example_method(cap: &str) -> &'static str {
    match cap {
        "net.http" => "http_get",
        "ai.query" => "ai_query",
        "secrets.get" => "secret_get",
        "fs.pick" => "open_file_picker",
        "fs.read" => "fs_read",
        "fs.write" => "fs_write",
        "panes.spawn" => "spawn_pane",
        "panes.read" => "list_panes",
        "panes.control" => "send_to_pane",
        "midi.in" => "open_midi_input",
        "midi.out" => "send_midi",
        "video.playback" => "open_video",
        "audio.record" => "audio_capture",
        "audio.playback" => "audio_play",
        "timer" => "set_timer",
        "pipe.open" => "pipe_open",
        "terminal.bindings" => "request_linked_terminal",
        "llm" => "llm_query",
        _ => "related emit method",
    }
}

/// Run the app in introspect mode to detect required capabilities, then diff
/// against the manifest-declared set. Returns `Err` if required capabilities are
/// missing from the manifest; returns `Ok` for all infra failures (subprocess
/// error, timeout, bad JSON) — those are logged as warnings and never block launch.
pub(crate) fn static_capability_check(
    type_id: &str,
    bin_path: &std::path::Path,
    py_exe: &std::ffi::OsStr,
    pythonpath: &str,
    path_env: &str,
    declared: &std::collections::HashSet<crate::app::permissions::Capability>,
) -> Result<(), std::io::Error> {
    use std::sync::mpsc;
    use std::time::Duration;

    log::info!("ProcessApp[{type_id}]: running static capability check");

    const INTROSPECT_ENV_WHITELIST: &[&str] =
        &["HOME", "PATH", "LANG", "LC_ALL", "TERM", "USER", "SHELL"];
    let mut cmd = std::process::Command::new(py_exe);
    cmd.arg(bin_path)
        .arg("--plexi-introspect")
        .env_clear()
        .env("PYTHONPATH", pythonpath)
        .env("PATH", path_env)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .stdin(std::process::Stdio::null());
    for var in INTROSPECT_ENV_WHITELIST {
        if let Ok(v) = std::env::var(var) {
            cmd.env(var, v);
        }
    }

    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            log::warn!(
                "ProcessApp[{type_id}]: static capability check spawn failed: {e} — skipping"
            );
            return Ok(());
        }
    };
    let pid = child.id();

    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });

    let output = match rx.recv_timeout(Duration::from_secs(5)) {
        Ok(Ok(out)) => out,
        Ok(Err(e)) => {
            log::warn!("ProcessApp[{type_id}]: static capability check failed: {e} — skipping");
            return Ok(());
        }
        Err(_) => {
            log::warn!("ProcessApp[{type_id}]: static capability check timed out (pid {pid}) — killing and skipping");
            unsafe {
                libc::kill(pid as libc::pid_t, libc::SIGKILL);
            }
            return Ok(());
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        log::warn!(
            "ProcessApp[{type_id}]: static capability check exited with {:?} — skipping\nstderr: {}",
            output.status.code(),
            stderr.trim(),
        );
        return Ok(());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = match serde_json::from_str(stdout.trim()) {
        Ok(v) => v,
        Err(e) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            log::warn!(
                "ProcessApp[{type_id}]: static capability check invalid JSON ({e}) — skipping\nstdout: {}\nstderr: {}",
                stdout.trim(),
                stderr.trim(),
            );
            return Ok(());
        }
    };

    let required_caps: Vec<String> = json
        .get("required_capabilities")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    log::info!(
        "ProcessApp[{type_id}]: introspect found required capabilities: {:?}",
        required_caps
    );

    let declared_strs: std::collections::HashSet<&str> =
        declared.iter().map(|c| c.as_str()).collect();
    let required_set: std::collections::HashSet<&str> =
        required_caps.iter().map(|s| s.as_str()).collect();

    for cap in declared {
        if !required_set.contains(cap.as_str()) {
            log::warn!(
                "ProcessApp[{type_id}]: capability '{}' declared in manifest but not detected in code",
                cap.as_str()
            );
        }
    }

    let missing: Vec<&str> = required_caps
        .iter()
        .map(|s| s.as_str())
        .filter(|s| !declared_strs.contains(s))
        .collect();

    if !missing.is_empty() {
        let msg = missing.iter()
            .map(|cap| {
                let example_method = cap_example_method(cap);
                format!(
                    "App declares no '{cap}' capability but calls emit.{example_method}(). Add '{cap}' to manifest.toml [app.capabilities].",
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        log::error!("ProcessApp[{type_id}]: static capability validation failed:\n{msg}");
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, msg));
    }

    log::info!("ProcessApp[{type_id}]: static capability check passed");
    Ok(())
}

#[cfg(test)]
mod tests;

impl Drop for ProcessApp {
    fn drop(&mut self) {
        self.render_diag.flush_if_nonempty(
            &self.type_id,
            self.scheduler_mode.next_frame_delay(),
            std::time::Instant::now(),
            "drop",
        );
        self.runtime.mark_closing();
        // Cancel active StreamProcess children (#675) — same escalation as
        // CancelProcess: SIGTERM, then SIGKILL after 1s on a background thread.
        for (corr_id, handle) in self.stream_handles.drain() {
            log::info!(
                "ProcessApp[{}]: drop — cancelling stream {corr_id} pid={}",
                self.type_id,
                handle.pid
            );
            handle
                .cancel
                .store(true, std::sync::atomic::Ordering::Relaxed);
            unsafe {
                libc::kill(handle.pid as libc::pid_t, libc::SIGTERM);
            }
            let pid = handle.pid;
            let _ = std::thread::Builder::new()
                .name(format!("drop-sigkill-{pid}"))
                .spawn(move || {
                    std::thread::sleep(std::time::Duration::from_secs(1));
                    unsafe {
                        libc::kill(pid as libc::pid_t, libc::SIGKILL);
                        libc::waitpid(pid as libc::pid_t, std::ptr::null_mut(), libc::WNOHANG);
                    }
                });
        }
        // Unregister any tools this pane exposed so the global registry stays clean.
        crate::plexi_ai::tool_dispatch::unregister(self.pane_id);
        self.send_event(&PlexiEvent::Shutdown);
        event_log::emit(HostEvent::AppClosed {
            app_id: self.type_id.clone(),
            type_id: self.type_id.clone(),
            pane_id: 0,
            reason: None,
            timestamp: event_log::now_timestamp(),
        });
        log::info!("app::{}: === SESSION END ===", self.type_id);
        if let Some(mut child) = self.process.take() {
            // Three-phase shutdown escalation (#83):
            //   1. Wait up to 2s for the child to exit cleanly after the
            //      `Shutdown` event we just sent.
            //   2. SIGTERM (`child.kill()` on Unix) and wait another 1s.
            //   3. Final `wait()` — at this point the child has been
            //      SIGTERM'd; on the rare case it's still alive we let
            //      `wait()` block. Subprocess-not-responding-to-SIGTERM
            //      is a developer-side bug; logging it is enough.
            //
            // The `try_wait` poll loop avoids dragging the host UI thread
            // through a 2-second hang on a normal close (clean exits land
            // within tens of ms).
            const POLL_MS: u64 = 25;
            let shutdown_deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            let mut exited = false;
            while std::time::Instant::now() < shutdown_deadline {
                match child.try_wait() {
                    Ok(Some(_)) => {
                        self.runtime.mark_process_exited();
                        exited = true;
                        break;
                    }
                    Ok(None) => {
                        std::thread::sleep(std::time::Duration::from_millis(POLL_MS));
                    }
                    Err(e) if e.raw_os_error() == Some(libc::ECHILD) => {
                        // Background reaper already called waitpid and won the
                        // race — ECHILD means the child is gone. Treat as clean exit.
                        self.runtime.mark_process_exited();
                        exited = true;
                        break;
                    }
                    Err(e) => {
                        log::warn!(
                            "ProcessApp[{}]: try_wait error during shutdown: {e}",
                            self.type_id
                        );
                        break;
                    }
                }
            }
            if !exited {
                log::warn!(
                    "ProcessApp[{}]: did not exit within 2s of Shutdown — sending SIGTERM",
                    self.type_id
                );
                let _ = child.kill();
                let kill_deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
                while std::time::Instant::now() < kill_deadline {
                    match child.try_wait() {
                        Ok(Some(_)) => {
                            self.runtime.mark_process_exited();
                            break;
                        }
                        Ok(None) => {
                            std::thread::sleep(std::time::Duration::from_millis(POLL_MS));
                        }
                        Err(_) => break,
                    }
                }
                // Final blocking wait — at this point we've SIGTERM'd; if
                // the OS hasn't reaped yet, give it the floor. On Unix
                // `Child::kill` already SIGKILL-equivalents in `std`, so
                // the process should be dead.
                let _ = child.wait();
                self.runtime.mark_process_exited();
            }
        }
    }
}

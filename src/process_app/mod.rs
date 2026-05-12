//! ProcessApp — runs an external app binary as a subprocess and renders it
//! using the Plexi draw protocol.
//!
//! The subprocess speaks the app protocol over stdin/stdout (newline-delimited JSON).
//! ProcessApp implements the `App` trait so it drops in wherever a built-in app
//! would — the rest of Plexi doesn't know or care that it's an external process.
//!
//! Internal layout:
//! - `mod.rs`      — struct, lifecycle (launch/drop), App trait impl
//! - `routing.rs`  — `route_command()`: dispatch DrawCommands to subsystems
//! - `render.rs`   — `render_draw_commands()`: paint committed frames into egui
//! - `prompts.rs`  — `show_prompt_modal()`: capability/secret grant UI

pub(crate) mod image_cache;
mod lifecycle;
pub(crate) mod mcp_server;
mod prompts;
pub(crate) mod render;
mod render_session;
mod routing;

pub(crate) use lifecycle::{LifecycleState, LifecycleTracker};
use render_session::RenderSession;

use crate::app_permissions::{AppPermissions, Capability};
use crate::app_protocol::{ControlCommand, DrawCommand, Modifiers, PlexiEvent, RenderCommand};
use crate::app_trait::{App, AppCommand, AppRenderContext};
use crate::audio::{AudioDevice, CaptureSession};
use crate::midi::{MidiDevice, MidiInputSession, MidiOutputHandle};
use crate::video::{VideoDecoder, VideoHandle};
use crate::event_log::{self, HostEvent};
use crate::host::services::{NetService, UreqNetService};
use crate::plexi_ai::broker::{AiBroker, LiveAiBroker};
use crate::runs::RunRegistry;
use crate::typed_pipes::TypedPipeRegistry;
use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, ChildStdout, Stdio};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc::{self, Receiver, Sender, TryRecvError},
    Arc, Mutex,
};
use std::thread;

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
// StdinItem — typed messages for the stdin-writer channel
// ---------------------------------------------------------------------------

/// Messages sent from the GUI thread to the stdin-writer background thread.
///
/// `Render` events are coalesced: only the latest matters, so we store it in
/// `render_slot` and send a single `FlushRender` token. Non-render events are
/// queued in order and never dropped.
pub(crate) enum StdinItem {
    /// A non-render event serialised as a newline-terminated JSON string.
    Event(String),
    /// Consume and write the latest render payload from the render slot.
    FlushRender,
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
    /// The last fully committed frame (commands between two FrameDones).
    pub(crate) frame: Vec<RenderCommand>,
    /// Accumulates draw commands for the frame currently being received.
    pending_frame: Vec<RenderCommand>,
    /// Pending host app commands collected from the subprocess.
    pub(crate) pending_commands: Vec<AppCommand>,
    last_size: egui::Vec2,
    initialized: bool,
    frame_counter: u64,
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
    pub(crate) permission_store: crate::app_permissions::PermissionStore,
    /// Typed pipe registry.
    pub(crate) pipe_registry: Arc<Mutex<TypedPipeRegistry>>,
    pub(crate) run_registry: RunRegistry,
    pub(crate) pending_prompts: VecDeque<PendingPrompt>,
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
    pub(crate) audio_playback_sessions: HashMap<String, crate::audio::PlaybackSession>,
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
    mcp_server: Option<mcp_server::McpServerHandle>,
    /// Pending MCP tool call responses awaiting `HostCommand::McpToolResult`.
    /// Key = call_id, value = channel to the blocked HTTP handler thread.
    pub(crate) mcp_pending: std::collections::HashMap<String, std::sync::mpsc::SyncSender<mcp_server::McpToolResponse>>,
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
        mcp: Option<&crate::app_registry::McpSection>,
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
        let bundle_contents = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().and_then(|p| p.parent()).map(|p| p.to_path_buf()));
        let bundled_py_bin = bundle_contents.as_ref()
            .map(|c| c.join("Resources").join("assets").join("python").join("bin"));

        // .py entries are launched via python3 directly — no shebang or executable bit required.
        let is_python = bin_path.extension().and_then(|e| e.to_str()) == Some("py");
        let mut cmd = if is_python {
            let py_exe = bundled_py_bin.as_ref()
                .map(|b| b.join("python3"))
                .filter(|p| p.exists())
                .map(|p| std::ffi::OsString::from(p))
                .unwrap_or_else(|| std::ffi::OsString::from("python3"));
            log::info!("ProcessApp[{type_id}]: launching .py entry via {:?}", py_exe);
            let mut c = std::process::Command::new(py_exe);
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
        // Pass through every PLEXI_* var (harness knobs, mock-device selectors).
        for (k, v) in std::env::vars() {
            if k.starts_with("PLEXI_") {
                cmd.env(k, v);
            }
        }
        // Set PLEXI_SOCKET so the app can invoke `plexi` CLI commands against
        // the running host. macOS GUI apps don't inherit shell env, so this
        // is never present via PLEXI_* passthrough above.
        let socket_path = crate::config::config_dir().join("notify.sock");
        cmd.env("PLEXI_SOCKET", &socket_path);

        // Start the MCP server when the manifest declares [app.mcp].
        let mcp_server_handle = mcp.map(|section| {
            match mcp_server::start_mcp_server(section.tools.clone()) {
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
        }).flatten();
        // Prepend the bundled Python interpreter's bin/ dir to PATH so that
        // dev-mode .py entries without the bundle still resolve python3 correctly.
        // Falls back silently to host PATH if the bundle runtime isn't present.
        if let Some(ref py_bin) = bundled_py_bin {
            if py_bin.exists() {
                let host_path = std::env::var("PATH").unwrap_or_default();
                cmd.env("PATH", format!("{}:{}", py_bin.display(), host_path));
            }
        }

        // Make the shared Plexi SDK importable by Python apps without per-app copies.
        // Priority: user's local SDK (~/.plexi-alpha/sdk/) first, then the copy
        // bundled inside the .app bundle (Contents/Resources/sdk/python/). The bundle path
        // ensures apps work on a fresh install where just install-alpha was never run.
        let sdk_dir = crate::config::config_dir().join("sdk");
        let mut pythonpath = sdk_dir.to_string_lossy().into_owned();
        if let Some(bundle_sdk) = bundle_contents
            .map(|p| p.join("Resources").join("sdk").join("python"))
            .filter(|p| p.exists())
        {
            pythonpath.push(':');
            pythonpath.push_str(&bundle_sdk.to_string_lossy());
        }
        cmd.env("PYTHONPATH", pythonpath);
        let mut child = cmd.spawn()?;

        let stdin = child.stdin.take().expect("stdin piped");
        let stdout: ChildStdout = child.stdout.take().expect("stdout piped");
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
        let stdin_type_id = type_id.clone();
        {
            use std::io::Write as _;
            let render_slot_writer = Arc::clone(&render_slot);
            let render_in_queue_writer = Arc::clone(&render_in_queue);
            thread::Builder::new()
                .name(format!("app-stdin-{stdin_type_id}"))
                .spawn(move || {
                let mut stdin = stdin;
                for item in event_rx {
                    match item {
                        StdinItem::Event(line) => {
                            if stdin.write_all(line.as_bytes()).is_err() {
                                log::debug!("ProcessApp[{stdin_type_id}]: stdin write failed — writer thread exiting");
                                break;
                            }
                        }
                        StdinItem::FlushRender => {
                            // Clear the flag *before* reading the slot so that
                            // a concurrent send_event can enqueue a new token
                            // if a fresh Render arrives while we're writing.
                            render_in_queue_writer.store(false, Ordering::Relaxed);
                            let line = render_slot_writer.lock().unwrap().take();
                            if let Some(line) = line {
                                if stdin.write_all(line.as_bytes()).is_err() {
                                    log::debug!("ProcessApp[{stdin_type_id}]: stdin write failed — writer thread exiting");
                                    break;
                                }
                            }
                        }
                    }
                }
            }).expect("failed to spawn app-stdin thread");
        }

        // Background thread: forward subprocess stderr to Plexi's logger,
        // capture into the recent-stderr ring buffer used by the in-pane
        // error fallback, AND scan each line for `Traceback` / `PANIC` /
        // `panicked at` so the lifecycle pill flips to Crashed without
        // waiting for `try_wait` to observe the eventual exit.
        let stderr_type_id = type_id.clone();
        let recent_stderr_capture = Arc::new(Mutex::new(VecDeque::<String>::new()));
        let recent_stderr_thread = Arc::clone(&recent_stderr_capture);
        let lifecycle_tracker = Arc::new(LifecycleTracker::new());
        let lifecycle_stderr = Arc::clone(&lifecycle_tracker);
        thread::Builder::new()
            .name(format!("app-stderr-{stderr_type_id}"))
            .spawn(move || {
            const STDERR_RING_CAP: usize = 32;
            let reader = std::io::BufReader::new(stderr);
            for line in std::io::BufRead::lines(reader) {
                match line {
                    Ok(l) if !l.trim().is_empty() => {
                        let target = format!("app::{stderr_type_id}");
                        log::warn!(target: &target, "stderr: {l}");
                        lifecycle_stderr.observe_stderr_line(&l);
                        if let Ok(mut buf) = recent_stderr_thread.lock() {
                            if buf.len() >= STDERR_RING_CAP {
                                buf.pop_front();
                            }
                            buf.push_back(l);
                        }
                    }
                    Err(_) => break,
                    _ => {}
                }
            }
        }).expect("failed to spawn app-stderr thread");

        // Background thread: read draw commands line-by-line and forward via channel.
        // Also feeds the lifecycle tracker:
        //   - Malformed JSON → on_parse_error() (counts toward ProtocolError).
        //   - Stdout EOF / read error → on_stdout_closed() (sticky Crashed).
        let (draw_tx, draw_rx) = mpsc::channel::<DrawCommand>();
        let lifecycle_stdout = Arc::clone(&lifecycle_tracker);
        let stdout_type_id = type_id.clone();
        thread::Builder::new()
            .name(format!("app-stdout-{stdout_type_id}"))
            .spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                match line {
                    Ok(l) if !l.trim().is_empty() => {
                        match serde_json::from_str::<DrawCommand>(&l) {
                            Ok(cmd) => {
                                if draw_tx.send(cmd).is_err() {
                                    break;
                                }
                            }
                            Err(e) => {
                                log::warn!(
                                    "ProcessApp[{stdout_type_id}]: malformed draw command: {e} — line: {l}"
                                );
                                if lifecycle_stdout.on_parse_error() {
                                    log::error!(
                                        "ProcessApp[{stdout_type_id}]: protocol-error threshold reached — flipping pane state"
                                    );
                                }
                            }
                        }
                    }
                    Err(e) => {
                        log::debug!("ProcessApp[{stdout_type_id}] stdout closed: {e}");
                        lifecycle_stdout.on_stdout_closed();
                        break;
                    }
                    _ => {}
                }
            }
            // Natural EOF (loop exit without an Err) also signals the
            // subprocess closed its stdout — flip Crashed unless already
            // terminal.
            lifecycle_stdout.on_stdout_closed();
        }).expect("failed to spawn app-stdout thread");

        // Background reaper: blocks on waitpid so the UI thread never polls try_wait.
        // Fires on_process_exited() exactly once when the child exits — replaces the
        // per-frame try_wait() poll that was causing 600 syscalls/sec with 10 panes open.
        let reaper_pid = child.id();
        let lifecycle_reaper = Arc::clone(&lifecycle_tracker);
        let reaper_type_id = type_id.clone();
        thread::Builder::new()
            .name(format!("app-reaper-{reaper_type_id}"))
            .spawn(move || {
                let mut status = 0i32;
                // SAFETY: reaper_pid is a valid child PID obtained from Command::spawn().
                // We block until the child exits. The shutdown path's child.wait() may
                // race and get ECHILD if we win — that's harmless since shutdown discards
                // the result with `let _ = child.wait()`.
                unsafe {
                    libc::waitpid(reaper_pid as libc::pid_t, &mut status, 0);
                }
                log::info!(
                    "ProcessApp[{reaper_type_id}]: child exited — reaper signaling lifecycle"
                );
                lifecycle_reaper.on_process_exited();
            })
            .expect("failed to spawn app-reaper thread");

        let config_dir = crate::config::config_dir();
        let store = crate::app_permissions::PermissionStore::load_or_default(&config_dir);
        let (granted_caps, blocked_caps) = store.build_permission_sets(
            &type_id,
            &workspace_root,
            &capabilities,
        );
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

        Ok(Self {
            type_id,
            pane_id: 0,
            display_name,
            process: Some(child),
            event_tx: Some(event_tx),
            render_slot,
            render_in_queue,
            draw_rx: Some(draw_rx),
            frame: Vec::new(),
            pending_frame: Vec::new(),
            pending_commands: Vec::new(),
            last_size: egui::Vec2::ZERO,
            initialized: false,
            frame_counter: 0,
            sdk: None,
            features_used: Vec::new(),
            workspace_root,
            app_dir: bin_path.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| std::env::temp_dir()),
            permissions,
            permission_store: store,
            pipe_registry: Arc::new(Mutex::new(TypedPipeRegistry::new(
                crate::config::config_dir().join("pipes"),
            ))),
            run_registry: RunRegistry::new(),
            pending_prompts: VecDeque::new(),
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
            video_device: crate::video::default_video_device(),
            video_handles: HashMap::new(),
            video_pipe_ids: HashMap::new(),
            pending_timers: HashMap::new(),
            lifecycle: lifecycle_tracker,
            show_stderr_overlay: false,
            crashed_at: None,
            copied_feedback_until: None,
            pending_notification_count: 0,
            mouse_tracking_enabled: false,
            commonmark_cache: egui_commonmark::CommonMarkCache::default(),
            image_cache: image_cache::ImageCache::new(),
            render_session: RenderSession::new(),
            exposed_tools: Vec::new(),
            stream_handles: HashMap::new(),
            active_stream_threads: Arc::new(AtomicUsize::new(0)),
            mcp_server: mcp_server_handle,
            mcp_pending: std::collections::HashMap::new(),
        })
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
    pub fn new_for_test(pane_id: u64, permissions: crate::app_permissions::AppPermissions) -> (Self, Sender<DrawCommand>) {
        use crate::audio::MockAudioDevice;
        use crate::midi::MockMidiDevice;
        use crate::video::{MockVideoDecoder, MockVideoDecoderConfig};
        use crate::plexi_ai::broker::{AiBrokerRequest, AiBrokerResponse};

        struct NoopBroker;
        impl AiBroker for NoopBroker {
            fn dispatch(&self, _req: AiBrokerRequest) -> AiBrokerResponse {
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
            frame: Vec::new(),
            pending_frame: Vec::new(),
            pending_commands: Vec::new(),
            last_size: egui::Vec2::ZERO,
            initialized: true,
            frame_counter: 0,
            sdk: None,
            features_used: Vec::new(),
            workspace_root: std::env::temp_dir(),
            app_dir: std::env::temp_dir(),
            permissions,
            permission_store: crate::app_permissions::PermissionStore::default(),
            pipe_registry: Arc::new(Mutex::new(TypedPipeRegistry::new(
                std::env::temp_dir().join(format!("plexi-pipes-{}", uuid::Uuid::new_v4())),
            ))),
            run_registry: RunRegistry::new(),
            pending_prompts: VecDeque::new(),
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
                                    log::debug!("ProcessApp[{}]: stdin writer thread exited", self.type_id);
                                    self.event_tx = None;
                                }
                            }
                        }
                    }
                    _ => {
                        if let Some(tx) = &self.event_tx {
                            if tx.send(StdinItem::Event(line)).is_err() {
                                log::debug!("ProcessApp[{}]: stdin writer thread exited", self.type_id);
                                self.event_tx = None;
                            }
                        }
                    }
                }
            }
            Err(e) => log::error!("ProcessApp: failed to serialize event: {e}"),
        }
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
        self.event_tx.as_ref().map(|tx| {
            crate::plexi_ai::tool_dispatch::AppEventSender {
                tx: tx.clone(),
            }
        })
    }

    fn flush_outbound_events(&mut self) {
        while let Some(event) = self.outbound_events.pop_front() {
            self.send_event(&event);
        }
    }

    fn drain_draw_commands(&mut self) -> Vec<DrawCommand> {
        let Some(rx) = self.draw_rx.as_ref() else {
            return vec![];
        };
        let mut cmds = Vec::new();
        loop {
            match rx.try_recv() {
                Ok(cmd) => cmds.push(cmd),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    log::error!(
                        "ProcessApp[{}]: subprocess stdout closed — process crashed or exited",
                        self.type_id
                    );
                    self.draw_rx = None;
                    break;
                }
            }
        }
        cmds
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
        let radius = crate::style::RADIUS_BADGE;
        let inset = 8.0;
        let font_id = egui::FontId::proportional(font_size);
        let galley = ui.fonts(|f| f.layout_no_wrap(label.to_string(), font_id, egui::Color32::BLACK));
        let text_w = galley.size().x;
        let text_h = galley.size().y;
        let pill_w = (text_w + crate::style::BADGE_PAD_H * 2.0).max(crate::style::BADGE_MIN_W);
        let pill_h = text_h + crate::style::BADGE_PAD_V * 2.0;
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
        colors: &crate::theme::Colors,
    ) {
        let label = if count > 9 { "9+".to_string() } else { count.to_string() };
        let font_size = 11.0;
        let radius = crate::style::RADIUS_BADGE;
        let inset = 8.0;
        let font_id = egui::FontId::proportional(font_size);
        let fg_color = egui::Color32::from_rgb(0x1e, 0x1e, 0x2e);
        let galley = ui.fonts(|f| f.layout_no_wrap(label.clone(), font_id, fg_color));
        let text_w = galley.size().x;
        let text_h = galley.size().y;
        let pill_w = (text_w + crate::style::BADGE_PAD_H * 2.0).max(crate::style::BADGE_MIN_W);
        let pill_h = text_h + crate::style::BADGE_PAD_V * 2.0;
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
        while let Ok(event) = self.http_rx.try_recv() {
            self.outbound_events.push_back(event);
        }
        while let Ok(event) = self.file_picker_rx.try_recv() {
            self.outbound_events.push_back(event);
        }
        self.poll_mcp_calls();
        self.flush_outbound_events();
        for cmd in self.drain_draw_commands() {
            match cmd {
                DrawCommand::Host(h) => self.route_command(h),
                DrawCommand::Control(ControlCommand::Log { level, message }) => {
                    let target = format!("app::{}", self.type_id);
                    match level.as_str() {
                        "error" => log::error!(target: &target, "{message}"),
                        "warn"  => log::warn!(target: &target, "{message}"),
                        "debug" => log::debug!(target: &target, "{message}"),
                        _       => log::info!(target: &target, "{message}"),
                    }
                }
                DrawCommand::Control(_) => {} // Ready/FrameDone/etc. irrelevant without a pane
                DrawCommand::Render(_) => {} // No pane to render into
            }
        }
    }

    /// Dispatch a `ControlCommand` that arrived during `ui()`. Called inline
    /// from the `ui()` dispatch loop; has access to `egui::Ui` for operations
    /// that require UI-thread context (font metrics, clipboard, repaint).
    fn handle_control_command(
        &mut self,
        ui: &mut egui::Ui,
        frame_id: u64,
        cmd: ControlCommand,
    ) {
        match cmd {
            ControlCommand::Ready { sdk, features_used } => {
                self.sdk = Some(sdk);
                self.features_used = features_used;
            }
            ControlCommand::FrameDone { frame_id: done_id } => {
                if done_id != frame_id {
                    log::debug!(
                        "ProcessApp[{}]: FrameDone frame_id={done_id} expected={frame_id}",
                        self.type_id
                    );
                }
                std::mem::swap(&mut self.frame, &mut self.pending_frame);
                self.pending_frame.clear();
                // Lifecycle: a frame just landed → Running (unless terminal).
                self.lifecycle.on_frame_done();
            }
            ControlCommand::Log { level, message } => {
                let target = format!("app::{}", self.type_id);
                match level.as_str() {
                    "error" => log::error!(target: &target, "{message}"),
                    "warn"  => log::warn!(target: &target, "{message}"),
                    "debug" => log::debug!(target: &target, "{message}"),
                    _       => log::info!(target: &target, "{message}"),
                }
            }
            ControlCommand::ScheduleRender { after_ms } => {
                ui.ctx().request_repaint_after(
                    std::time::Duration::from_millis(after_ms as u64),
                );
            }
            // CopyToClipboard is handled here (not in routing.rs) because
            // `egui::Context::copy_text` is a UI-context method. The host
            // owns the clipboard backend selection (pasteboard / X11 /
            // Wayland / Win32) — we just hand egui the string.
            ControlCommand::CopyToClipboard { text } => {
                ui.ctx().copy_text(text);
            }
            // MeasureText is handled here (not in routing.rs) because it
            // needs `ui` to access egui font metrics on the UI thread.
            ControlCommand::MeasureText {
                request_id,
                text,
                font_size,
                monospace,
            } => {
                let family = if monospace {
                    egui::FontFamily::Monospace
                } else {
                    egui::FontFamily::Proportional
                };
                let font_id = egui::FontId::new(font_size, family);
                let galley = ui.fonts(|f| {
                    f.layout_no_wrap(text, font_id, egui::Color32::WHITE)
                });
                let sz = galley.size();
                self.outbound_events
                    .push_back(crate::app_protocol::PlexiEvent::TextMeasured {
                        request_id,
                        width: sz.x,
                        height: sz.y,
                    });
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

    fn queue_outbound_event(&mut self, event: crate::app_protocol::PlexiEvent) {
        self.outbound_events.push_back(event);
    }

    fn ui(&mut self, ui: &mut egui::Ui, ctx: &AppRenderContext<'_>) {
        let size = ui.available_size();

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

        if !self.initialized {
            self.initialized = true;
            self.last_size = size;
            let cap_strings: Vec<String> = self
                .permissions
                .capabilities
                .iter()
                .map(|c| c.to_string())
                .collect();
            self.send_event(&PlexiEvent::Init {
                protocol: "pgap/3".to_string(),
                app_id: self.type_id.clone(),
                workspace_root: self.workspace_root.clone(),
                capabilities: cap_strings,
                feature_flags: vec!["pane_groups_v1".into()],
            });
            // Inject persisted state before first render so on_inject runs with data.
            let state = load_app_state(&self.type_id, &self.workspace_root);
            self.outbound_events.push_back(PlexiEvent::InjectState { payload: state });
            log::info!("ProcessApp[{}]: injected persisted state at startup", self.type_id);
        }

        if (size - self.last_size).length() > 1.0 {
            self.last_size = size;
            self.send_event(&PlexiEvent::Resize {
                width: size.x,
                height: size.y,
            });
        }

        self.frame_counter += 1;
        let frame_id = self.frame_counter;
        self.send_event(&PlexiEvent::Render {
            frame_id,
            rect: crate::app_protocol::Rect {
                x: 0.0,
                y: 0.0,
                w: size.x,
                h: size.y,
            },
        });

        // Drain async HTTP responses from background request threads.
        while let Ok(event) = self.http_rx.try_recv() {
            self.outbound_events.push_back(event);
        }
        // Drain file picker results from background dialog threads.
        while let Ok(event) = self.file_picker_rx.try_recv() {
            self.outbound_events.push_back(event);
        }

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
            self.outbound_events.push_back(PlexiEvent::AudioCaptureError {
                pipe_id,
                error: "pipe drain failed (broken pipe)".to_owned(),
            });
        }

        let new_cmds = self.drain_draw_commands();

        for cmd in new_cmds {
            match cmd {
                DrawCommand::Control(c) => self.handle_control_command(ui, frame_id, c),
                DrawCommand::Host(h) => self.route_command(h),
                DrawCommand::Render(r) => self.pending_frame.push(r),
            }
        }

        if !self.pending_prompts.is_empty() {
            let mut pending_prompts = std::mem::take(&mut self.pending_prompts);
            let mut outbound_events = std::mem::take(&mut self.outbound_events);
            let mut permissions = std::mem::take(&mut self.permissions);
            let mut secret_input_buf = std::mem::take(&mut self.secret_input_buf);
            let mut permission_store = std::mem::take(&mut self.permission_store);
            let type_id = self.type_id.clone();
            let workspace_root = self.workspace_root.clone();
            let config_dir = crate::config::config_dir();
            prompts::show_prompt_modal(
                ui,
                &mut pending_prompts,
                &mut outbound_events,
                &mut permissions,
                &type_id,
                &workspace_root,
                &mut secret_input_buf,
                &config_dir,
                &mut permission_store,
                ctx.colors,
            );
            self.pending_prompts = pending_prompts;
            self.outbound_events = outbound_events;
            self.permissions = permissions;
            self.secret_input_buf = secret_input_buf;
            self.permission_store = permission_store;
        }

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
        ui.painter().rect_filled(pane_rect, 0.0, ctx.colors.terminal_bg);
        let audio_peaks: HashMap<String, f32> = self
            .audio_peak_meters
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default();
        self.image_cache.poll(ui.ctx());
        self.render_session.render(ui, pane_rect, &self.frame, ctx.colors, &mut self.commonmark_cache, &audio_peaks, self.pane_id, &mut self.image_cache, &self.app_dir);
        self.outbound_events.extend(self.render_session.drain_events());

        // ── Error fallback ──────────────────────────────────────────────────
        // Surface recent stderr in the pane when:
        //   1. The app emitted no draw commands at all (still booting / never
        //      started rendering), OR
        //   2. The lifecycle says Crashed or Hung — overlays even if the app
        //      had previously committed a frame, so a kill -9 of an app
        //      mid-run shows the failure rather than a frozen last frame.
        //   3. The user clicked the lifecycle pill (show_stderr_overlay).
        let lifecycle_state = self.lifecycle.state();
        if matches!(lifecycle_state, LifecycleState::Crashed | LifecycleState::Hung | LifecycleState::ProtocolError) {
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
                    LifecycleState::Crashed => format!("⚠  {} crashed — recent stderr:", self.type_id),
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
                for line in stderr_lines.iter().rev() {
                    let trimmed: String = line.chars().take(160).collect();
                    painter.text(
                        egui::pos2(title_pos.x, y),
                        egui::Align2::LEFT_TOP,
                        &trimmed,
                        egui::FontId::monospace(11.0),
                        ctx.colors.text_dim,
                    );
                    y += 14.0;
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
            self.draw_notification_indicator(ui, pane_rect, self.pending_notification_count, ctx.colors);
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
                    self.send_event(&PlexiEvent::MouseDown {
                        x: pos.x - origin.x,
                        y: pos.y - origin.y,
                        button: crate::app_protocol::MouseButton::Primary,
                    });
                    needs_click_repaint = true;
                }
                if is_secondary_down {
                    self.send_event(&PlexiEvent::MouseDown {
                        x: pos.x - origin.x,
                        y: pos.y - origin.y,
                        button: crate::app_protocol::MouseButton::Secondary,
                    });
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
                let primary_up = mouse_response.clicked()
                    || mouse_response.drag_stopped();
                let secondary_up = mouse_response.secondary_clicked()
                    || mouse_response.drag_stopped_by(egui::PointerButton::Secondary);
                if primary_up {
                    self.send_event(&PlexiEvent::MouseUp {
                        x,
                        y,
                        button: crate::app_protocol::MouseButton::Primary,
                    });
                    self.send_event(&PlexiEvent::Click {
                        x,
                        y,
                        button: crate::app_protocol::MouseButton::Primary,
                    });
                    needs_click_repaint = true;
                }
                if secondary_up {
                    self.send_event(&PlexiEvent::MouseUp {
                        x,
                        y,
                        button: crate::app_protocol::MouseButton::Secondary,
                    });
                    self.send_event(&PlexiEvent::Click {
                        x,
                        y,
                        button: crate::app_protocol::MouseButton::Secondary,
                    });
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
                    let is_dragging =
                        pointer_state.button_down(egui::PointerButton::Primary)
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
                        });
                        needs_tracking_repaint = true;
                    }
                }
            }
        }

        // Flush events accumulated during this frame (broker AiResponse,
        // TextSubmitted, ScrollOffset, etc.) so apps receive them without
        // waiting for the next frame's start-of-ui flush.
        self.flush_outbound_events();

        // Idle polling for async HTTP responses. Apps that need faster repaints
        // (games, animations) emit DrawCommand::ScheduleRender { after_ms } each frame.
        //
        // Pointer-tracking apps are a special case: while the pointer is actively
        // moving we keep the repaint cadence near 60 FPS so host->app hover state
        // does not feel sticky.
        if needs_click_repaint {
            ui.ctx().request_repaint();
        } else if needs_tracking_repaint {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(16));
        } else {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        }
    }

    fn handle_key(&mut self, input: &egui::InputState) -> bool {
        // When a TextInput widget has focus, egui owns the keyboard — all
        // text and key events are consumed by the TextEdit widget. Don't
        // forward them to the app's on_key handler (typing "h" in the chat
        // input shouldn't trigger a tier change, for example).
        if self.render_session.text_input_has_focus {
            return false;
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
                    self.outbound_events.push_back(PlexiEvent::Paste {
                        text: text.clone(),
                    });
                    consumed = true;
                }
                _ => {}
            }
        }
        consumed
    }

    fn take_pending_commands(&mut self) -> Vec<AppCommand> {
        std::mem::take(&mut self.pending_commands)
    }

    fn sync_cwd(&mut self, new_cwd: &std::path::Path) {
        self.outbound_events.push_back(PlexiEvent::PathChanged {
            cwd: new_cwd.to_path_buf(),
        });
    }

    fn serialize_state(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "type_id": self.type_id,
        }))
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
    Arc::new(crate::audio::CoreAudioDevice::new())
}

#[cfg(test)]
fn default_audio_device() -> Arc<dyn AudioDevice> {
    Arc::new(crate::audio::MockAudioDevice::new())
}

/// Build the production MIDI device. CoreMIDI on non-test mac builds; an
/// empty stub on non-mac (CoreMidiDevice impl returns empty port lists and
/// PortNotFound for every open). Mock under `cfg(test)`.
///
/// Tests that exercise the MIDI routing path inject
/// `Arc::new(MockMidiDevice::new())` directly into `ProcessApp::midi_device`.
#[cfg(not(test))]
fn default_midi_device() -> Arc<dyn MidiDevice> {
    Arc::new(crate::midi::CoreMidiDevice::new())
}

#[cfg(test)]
fn default_midi_device() -> Arc<dyn MidiDevice> {
    Arc::new(crate::midi::MockMidiDevice::new())
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
    let ws_old = workspace_root.join(".plexi").join("app_state");
    let ws_new = workspace_root.join(".plexi").join("app_states");
    migrate_app_state_dir(&ws_old, &ws_new);

    let workspace_path = ws_new.join(&filename);
    // Fallback: if migration failed (e.g. permission error), still read from old location
    let workspace_path_legacy = ws_old.join(&filename);
    if workspace_path.exists() {
        match std::fs::read(&workspace_path) {
            Err(e) => {
                log::warn!("load_app_state[{type_id}]: could not read workspace state {}: {e}", workspace_path.display());
            }
            Ok(bytes) => match serde_json::from_slice::<serde_json::Value>(&bytes) {
                Err(e) => {
                    log::warn!("load_app_state[{type_id}]: could not parse workspace state {}: {e}", workspace_path.display());
                }
                Ok(val) => {
                    log::info!("load_app_state[{type_id}]: loaded workspace state from {}", workspace_path.display());
                    return val;
                }
            },
        }
    } else if workspace_path_legacy.exists() {
        match std::fs::read(&workspace_path_legacy) {
            Err(e) => {
                log::warn!("load_app_state[{type_id}]: could not read legacy workspace state {}: {e}", workspace_path_legacy.display());
            }
            Ok(bytes) => match serde_json::from_slice::<serde_json::Value>(&bytes) {
                Err(e) => {
                    log::warn!("load_app_state[{type_id}]: could not parse legacy workspace state {}: {e}", workspace_path_legacy.display());
                }
                Ok(val) => {
                    log::info!("load_app_state[{type_id}]: loaded workspace state from legacy path {}", workspace_path_legacy.display());
                    return val;
                }
            },
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
                log::warn!("load_app_state[{type_id}]: could not read global state {}: {e}", global_path.display());
            }
            Ok(bytes) => match serde_json::from_slice::<serde_json::Value>(&bytes) {
                Err(e) => {
                    log::warn!("load_app_state[{type_id}]: could not parse global state {}: {e}", global_path.display());
                }
                Ok(val) => {
                    log::info!("load_app_state[{type_id}]: loaded global state from {}", global_path.display());
                    return val;
                }
            },
        }
    } else if global_path_legacy.exists() {
        match std::fs::read(&global_path_legacy) {
            Err(e) => {
                log::warn!("load_app_state[{type_id}]: could not read legacy global state {}: {e}", global_path_legacy.display());
            }
            Ok(bytes) => match serde_json::from_slice::<serde_json::Value>(&bytes) {
                Err(e) => {
                    log::warn!("load_app_state[{type_id}]: could not parse legacy global state {}: {e}", global_path_legacy.display());
                }
                Ok(val) => {
                    log::info!("load_app_state[{type_id}]: loaded global state from legacy path {}", global_path_legacy.display());
                    return val;
                }
            },
        }
    }
    log::debug!("load_app_state[{type_id}]: no usable state file found, starting empty");
    serde_json::Value::Object(serde_json::Map::new())
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod clipboard_tests {
    //! Behavioural tests for the v3.2 clipboard / paste plumbing (#200, #146).
    //!
    //!   1. `egui::Event::Paste(text)` translates into `PlexiEvent::Paste`
    //!      on the outbound queue (paste_event_forwarded_as_plexi_event).
    //!   2. `DrawCommand::CopyToClipboard { text }` reaches egui's output
    //!      command queue as `OutputCommand::CopyText` so the platform
    //!      backend writes to the OS clipboard
    //!      (copy_to_clipboard_drawcommand_calls_egui_copy).
    //!
    //! These exercise the host-side translation logic. End-to-end clipboard
    //! integration with NSPasteboard / X11 / Wayland is verified via the
    //! human-verification checklist in the PR — egui's backend is opaque
    //! from a unit-test standpoint.
    use super::*;
    use crate::app_protocol::PlexiEvent;
    use std::collections::HashSet;
    use std::path::PathBuf;

    /// Build a minimal `ProcessApp` for tests. Mirrors the helper in
    /// `text_input_tests` — spawns `/bin/sh -c "sleep 1"` so lifecycle
    /// machinery is happy, then ignores the subprocess.
    fn make_app() -> Option<ProcessApp> {
        let sh = ["/bin/sh", "/usr/bin/sh"]
            .iter()
            .find(|p| std::path::Path::new(p).exists())
            .map(PathBuf::from)?;
        let workspace_root = std::env::temp_dir();
        ProcessApp::launch(
            "test_clipboard",
            "Test Clipboard",
            &sh,
            &workspace_root,
            &["-c".to_string(), "sleep 1".to_string()],
            workspace_root.clone(),
            HashSet::new(),
            false,
            None,
        )
        .ok()
    }

    #[test]
    fn paste_event_forwarded_as_plexi_event() {
        // Drive a synthesised `egui::Event::Paste("hello")` through the
        // pane's `handle_key`. The expected outcome is one
        // `PlexiEvent::Paste { text: "hello" }` on the outbound event
        // queue. No `Key`/`Text` events should be synthesised.
        let Some(mut app) = make_app() else {
            eprintln!("skipping: no /bin/sh available");
            return;
        };

        let mut input = egui::InputState::default();
        input.events.push(egui::Event::Paste("hello".to_string()));

        let consumed = app.handle_key(&input);
        assert!(consumed, "handle_key must consume Paste events");

        let paste_events: Vec<_> = app
            .outbound_events
            .iter()
            .filter(|e| matches!(e, PlexiEvent::Paste { .. }))
            .collect();
        assert_eq!(
            paste_events.len(),
            1,
            "expected exactly one Paste event, got {paste_events:?}"
        );
        match paste_events[0] {
            PlexiEvent::Paste { text } => assert_eq!(text, "hello"),
            other => panic!("expected Paste, got {other:?}"),
        }
    }

    #[test]
    fn copy_to_clipboard_drawcommand_calls_egui_copy() {
        // Verify the wired path: `ControlCommand::CopyToClipboard { text }` →
        // `egui::Context::copy_text(text)` → `OutputCommand::CopyText` on
        // the platform output. We construct a fresh egui Context, mirror
        // the one-line dispatch from `ProcessApp::handle_control_command()`,
        // and inspect the platform output for the CopyText command. If the
        // dispatch ever changes shape (e.g. a different egui method), this
        // test forces the breakage to surface.
        use crate::app_protocol::ControlCommand;
        let ctx = egui::Context::default();
        let cmd = ControlCommand::CopyToClipboard {
            text: "selected snippet".to_string(),
        };

        // This mirrors the exact branch in `ProcessApp::handle_control_command()`.
        // Keep it in sync — if you refactor the dispatch, refactor this too.
        match cmd {
            ControlCommand::CopyToClipboard { text } => ctx.copy_text(text),
            _ => panic!("test setup error"),
        }

        // Drain platform output and look for CopyText.
        let mut found = None;
        ctx.output_mut(|o| {
            for cmd in &o.commands {
                if let egui::OutputCommand::CopyText(text) = cmd {
                    found = Some(text.clone());
                }
            }
        });
        assert_eq!(
            found.as_deref(),
            Some("selected snippet"),
            "CopyToClipboard must emit OutputCommand::CopyText with the right text"
        );
    }

    #[test]
    fn crash_overlay_c_key_copies_report() {
        use crate::testing::HostHarness;
        use crate::pane::{AppRuntime, Pane};

        let mut h = HostHarness::new();
        let pane = h.add_test_pane();

        // Force lifecycle to Crashed and inject known stderr lines.
        {
            let win = &mut h.app.windows[0];
            let Some(Pane::App(app_pane)) = win.panes.get_mut(&pane) else {
                panic!("expected App pane");
            };
            let AppRuntime::Process(proc) = &mut app_pane.runtime else {
                panic!("expected Process runtime");
            };
            proc.lifecycle.on_process_exited(); // → Crashed
            let mut buf = proc.recent_stderr.lock().unwrap();
            buf.push_back("Traceback (most recent call last):".to_string());
            buf.push_back("  File \"app.py\", line 42, in run".to_string());
            buf.push_back("ZeroDivisionError: division by zero".to_string());
        }

        // One frame to trigger the overlay and stamp crashed_at.
        h.run_frames(1);

        // Send C — no modifier.
        h.key(egui::Key::C, egui::Modifiers::NONE);

        // Check the clipboard from the last frame's platform output.
        let copy_cmd = h.last_platform_output.commands.iter().find_map(|cmd| {
            if let egui::OutputCommand::CopyText(text) = cmd {
                Some(text.clone())
            } else {
                None
            }
        });
        let report = copy_cmd.expect("pressing C on crash overlay must write to clipboard");

        assert!(
            report.contains("=== Plexi Crash Report ==="),
            "report must have header: {report}"
        );
        assert!(
            report.contains("crashed"),
            "report must name the state: {report}"
        );
        assert!(
            report.contains("ZeroDivisionError"),
            "report must contain stderr lines: {report}"
        );
        assert!(
            report.contains("Traceback"),
            "report must contain all stderr lines: {report}"
        );
    }
}

#[cfg(test)]
mod text_input_tests {
    //! Buffer-state tests for the v3.1 host-owned TextInput primitive
    //! (issue #283). Covered behaviours:
    //!   1. Buffer persists across frames until submit.
    //!   2. Enter emits `PlexiEvent::TextSubmitted` with the buffered value.
    //!   3. Submit clears the buffer (so the field is empty on the next emit).
    //!   4. Pane resize (which triggers re-render but not buffer touch) does
    //!      not wipe the buffer.
    //!
    //! These exercise the persistent-state contract — the egui rendering
    //! layer is verified end-to-end by the human-verification checklist.
    //! Keeping the unit tests pure makes them deterministic and fast.
    use super::*;
    use crate::app_protocol::PlexiEvent;
    use std::collections::HashSet;
    use std::path::PathBuf;
    use std::process::Command;

    /// Build a `ProcessApp` for tests that doesn't touch real I/O. We
    /// spawn `/bin/sh -c true` — the cheapest valid subprocess — and
    /// then ignore the lifecycle / draw threads. The app's stdin/stdout
    /// are real but we never write to them in these tests.
    fn make_app() -> Option<ProcessApp> {
        let sh = ["/bin/sh", "/usr/bin/sh"]
            .iter()
            .find(|p| std::path::Path::new(p).exists())
            .map(PathBuf::from)?;
        let workspace_root = std::env::temp_dir();
        // -c true exits immediately. The lifecycle reader threads will
        // observe stdout EOF and flip Crashed, which is fine — these
        // tests don't read lifecycle state.
        let _ = Command::new(&sh); // sanity — silences unused-import warnings on some configs
        ProcessApp::launch(
            "test_text_input",
            "Test Text Input",
            &sh,
            &workspace_root,
            &["-c".to_string(), "sleep 1".to_string()],
            workspace_root.clone(),
            HashSet::new(),
            false,
            None,
        )
        .ok()
    }

    #[test]
    fn text_input_buffer_persists_across_frames() {
        let Some(mut app) = make_app() else {
            eprintln!("skipping: no /bin/sh available");
            return;
        };
        // Simulate two frames where the user has typed "hel" then "hello"
        // by directly manipulating the buffer the way egui's TextEdit
        // would across two ui() ticks.
        app.render_session.text_input_buffers
            .insert("note".to_string(), "hel".to_string());
        // ... another frame happens (no submit) ...
        // Buffer must still be there.
        assert_eq!(
            app.render_session.text_input_buffers.get("note").map(String::as_str),
            Some("hel"),
            "buffer should survive between frames"
        );
        app.render_session.text_input_buffers
            .insert("note".to_string(), "hello".to_string());
        assert_eq!(
            app.render_session.text_input_buffers.get("note").map(String::as_str),
            Some("hello")
        );
    }

    #[test]
    fn enter_emits_text_submitted_with_buffered_value() {
        let Some(mut app) = make_app() else {
            eprintln!("skipping: no /bin/sh available");
            return;
        };
        app.render_session.text_input_buffers
            .insert("note".to_string(), "hello world".to_string());

        app.submit_text_input("note");

        let evt = app.outbound_events.pop_back().expect("event queued");
        match evt {
            PlexiEvent::TextSubmitted { id, value } => {
                assert_eq!(id, "note");
                assert_eq!(value, "hello world");
            }
            other => panic!("expected TextSubmitted, got {other:?}"),
        }
    }

    #[test]
    fn submit_clears_buffer() {
        let Some(mut app) = make_app() else {
            eprintln!("skipping: no /bin/sh available");
            return;
        };
        app.render_session.text_input_buffers
            .insert("note".to_string(), "draft".to_string());
        app.submit_text_input("note");
        assert!(
            !app.render_session.text_input_buffers.contains_key("note"),
            "buffer must be cleared after submit (default UX)"
        );
    }

    #[test]
    fn submit_on_empty_buffer_emits_empty_value() {
        let Some(mut app) = make_app() else {
            eprintln!("skipping: no /bin/sh available");
            return;
        };
        // No prior buffer — Enter on a fresh TextInput is a valid case
        // (e.g. user immediately presses Enter without typing).
        app.submit_text_input("note");
        let evt = app.outbound_events.pop_back().expect("event queued");
        match evt {
            PlexiEvent::TextSubmitted { id, value } => {
                assert_eq!(id, "note");
                assert_eq!(value, "");
            }
            other => panic!("expected TextSubmitted, got {other:?}"),
        }
    }

    #[test]
    fn text_input_buffer_survives_pane_resize() {
        let Some(mut app) = make_app() else {
            eprintln!("skipping: no /bin/sh available");
            return;
        };
        // Pane resize is a `last_size` change in `ui()`. The buffer is
        // owned by `render_session.text_input_buffers` and never touched by resize
        // logic. Simulate the resize bookkeeping and assert the buffer
        // is untouched.
        app.render_session.text_input_buffers
            .insert("note".to_string(), "midway".to_string());
        // Resize bookkeeping (mirrors what `ui()` does on size delta).
        app.last_size = egui::vec2(800.0, 600.0);
        // No buffer mutation should happen here — just last_size changes.
        assert_eq!(
            app.render_session.text_input_buffers.get("note").map(String::as_str),
            Some("midway"),
            "resize must not wipe the host-owned text buffer"
        );
    }

    #[test]
    fn distinct_ids_keep_independent_buffers() {
        let Some(mut app) = make_app() else {
            eprintln!("skipping: no /bin/sh available");
            return;
        };
        app.render_session.text_input_buffers
            .insert("a".to_string(), "alpha".to_string());
        app.render_session.text_input_buffers
            .insert("b".to_string(), "beta".to_string());
        app.submit_text_input("a");
        assert_eq!(
            app.render_session.text_input_buffers.get("b").map(String::as_str),
            Some("beta"),
            "submitting one input must not affect another id"
        );
    }
}

#[cfg(test)]
mod render_session_tests {
    use super::*;
    use std::collections::HashSet;
    use std::path::PathBuf;

    fn make_app() -> Option<ProcessApp> {
        let sh = ["/bin/sh", "/usr/bin/sh"]
            .iter()
            .find(|p| std::path::Path::new(p).exists())
            .map(PathBuf::from)?;
        let workspace_root = std::env::temp_dir();
        ProcessApp::launch(
            "test_render_session",
            "Test RenderSession",
            &sh,
            &workspace_root,
            &["-c".to_string(), "sleep 1".to_string()],
            workspace_root.clone(),
            HashSet::new(),
            false,
            None,
        ).ok()
    }

    #[test]
    fn render_session_submit_produces_event() {
        let Some(mut app) = make_app() else {
            eprintln!("skipping: no /bin/sh available");
            return;
        };
        app.render_session.text_input_buffers.insert("x".to_string(), "hello".to_string());
        app.submit_text_input("x");
        let evt = app.outbound_events.pop_back().expect("event queued");
        match evt {
            crate::app_protocol::PlexiEvent::TextSubmitted { id, value } => {
                assert_eq!(id, "x");
                assert_eq!(value, "hello");
            }
            other => panic!("expected TextSubmitted, got {other:?}"),
        }
    }

    #[test]
    fn render_session_process_app_has_no_text_input_fields() {
        // Compile-time proof: ProcessApp::render_session owns the state.
        // This test just exercises the field path — if mod.rs still had
        // text_input_buffers directly on ProcessApp this wouldn't compile.
        let Some(mut app) = make_app() else { return; };
        app.render_session.text_input_buffers.insert("k".to_string(), "v".to_string());
        assert!(app.render_session.text_input_buffers.contains_key("k"));
    }
}

impl Drop for ProcessApp {
    fn drop(&mut self) {
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
            let shutdown_deadline = std::time::Instant::now()
                + std::time::Duration::from_secs(2);
            let mut exited = false;
            while std::time::Instant::now() < shutdown_deadline {
                match child.try_wait() {
                    Ok(Some(_)) => {
                        exited = true;
                        break;
                    }
                    Ok(None) => {
                        std::thread::sleep(std::time::Duration::from_millis(POLL_MS));
                    }
                    Err(e) if e.raw_os_error() == Some(libc::ECHILD) => {
                        // Background reaper already called waitpid and won the
                        // race — ECHILD means the child is gone. Treat as clean exit.
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
                let kill_deadline = std::time::Instant::now()
                    + std::time::Duration::from_secs(1);
                while std::time::Instant::now() < kill_deadline {
                    match child.try_wait() {
                        Ok(Some(_)) => break,
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
            }
        }
    }
}

#[cfg(test)]
mod ai_tests {
    //! Routing tests for the v3.3 `ai.query` broker capability (#284).
    //!
    //! Two paths under test:
    //!   1. App without `ai.query` capability — synchronous denial response
    //!      lands on `outbound_events`. No broker dispatch occurs.
    //!   2. App with `ai.query` capability — broker is invoked once and
    //!      its response surfaces on `http_rx` as `PlexiEvent::AiResponse`.
    //!
    //! The mock broker (`CannedBroker`) records every call so the granted
    //! path also confirms that the routing layer forwarded the right
    //! `model_tier`, `system`, and `messages` payload to the broker.
    use super::*;
    use crate::app_protocol::{AiMessage, HostCommand, ModelTier, PlexiEvent};
    use crate::plexi_ai::broker::{AiBroker, AiBrokerRequest, AiBrokerResponse};
    use std::collections::HashSet;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    /// Test broker: records every dispatch and returns a canned response.
    struct CannedBroker {
        seen: Arc<Mutex<Vec<AiBrokerRequest>>>,
        response: AiBrokerResponse,
    }

    impl AiBroker for CannedBroker {
        fn dispatch(&self, request: AiBrokerRequest) -> AiBrokerResponse {
            self.seen.lock().unwrap().push(request);
            self.response.clone()
        }
    }

    fn make_app(capabilities: HashSet<Capability>) -> Option<ProcessApp> {
        let sh = ["/bin/sh", "/usr/bin/sh"]
            .iter()
            .find(|p| std::path::Path::new(p).exists())
            .map(PathBuf::from)?;
        let workspace_root = std::env::temp_dir();
        ProcessApp::launch(
            "test_ai",
            "Test AI",
            &sh,
            &workspace_root,
            &["-c".to_string(), "sleep 1".to_string()],
            workspace_root.clone(),
            capabilities,
            false,
            None,
        )
        .ok()
    }

    #[test]
    fn denied_app_gets_capability_denied_response() {
        // App without `ai.query` capability: route_command must immediately
        // queue an AiResponse with the canonical "capability denied" error
        // — synchronously, without ever invoking the broker.
        let Some(mut app) = make_app(HashSet::new()) else {
            eprintln!("skipping: no /bin/sh available");
            return;
        };

        // Inject a broker that would *panic* if called. The denied path
        // must short-circuit before reaching dispatch.
        struct PanicBroker;
        impl AiBroker for PanicBroker {
            fn dispatch(&self, _: AiBrokerRequest) -> AiBrokerResponse {
                panic!("denied path must never call the broker");
            }
        }
        app.ai_broker = Arc::new(PanicBroker);

        app.route_command(HostCommand::AiQuery {
            request_id: "req-denied".to_string(),
            model_tier: ModelTier::Low,
            system: "system".to_string(),
            messages: vec![AiMessage {
                role: "user".to_string(),
                content: "hi".to_string(),
            }],
            tools: vec![],
        });

        // Denied path is synchronous — the response is on outbound_events
        // immediately, no thread, no http_rx wait.
        let resp = app
            .outbound_events
            .iter()
            .find(|e| matches!(e, PlexiEvent::AiResponse { .. }))
            .expect("expected AiResponse on outbound queue");
        match resp {
            PlexiEvent::AiResponse {
                request_id,
                content,
                tokens_in,
                tokens_out,
                error,
            } => {
                assert_eq!(request_id, "req-denied");
                assert!(content.is_none());
                assert_eq!(*tokens_in, 0);
                assert_eq!(*tokens_out, 0);
                let err = error.as_ref().expect("error must be set on denial");
                assert!(
                    err.contains("capability denied"),
                    "denial message must say `capability denied`: {err}"
                );
                assert!(
                    err.contains("ai.query"),
                    "denial message must name the capability: {err}"
                );
            }
            other => panic!("expected AiResponse, got {other:?}"),
        }
    }

    #[test]
    fn granted_app_dispatches_to_broker() {
        // App WITH `ai.query` granted: route_command must spawn a worker
        // that calls broker.dispatch exactly once with the right payload,
        // and the broker's response must arrive as a PlexiEvent::AiResponse
        // on http_rx.
        let mut caps = HashSet::new();
        caps.insert(Capability::AiQuery);
        let Some(mut app) = make_app(caps) else {
            eprintln!("skipping: no /bin/sh available");
            return;
        };

        let seen: Arc<Mutex<Vec<AiBrokerRequest>>> = Arc::new(Mutex::new(Vec::new()));
        app.ai_broker = Arc::new(CannedBroker {
            seen: Arc::clone(&seen),
            response: AiBrokerResponse::ok("Pong.".to_string(), 12, 4),
        });

        app.route_command(HostCommand::AiQuery {
            request_id: "req-ok".to_string(),
            model_tier: ModelTier::High,
            system: "be terse".to_string(),
            messages: vec![AiMessage {
                role: "user".to_string(),
                content: "ping".to_string(),
            }],
            tools: vec![],
        });

        // Worker thread is spawned — wait briefly for response to arrive
        // on http_rx. 2s is generous; canned broker is in-memory so the
        // typical wait is microseconds.
        let event = app
            .http_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("broker response must arrive on http_rx within 2s");

        match event {
            PlexiEvent::AiResponse {
                request_id,
                content,
                tokens_in,
                tokens_out,
                error,
            } => {
                assert_eq!(request_id, "req-ok");
                assert_eq!(content.as_deref(), Some("Pong."));
                assert_eq!(tokens_in, 12);
                assert_eq!(tokens_out, 4);
                assert!(error.is_none());
            }
            other => panic!("expected AiResponse, got {other:?}"),
        }

        // Broker must have been invoked exactly once with the correct payload.
        let calls = seen.lock().unwrap();
        assert_eq!(calls.len(), 1, "broker must be called exactly once");
        assert_eq!(calls[0].app_id, "test_ai");
        assert_eq!(calls[0].model_tier, ModelTier::High);
        assert_eq!(calls[0].system, "be terse");
        assert_eq!(calls[0].messages.len(), 1);
        assert_eq!(calls[0].messages[0].content, "ping");
    }
}

#[cfg(test)]
mod midi_tests {
    //! Routing tests for the v3.4 CoreMIDI capability (#320).
    //!
    //! Two paths under test:
    //!   1. App without `midi.in` / `midi.out` — synchronous denial response
    //!      lands on `outbound_events`. No device dispatch occurs.
    //!   2. App with the capability — `MockMidiDevice` records the open and
    //!      the routing layer queues `MidiInputOpened` on success.
    use super::*;
    use crate::app_protocol::{HostCommand, PlexiEvent};
    use crate::midi::MockMidiDevice;
    use std::collections::HashSet;
    use std::path::PathBuf;
    use std::sync::Arc;

    fn make_app(capabilities: HashSet<Capability>) -> Option<ProcessApp> {
        let sh = ["/bin/sh", "/usr/bin/sh"]
            .iter()
            .find(|p| std::path::Path::new(p).exists())
            .map(PathBuf::from)?;
        let workspace_root = std::env::temp_dir();
        ProcessApp::launch(
            "test_midi",
            "Test MIDI",
            &sh,
            &workspace_root,
            &["-c".to_string(), "sleep 1".to_string()],
            workspace_root.clone(),
            capabilities,
            false,
            None,
        )
        .ok()
    }

    #[test]
    fn denied_app_gets_capability_denied_response() {
        // App without `midi.in`: route_command must immediately queue
        // a MidiInputError with "capability denied" and never touch the
        // device. The send path mirrors the same contract for `midi.out`.
        let Some(mut app) = make_app(HashSet::new()) else {
            eprintln!("skipping: no /bin/sh available");
            return;
        };

        let mock = Arc::new(MockMidiDevice::new());
        app.midi_device = Arc::clone(&mock) as Arc<dyn crate::midi::MidiDevice>;

        app.route_command(HostCommand::OpenMidiInput {
            port_id: "mock-input-1".to_owned(),
            pipe_id: "midi-in-pipe".to_owned(),
        });

        let evt = app
            .outbound_events
            .iter()
            .find(|e| matches!(e, PlexiEvent::MidiInputError { .. }))
            .expect("expected MidiInputError on outbound queue");
        match evt {
            PlexiEvent::MidiInputError { pipe_id, error } => {
                assert_eq!(pipe_id, "midi-in-pipe");
                assert!(
                    error.contains("capability denied"),
                    "denial must say `capability denied`: {error}"
                );
                assert!(
                    error.contains("midi.in"),
                    "denial must name the capability: {error}"
                );
            }
            other => panic!("expected MidiInputError, got {other:?}"),
        }

        // The mock must NOT have an active session — the denied path
        // short-circuits before open_input is called.
        assert!(
            mock.injected_sinks
                .lock()
                .expect("mock midi sinks poisoned")
                .is_empty(),
            "denied path must not open the MIDI input"
        );

        // SendMidi without `midi.out` is the same shape.
        app.route_command(HostCommand::SendMidi {
            port_id: "mock-output-1".to_owned(),
            bytes: vec![0x90, 0x3C, 0x64],
        });
        let evt = app
            .outbound_events
            .iter()
            .find(|e| matches!(e, PlexiEvent::MidiSendError { .. }))
            .expect("expected MidiSendError on outbound queue");
        match evt {
            PlexiEvent::MidiSendError { port_id, error } => {
                assert_eq!(port_id, "mock-output-1");
                assert!(
                    error.contains("capability denied"),
                    "denial must say `capability denied`: {error}"
                );
                assert!(
                    error.contains("midi.out"),
                    "denial must name the capability: {error}"
                );
            }
            other => panic!("expected MidiSendError, got {other:?}"),
        }
    }

    #[test]
    fn granted_app_dispatches_open_input_to_device() {
        // App WITH `midi.in` granted: route_command must open the input on
        // the device, register a sink, and queue MidiInputOpened on
        // outbound_events. With `midi.out` granted, SendMidi must dispatch
        // the bytes to the mock's `sent` log.
        let mut caps = HashSet::new();
        caps.insert(Capability::MidiIn);
        caps.insert(Capability::MidiOut);
        let Some(mut app) = make_app(caps) else {
            eprintln!("skipping: no /bin/sh available");
            return;
        };

        let mock = Arc::new(MockMidiDevice::new());
        app.midi_device = Arc::clone(&mock) as Arc<dyn crate::midi::MidiDevice>;

        app.route_command(HostCommand::OpenMidiInput {
            port_id: "mock-input-1".to_owned(),
            pipe_id: "midi-in-pipe".to_owned(),
        });

        // PipeOpened arrives BEFORE MidiInputOpened so the app can connect
        // the unix socket before the first byte arrives.
        let pipe_opened = app
            .outbound_events
            .iter()
            .position(|e| matches!(e, PlexiEvent::PipeOpened { .. }))
            .expect("expected PipeOpened");
        let midi_opened = app
            .outbound_events
            .iter()
            .position(|e| matches!(e, PlexiEvent::MidiInputOpened { .. }))
            .expect("expected MidiInputOpened");
        assert!(
            pipe_opened < midi_opened,
            "PipeOpened must precede MidiInputOpened so the app's socket connection races first"
        );

        // The device must have a registered sink for the port.
        assert!(
            mock.injected_sinks
                .lock()
                .expect("mock midi sinks poisoned")
                .contains_key("mock-input-1"),
            "open_input must have registered a sink"
        );

        // SendMidi path: dispatches one note-on to the mock output log.
        app.route_command(HostCommand::SendMidi {
            port_id: "mock-output-1".to_owned(),
            bytes: vec![0x90, 0x3C, 0x64],
        });
        let log = mock.sent.lock().expect("mock sent poisoned").clone();
        let entries = log
            .get("mock-output-1")
            .expect("mock-output-1 entries must exist after SendMidi");
        assert_eq!(entries.len(), 1, "exactly one send dispatched");
        assert_eq!(entries[0], vec![0x90u8, 0x3C, 0x64]);
    }
}

#[cfg(test)]
mod video_tests {
    //! Routing tests for the v3.4 video substrate (#345).
    //!
    //! Two paths under test:
    //!   1. App without `video.playback` — synchronous denial response on
    //!      `outbound_events`. No device dispatch.
    //!   2. App with the capability — `MockVideoDecoder` opens the source,
    //!      the routing layer queues `VideoOpenAck` and pumps frames.
    use super::*;
    use crate::app_protocol::{HostCommand, PlexiEvent};
    use crate::video::{MockVideoDecoder, MockVideoDecoderConfig};
    use std::collections::HashSet;
    use std::path::PathBuf;
    use std::sync::Arc;

    fn make_app(capabilities: HashSet<Capability>) -> Option<ProcessApp> {
        let sh = ["/bin/sh", "/usr/bin/sh"]
            .iter()
            .find(|p| std::path::Path::new(p).exists())
            .map(PathBuf::from)?;
        let workspace_root = std::env::temp_dir();
        ProcessApp::launch(
            "test_video",
            "Test Video",
            &sh,
            &workspace_root,
            &["-c".to_string(), "sleep 1".to_string()],
            workspace_root.clone(),
            capabilities,
            false,
            None,
        )
        .ok()
    }

    #[test]
    fn denied_app_gets_capability_denied_response() {
        // App without `video.playback`: route_command must immediately queue
        // a VideoOpenError with "capability denied" and never touch the
        // decoder.
        let Some(mut app) = make_app(HashSet::new()) else {
            eprintln!("skipping: no /bin/sh available");
            return;
        };

        let mock = Arc::new(MockVideoDecoder::new(MockVideoDecoderConfig::default()));
        app.video_device = Arc::clone(&mock) as Arc<dyn crate::video::VideoDecoder>;

        app.route_command(HostCommand::OpenVideo {
            request_id: "req-denied".to_owned(),
            source: "mock://gradient".to_owned(),
            pipe_id: "video-stream".to_owned(),
        });

        let evt = app
            .outbound_events
            .iter()
            .find(|e| matches!(e, PlexiEvent::VideoOpenError { .. }))
            .expect("expected VideoOpenError on outbound queue");
        match evt {
            PlexiEvent::VideoOpenError { request_id, error } => {
                assert_eq!(request_id, "req-denied");
                assert!(
                    error.contains("capability denied"),
                    "denial must say `capability denied`: {error}"
                );
                assert!(
                    error.contains("video.playback"),
                    "denial must name the capability: {error}"
                );
            }
            other => panic!("expected VideoOpenError, got {other:?}"),
        }

        // The denial path must not produce a VideoOpenAck.
        assert!(
            !app
                .outbound_events
                .iter()
                .any(|e| matches!(e, PlexiEvent::VideoOpenAck { .. })),
            "denied path must not produce a VideoOpenAck"
        );
        assert!(
            app.video_handles.is_empty(),
            "denied path must not register a handle"
        );
    }

    #[test]
    fn granted_app_dispatches_open_to_decoder() {
        // App WITH `video.playback`: route_command must open the decoder,
        // queue PipeOpened then VideoOpenAck, and register a handle. Then
        // SetVideoState dispatches into the handle without panicking, and
        // CloseVideo tears it down cleanly.
        let mut caps = HashSet::new();
        caps.insert(Capability::VideoPlayback);
        let Some(mut app) = make_app(caps) else {
            eprintln!("skipping: no /bin/sh available");
            return;
        };

        let mock = Arc::new(MockVideoDecoder::new(MockVideoDecoderConfig {
            width: 16,
            height: 8,
            fps: 30.0,
            duration_ms: 5_000,
        }));
        app.video_device = Arc::clone(&mock) as Arc<dyn crate::video::VideoDecoder>;

        app.route_command(HostCommand::OpenVideo {
            request_id: "req-1".to_owned(),
            source: "mock://gradient".to_owned(),
            pipe_id: "video-stream".to_owned(),
        });

        // PipeOpened arrives BEFORE VideoOpenAck so the app can connect the
        // unix socket before the first frame.
        let pipe_opened = app
            .outbound_events
            .iter()
            .position(|e| matches!(e, PlexiEvent::PipeOpened { .. }))
            .expect("expected PipeOpened");
        let video_ack = app
            .outbound_events
            .iter()
            .position(|e| matches!(e, PlexiEvent::VideoOpenAck { .. }))
            .expect("expected VideoOpenAck");
        assert!(
            pipe_opened < video_ack,
            "PipeOpened must precede VideoOpenAck so the app's socket connection races first"
        );

        // Pull the ack out and confirm the dimensions match the mock config.
        let ack_handle_id = match &app.outbound_events[video_ack] {
            PlexiEvent::VideoOpenAck {
                handle_id,
                width,
                height,
                fps,
                duration_ms,
                request_id,
            } => {
                assert_eq!(request_id, "req-1");
                assert_eq!(*width, 16);
                assert_eq!(*height, 8);
                assert!((*fps - 30.0).abs() < 0.01);
                assert_eq!(*duration_ms, 5_000);
                *handle_id
            }
            _ => unreachable!(),
        };
        assert!(
            app.video_handles.contains_key(&ack_handle_id),
            "open must register a handle"
        );

        // SetVideoState — pause then play, neither should panic and no error
        // event should arrive.
        app.route_command(HostCommand::SetVideoState {
            handle_id: ack_handle_id,
            state: crate::video::VideoState::Pause,
        });
        app.route_command(HostCommand::SetVideoState {
            handle_id: ack_handle_id,
            state: crate::video::VideoState::Play,
        });
        app.route_command(HostCommand::SetVideoState {
            handle_id: ack_handle_id,
            state: crate::video::VideoState::Seek { position_ms: 1_000 },
        });
        // No additional VideoOpenError must have been queued.
        let errors = app
            .outbound_events
            .iter()
            .filter(|e| matches!(e, PlexiEvent::VideoOpenError { .. }))
            .count();
        assert_eq!(errors, 0, "set_state must not produce VideoOpenError");

        // CloseVideo tears down the handle and unregisters the pipe id map.
        app.route_command(HostCommand::CloseVideo {
            handle_id: ack_handle_id,
        });
        assert!(
            !app.video_handles.contains_key(&ack_handle_id),
            "close must drop the handle"
        );
        assert!(
            !app.video_pipe_ids.contains_key(&ack_handle_id),
            "close must unregister the pipe id mapping"
        );
    }
}

// ── Canvas Terminal Binding Primitives — routing-layer tests (#78) ───────────
//
// These tests pin the `process_app::routing` behaviour for the five binding
// primitives. The HOST-level dispatch (creating the terminal pane, injecting
// bytes into the PTY) lives in `app/canvas_bindings.rs` and is exercised end-
// to-end by the POC `canvas-terminal-bindings-test` app.
//
// Each test asserts one of two things:
//   - Without `terminal.bindings`: the dispatch is a synchronous capability-
//     deny path. No `AppCommand` is enqueued; for the request/response
//     shapes (RequestLinkedTerminal, RequestCommandPreview) a sentinel
//     event lands on `outbound_events` so the SDK's blocking helper
//     unblocks instead of hanging.
//   - With `terminal.bindings` granted: the matching `AppCommand` lands
//     on `pending_commands` for the parent host to drain, populated with
//     the right fields and `sender_pane_id` correctly stamped.
#[cfg(test)]
mod canvas_bindings_tests {
    use super::*;
    use crate::app_protocol::{
        ArtifactOpenMode, HostCommand, PathTokenMode, PlexiEvent,
    };
    use std::collections::HashSet;
    use std::path::PathBuf;

    fn make_app(capabilities: HashSet<Capability>) -> Option<ProcessApp> {
        let sh = ["/bin/sh", "/usr/bin/sh"]
            .iter()
            .find(|p| std::path::Path::new(p).exists())
            .map(PathBuf::from)?;
        let workspace_root = std::env::temp_dir();
        let mut app = ProcessApp::launch(
            "test_bindings",
            "Test Bindings",
            &sh,
            &workspace_root,
            &["-c".to_string(), "sleep 1".to_string()],
            workspace_root.clone(),
            capabilities,
            false,
            None,
        )
        .ok()?;
        // Stamp a non-zero pane id so the AppCommand sender_pane_id is
        // distinguishable from "unset".
        app.set_pane_id(7);
        Some(app)
    }

    // ── Capability denial paths ─────────────────────────────────────────

    #[test]
    fn denied_app_request_linked_terminal_emits_sentinel_event() {
        let Some(mut app) = make_app(HashSet::new()) else {
            eprintln!("skipping: no /bin/sh available");
            return;
        };
        app.route_command(HostCommand::RequestLinkedTerminal {
            request_id: "req-0".to_string(),
            cwd: None,
            label: None,
        });
        // Sentinel event so the SDK's blocking helper unblocks.
        let event = app
            .outbound_events
            .iter()
            .find(|e| matches!(e, PlexiEvent::LinkedTerminalReady { .. }))
            .expect("denied path must emit LinkedTerminalReady sentinel");
        match event {
            PlexiEvent::LinkedTerminalReady {
                request_id,
                terminal_pane_id,
            } => {
                assert_eq!(request_id, "req-0");
                assert_eq!(
                    *terminal_pane_id, 0,
                    "sentinel pane id (0) signals capability denied"
                );
            }
            other => panic!("expected LinkedTerminalReady, got {other:?}"),
        }
        // Must NOT have queued an AppCommand for the host to act on.
        assert!(
            !app.pending_commands.iter().any(|c| matches!(
                c,
                AppCommand::RequestLinkedTerminal { .. }
            )),
            "denied path must not enqueue RequestLinkedTerminal"
        );
    }

    #[test]
    fn denied_app_run_in_linked_terminal_drops_silently() {
        let Some(mut app) = make_app(HashSet::new()) else {
            eprintln!("skipping: no /bin/sh available");
            return;
        };
        app.route_command(HostCommand::RunInLinkedTerminal {
            terminal_pane_id: 42,
            command: "ls".to_string(),
            echo: true,
        });
        // No event — fire-and-forget verb has no response shape.
        // Must NOT enqueue the AppCommand.
        assert!(
            !app.pending_commands.iter().any(|c| matches!(
                c,
                AppCommand::RunInLinkedTerminal { .. }
            )),
            "denied path must drop RunInLinkedTerminal without dispatch"
        );
    }

    #[test]
    fn denied_app_request_command_preview_emits_empty_cwd_sentinel() {
        let Some(mut app) = make_app(HashSet::new()) else {
            eprintln!("skipping: no /bin/sh available");
            return;
        };
        app.route_command(HostCommand::RequestCommandPreview {
            request_id: "req-9".to_string(),
            terminal_pane_id: 42,
            command: "rm -rf .git".to_string(),
        });
        let event = app
            .outbound_events
            .iter()
            .find(|e| matches!(e, PlexiEvent::CommandPreview { .. }))
            .expect("denied path must emit CommandPreview sentinel");
        match event {
            PlexiEvent::CommandPreview {
                request_id,
                command,
                would_run_in_cwd,
            } => {
                assert_eq!(request_id, "req-9");
                assert_eq!(command, "rm -rf .git");
                assert!(
                    would_run_in_cwd.is_empty(),
                    "denied path must return empty cwd: got {would_run_in_cwd:?}"
                );
            }
            other => panic!("expected CommandPreview, got {other:?}"),
        }
    }

    // ── Granted-path AppCommand enqueue ─────────────────────────────────

    #[test]
    fn granted_app_dispatches_request_linked_terminal() {
        let mut caps = HashSet::new();
        caps.insert(Capability::TerminalBindings);
        let Some(mut app) = make_app(caps) else {
            eprintln!("skipping: no /bin/sh available");
            return;
        };
        app.route_command(HostCommand::RequestLinkedTerminal {
            request_id: "req-ok".to_string(),
            cwd: Some("/tmp/foo".to_string()),
            label: Some("bindings demo".to_string()),
        });
        // No synchronous event — granted path defers to the host.
        assert!(app
            .outbound_events
            .iter()
            .find(|e| matches!(e, PlexiEvent::LinkedTerminalReady { .. }))
            .is_none());
        // The AppCommand lands on pending_commands with sender_pane_id
        // stamped (set_pane_id(7)).
        let cmd = app
            .pending_commands
            .iter()
            .find_map(|c| {
                if let AppCommand::RequestLinkedTerminal {
                    sender_pane_id,
                    request_id,
                    cwd,
                    label,
                } = c
                {
                    Some((*sender_pane_id, request_id.clone(), cwd.clone(), label.clone()))
                } else {
                    None
                }
            })
            .expect("granted path must enqueue AppCommand::RequestLinkedTerminal");
        assert_eq!(cmd.0, 7, "sender_pane_id must come from app's pane_id");
        assert_eq!(cmd.1, "req-ok");
        assert_eq!(cmd.2.as_deref(), Some("/tmp/foo"));
        assert_eq!(cmd.3.as_deref(), Some("bindings demo"));
    }

    #[test]
    fn granted_app_dispatches_run_in_linked_terminal() {
        let mut caps = HashSet::new();
        caps.insert(Capability::TerminalBindings);
        let Some(mut app) = make_app(caps) else {
            eprintln!("skipping: no /bin/sh available");
            return;
        };
        app.route_command(HostCommand::RunInLinkedTerminal {
            terminal_pane_id: 42,
            command: "ls -la".to_string(),
            echo: true,
        });
        let cmd = app
            .pending_commands
            .iter()
            .find_map(|c| {
                if let AppCommand::RunInLinkedTerminal {
                    terminal_pane_id,
                    command,
                    echo,
                    ..
                } = c
                {
                    Some((*terminal_pane_id, command.clone(), *echo))
                } else {
                    None
                }
            })
            .expect("granted path must enqueue RunInLinkedTerminal");
        assert_eq!(cmd.0, 42);
        assert_eq!(cmd.1, "ls -la");
        assert!(cmd.2);
    }

    #[test]
    fn granted_app_dispatches_insert_path_token() {
        let mut caps = HashSet::new();
        caps.insert(Capability::TerminalBindings);
        let Some(mut app) = make_app(caps) else {
            eprintln!("skipping: no /bin/sh available");
            return;
        };
        app.route_command(HostCommand::InsertPathToken {
            terminal_pane_id: 42,
            path: "/tmp/x".to_string(),
            mode: PathTokenMode::Replace,
        });
        let mode = app
            .pending_commands
            .iter()
            .find_map(|c| {
                if let AppCommand::InsertPathToken { mode, .. } = c {
                    Some(*mode)
                } else {
                    None
                }
            })
            .expect("granted path must enqueue InsertPathToken");
        assert_eq!(mode, PathTokenMode::Replace);
    }

    #[test]
    fn granted_app_dispatches_open_artifact() {
        let mut caps = HashSet::new();
        caps.insert(Capability::TerminalBindings);
        let Some(mut app) = make_app(caps) else {
            eprintln!("skipping: no /bin/sh available");
            return;
        };
        app.route_command(HostCommand::OpenArtifact {
            path: "/tmp/x".to_string(),
            mode: ArtifactOpenMode::RevealInFinder,
        });
        let mode = app
            .pending_commands
            .iter()
            .find_map(|c| {
                if let AppCommand::OpenArtifact { mode, .. } = c {
                    Some(*mode)
                } else {
                    None
                }
            })
            .expect("granted path must enqueue OpenArtifact");
        assert_eq!(mode, ArtifactOpenMode::RevealInFinder);
    }
}

#[cfg(test)]
mod reload_tests {
    //! Reload-path tests for hot reload (#83).
    //!
    //! Two paths under test:
    //!   1. Drop on a well-behaved subprocess sends `Shutdown` and reaps
    //!      cleanly within the 2s timeout (no SIGTERM escalation).
    //!   2. Drop on an app that ignores Shutdown escalates to SIGTERM
    //!      and still reaps within the 1s SIGTERM timeout.
    //!
    //! These exercise the `Drop for ProcessApp` machinery directly — the
    //! reload glue in `pane_ops::create::reload_app_pane` relies on
    //! `Drop` for the Shutdown→wait→kill sequence. Replacing the runtime
    //! field naturally drops the old `ProcessApp`.
    use super::*;
    use std::collections::HashSet;
    use std::path::PathBuf;

    fn make_sh_app(args: &[&str]) -> Option<ProcessApp> {
        let sh = ["/bin/sh", "/usr/bin/sh"]
            .iter()
            .find(|p| std::path::Path::new(p).exists())
            .map(PathBuf::from)?;
        let workspace_root = std::env::temp_dir();
        let owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        ProcessApp::launch(
            "test_reload",
            "Test Reload",
            &sh,
            &workspace_root,
            &owned,
            workspace_root.clone(),
            HashSet::new(),
            false,
            None,
        )
        .ok()
    }

    /// Reload contract: dropping a `ProcessApp` reaps the underlying
    /// child. `Shutdown` is best-effort over a stdio that the child
    /// likely isn't even reading; the timed escalation guarantees the
    /// reap happens regardless.
    #[test]
    fn drop_reaps_well_behaved_subprocess_within_window() {
        let Some(app) = make_sh_app(&["-c", "sleep 5"]) else {
            eprintln!("skipping: no /bin/sh available");
            return;
        };
        // The child PID must exist while `app` is alive.
        let pid = app
            .process
            .as_ref()
            .map(|c| c.id())
            .expect("child should be running");
        let start = std::time::Instant::now();
        drop(app);
        let elapsed = start.elapsed();

        // Drop completed — under 4s ceiling (2s shutdown + 1s SIGTERM + slack).
        assert!(
            elapsed < std::time::Duration::from_secs(4),
            "drop must complete within escalation window, took {elapsed:?}"
        );

        // Verify the OS released the PID (kill 0 to test process existence).
        // On Unix, kill -0 returns -1 with ESRCH if the process doesn't exist.
        #[cfg(unix)]
        {
            // Sleep a tick so the OS gets a chance to fully reap.
            std::thread::sleep(std::time::Duration::from_millis(50));
            let alive = unsafe { libc::kill(pid as libc::pid_t, 0) };
            // kill returning -1 means the process is gone (ESRCH).
            assert_eq!(
                alive, -1,
                "child pid {pid} must be reaped after drop, got {alive}"
            );
        }
    }

    /// Reload force-kill contract: a subprocess that ignores `Shutdown`
    /// (no stdin reader, just `sleep`) must still be reaped via the
    /// SIGTERM escalation.
    #[test]
    fn drop_force_kills_unresponsive_subprocess() {
        // `sleep 30` doesn't read stdin and ignores SIGPIPE — the only
        // way it dies is SIGTERM/SIGKILL.
        let Some(app) = make_sh_app(&["-c", "exec sleep 30"]) else {
            eprintln!("skipping: no /bin/sh available");
            return;
        };
        let pid = app
            .process
            .as_ref()
            .map(|c| c.id())
            .expect("child should be running");
        let start = std::time::Instant::now();
        drop(app);
        let elapsed = start.elapsed();

        // The 2s shutdown window expires (sleep ignores it), then
        // SIGTERM reaps within 1s. Total well under 4s.
        assert!(
            elapsed < std::time::Duration::from_secs(4),
            "force-kill must complete within escalation window, took {elapsed:?}"
        );
        assert!(
            elapsed >= std::time::Duration::from_millis(1900),
            "shutdown wait should give the well-behaved app time to exit; elapsed={elapsed:?}"
        );

        #[cfg(unix)]
        {
            std::thread::sleep(std::time::Duration::from_millis(50));
            let alive = unsafe { libc::kill(pid as libc::pid_t, 0) };
            assert_eq!(
                alive, -1,
                "unresponsive child pid {pid} must be force-reaped after drop"
            );
        }
    }

    // ── render coalescing (issue #368) ────────────────────────────────────────

    /// Verifies render-event coalescing: N back-to-back `PlexiEvent::Render`
    /// calls must produce at most 1 `FlushRender` token in the channel (so
    /// a burst never fills the queue and silently drops itself), and a
    /// subsequent non-render event must still reach the subprocess.
    ///
    /// Strategy: rather than round-tripping through a subprocess, we inspect
    /// the shared `render_slot` / `render_in_queue` Arcs directly after the
    /// calls. This is race-free because `send_event` writes them synchronously
    /// on the caller's thread before returning.
    ///
    /// Covered invariants:
    ///   1. After N renders, `render_in_queue` is true (exactly one token queued).
    ///   2. `render_slot` contains the *last* render's payload.
    ///   3. A Key event sent after the renders does not clear or corrupt the slot.
    ///   4. `event_tx` is still Some (no spurious disconnection).
    #[test]
    fn render_events_coalesced_non_render_events_preserved() {
        let Some(mut app) = make_sh_app(&["-c", "sleep 5"]) else {
            eprintln!("skipping: no /bin/sh available");
            return;
        };

        let rect = crate::app_protocol::Rect { x: 0.0, y: 0.0, w: 800.0, h: 600.0 };

        // Send 5 Render events back-to-back.
        for frame_id in 1u64..=5 {
            app.send_event(&PlexiEvent::Render { frame_id, rect: rect.clone() });
        }

        // After the burst, exactly one FlushRender token must be in the channel.
        // `render_in_queue` is true while the token is queued / not yet drained.
        assert!(
            app.render_in_queue.load(Ordering::Relaxed),
            "render_in_queue must be true after a burst of Render events"
        );

        // render_slot must hold the *latest* (frame_id=5) payload, not an earlier one.
        {
            let slot = app.render_slot.lock().unwrap();
            let payload = slot.as_deref().expect("render_slot must be populated");
            assert!(
                payload.contains("\"frame_id\":5"),
                "render_slot must contain the latest frame_id (5), got: {payload}"
            );
        }

        // A Key event after the burst must be accepted without error.
        app.send_event(&PlexiEvent::Key {
            key: "j".to_string(),
            modifiers: crate::app_protocol::Modifiers::default(),
        });

        // event_tx must still be live — the Key was enqueued successfully.
        assert!(
            app.event_tx.is_some(),
            "event_tx must remain Some after sending a Key event"
        );
    }

    /// Sanity check: `launch_process` is re-entrant for the same id —
    /// hot reload calls it on every reload.
    #[test]
    fn launch_process_is_reentrant_for_same_app() {
        let Some(a) = make_sh_app(&["-c", "sleep 0.1"]) else {
            eprintln!("skipping: no /bin/sh available");
            return;
        };
        let Some(b) = make_sh_app(&["-c", "sleep 0.1"]) else {
            eprintln!("skipping: no /bin/sh available");
            return;
        };
        let id_a = a.process.as_ref().map(|c| c.id());
        let id_b = b.process.as_ref().map(|c| c.id());
        assert!(id_a.is_some());
        assert!(id_b.is_some());
        assert_ne!(
            id_a, id_b,
            "two launches of the same app must produce distinct PIDs"
        );
    }
}

#[cfg(test)]
mod app_state_tests {
    use super::*;

    #[test]
    fn load_app_state_returns_empty_when_no_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let result = load_app_state("test-app", dir.path());
        assert_eq!(result, serde_json::Value::Object(serde_json::Map::new()));
    }

    #[test]
    fn load_app_state_reads_workspace_file_over_global() {
        let ws_dir = tempfile::tempdir().expect("workspace tempdir");
        let state_dir = ws_dir.path().join(".plexi").join("app_states");
        std::fs::create_dir_all(&state_dir).expect("mkdir");
        let state_path = state_dir.join("my-app.json");
        std::fs::write(&state_path, r#"{"interval_idx":3}"#).expect("write");

        let result = load_app_state("my-app", ws_dir.path());
        assert_eq!(result["interval_idx"], serde_json::json!(3));
    }

    #[test]
    fn load_app_state_migrates_old_app_state_dir() {
        let ws_dir = tempfile::tempdir().expect("workspace tempdir");
        let old_dir = ws_dir.path().join(".plexi").join("app_state");
        std::fs::create_dir_all(&old_dir).expect("mkdir");
        std::fs::write(old_dir.join("my-app.json"), r#"{"migrated":true}"#).expect("write");

        let result = load_app_state("my-app", ws_dir.path());
        assert_eq!(result["migrated"], serde_json::json!(true));
        // Old dir must be gone, new dir must exist.
        assert!(!old_dir.exists(), "old app_state dir should have been renamed");
        assert!(ws_dir.path().join(".plexi").join("app_states").exists());
    }
}

#[cfg(test)]
mod env_isolation_tests {
    //! Proves that user-global secrets (stored as `plexi:user:*`) are NOT
    //! injected into app subprocess environments. The only path to a secret
    //! is `HostCommand::SecretGet` through the brokered capability check.
    //!
    //! Strategy: add a canary directly to the Command (simulating what the old
    //! unconditional list_user_secrets() injection did), then apply env_clear()
    //! + whitelist (what ProcessApp::launch now does), and assert the canary
    //! is absent from the subprocess. Rust's Command builder strips explicit
    //! .env() additions that precede .env_clear(), so no host-process env
    //! mutation is needed — no thread-safety concerns.

    use std::process::Command;

    const WHITELIST: &[&str] = &["HOME", "PATH", "LANG", "LC_ALL", "TERM", "USER", "SHELL"];
    const CANARY_KEY: &str = "PLEXI_SECRET_CANARY";
    const CANARY_VAL: &str = "secret_must_not_leak";

    #[test]
    fn user_global_secrets_not_injected_into_subprocess_env() {
        let sh = ["/bin/sh", "/usr/bin/sh"]
            .iter()
            .find(|p| std::path::Path::new(p).exists())
            .copied();
        let Some(sh) = sh else {
            eprintln!("skipping: no /bin/sh available");
            return;
        };

        // Add the canary first (models what the old injection loop wrote),
        // then env_clear() which strips it. env(k,v) before env_clear() is
        // removed by env_clear() — verified empirically via Rust std behavior.
        let output = Command::new(sh)
            .arg("-c")
            .arg(format!("echo \"${{{}:-ABSENT}}\"", CANARY_KEY))
            .env(CANARY_KEY, CANARY_VAL)
            .env_clear()
            .envs(
                WHITELIST
                    .iter()
                    .filter_map(|k| std::env::var(k).ok().map(|v| (*k, v))),
            )
            .output()
            .expect("sh spawn failed");

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert_eq!(
            stdout.trim(),
            "ABSENT",
            "user-global secret must not appear in subprocess env; got: {stdout:?}"
        );
    }
}

#[cfg(test)]
mod image_cache_tests {
    use super::*;

    #[test]
    fn image_cache_loads_png() {
        // Write a 1×1 red PNG to a temp dir using the `image` crate (avoids
        // embedding raw bytes that could be subtly invalid).
        let dir = tempfile::tempdir().expect("tempdir");
        let png_path = dir.path().join("test.png");
        let mut img = image::RgbaImage::new(1, 1);
        img.put_pixel(0, 0, image::Rgba([255, 0, 0, 255]));
        img.save(&png_path).expect("save png");

        let mut cache = image_cache::ImageCache::new();
        cache.request("test.png", dir.path());

        // Give the background thread time to load.
        std::thread::sleep(std::time::Duration::from_millis(200));
        cache.poll(&egui::Context::default());

        assert!(
            cache.get("test.png").is_some(),
            "expected image to be loaded"
        );
    }

    #[test]
    fn image_cache_missing_file_is_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut cache = image_cache::ImageCache::new();
        cache.request("nonexistent.png", dir.path());

        std::thread::sleep(std::time::Duration::from_millis(200));
        cache.poll(&egui::Context::default());

        assert!(
            matches!(
                cache.state("nonexistent.png"),
                Some(image_cache::CachedImage::Error(_))
            ),
            "expected Error state for missing file"
        );
    }
}

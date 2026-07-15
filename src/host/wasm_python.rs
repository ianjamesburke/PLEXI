//! CPython-in-WASM adapter boundary for SDK v3 Python apps.
//!
//! This module owns the deterministic host side of the Python compatibility
//! path: manifest routing, CPython bundle resolution, and JSON bridge
//! marshalling. It intentionally does not fall back to the native PGAP
//! subprocess path when the CPython WASM bundle is unavailable.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::Command;
#[cfg(test)]
use std::process::Stdio;
use std::sync::{Arc, LazyLock, Mutex};
use std::task::{Context, Poll, Waker};
use std::thread::JoinHandle;

#[cfg(test)]
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use wasmtime::{Engine as WasmtimeEngine, Linker, Module, Store};
use wasmtime_wasi::cli::{IsTerminal, StdinStream, StdoutStream};
use wasmtime_wasi::p1::{self, WasiP1Ctx};
use wasmtime_wasi::{DirPerms, FilePerms, WasiCtxBuilder};

use crate::app::registry::{AppManifest, RuntimeExecution};

use super::wasm_app::bindings::plexi::platform::types::{
    BadgeColor, ButtonNode, ButtonStyle, CanvasCircle, CanvasCommand, CanvasLine, CanvasNode,
    CanvasRect, CanvasText, Color, ColumnNode, IndexedNode, ListNode, PaddingNode, ProgressBarNode,
    RowNode, ScrollNode, TextInputNode, TextNode, UiNodeData, UiTree,
};
#[cfg(test)]
use super::wasm_app::bindings::plexi::platform::types::{
    FileReadEffect, FileWriteEffect, HttpFetchEffect, InputEvent, KeyEvent, StateSnapshot,
    TimerEffect, UiActionEvent, UiValueChangeEvent,
};
use super::wasm_app::Alignment;
#[cfg(test)]
use super::wasm_app::{Effect, Grants, StateStore, WasmApp};

#[derive(Default)]
struct InputState {
    bytes: VecDeque<u8>,
    closed: bool,
    waker: Option<Waker>,
}

/// Cloneable WASI stdin whose producer can append bytes after instantiation.
#[derive(Clone, Default)]
pub struct AppendableStdin {
    state: Arc<Mutex<InputState>>,
}

impl AppendableStdin {
    pub fn push(&self, bytes: &[u8]) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.bytes.extend(bytes);
        if let Some(waker) = state.waker.take() {
            waker.wake();
        }
    }

    pub fn push_json_line(&self, value: &Value) -> Result<(), WasmPythonError> {
        let mut line = serde_json::to_vec(value)
            .map_err(|error| WasmPythonError::BridgeJson(error.to_string()))?;
        line.push(b'\n');
        self.push(&line);
        Ok(())
    }

    pub fn close(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.closed = true;
        if let Some(waker) = state.waker.take() {
            waker.wake();
        }
    }
}

impl IsTerminal for AppendableStdin {
    fn is_terminal(&self) -> bool {
        false
    }
}

impl StdinStream for AppendableStdin {
    fn async_stream(&self) -> Box<dyn AsyncRead + Send + Sync> {
        Box::new(self.clone())
    }
}

impl AsyncRead for AppendableStdin {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if !state.bytes.is_empty() {
            let count = buf.remaining().min(state.bytes.len());
            let bytes: Vec<u8> = state.bytes.drain(..count).collect();
            buf.put_slice(&bytes);
            return Poll::Ready(Ok(()));
        }
        if state.closed {
            return Poll::Ready(Ok(()));
        }
        state.waker = Some(cx.waker().clone());
        Poll::Pending
    }
}

struct OutputState {
    bytes: Vec<u8>,
    wake: Option<Arc<dyn Fn() + Send + Sync>>,
    wake_pending: bool,
    notifications_enabled: bool,
}

impl Default for OutputState {
    fn default() -> Self {
        Self {
            bytes: Vec::new(),
            wake: None,
            wake_pending: false,
            notifications_enabled: true,
        }
    }
}

/// Cloneable WASI output that can be drained without closing the guest stream.
#[derive(Clone, Default)]
pub struct DrainableOutput {
    state: Arc<Mutex<OutputState>>,
}

impl DrainableOutput {
    pub fn drain(&self) -> Vec<u8> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.wake_pending = false;
        std::mem::take(&mut state.bytes)
    }

    fn set_waker(&self, wake: Arc<dyn Fn() + Send + Sync>) {
        let should_wake = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.wake = Some(wake.clone());
            if state.notifications_enabled && !state.wake_pending && state.bytes.contains(&b'\n') {
                state.wake_pending = true;
                true
            } else {
                false
            }
        };
        if should_wake {
            wake();
        }
    }

    fn set_notifications_enabled(&self, enabled: bool) {
        let wake = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.notifications_enabled = enabled;
            if enabled && !state.wake_pending && state.bytes.contains(&b'\n') {
                state.wake_pending = true;
                state.wake.clone()
            } else {
                None
            }
        };
        if let Some(wake) = wake {
            wake();
        }
    }
}

impl IsTerminal for DrainableOutput {
    fn is_terminal(&self) -> bool {
        false
    }
}

impl StdoutStream for DrainableOutput {
    fn async_stream(&self) -> Box<dyn AsyncWrite + Send + Sync> {
        Box::new(self.clone())
    }
}

impl AsyncWrite for DrainableOutput {
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .bytes
            .extend_from_slice(buf);
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        let wake = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.notifications_enabled && !state.wake_pending && state.bytes.contains(&b'\n') {
                state.wake_pending = true;
                state.wake.clone()
            } else {
                None
            }
        };
        if let Some(wake) = wake {
            wake();
        }
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

pub const CPYTHON_BUNDLE_VERSION: &str = "3.12.12";
#[cfg(test)]
pub const CPYTHON_WASI_SDK_VERSION: &str = "20";
pub const CPYTHON_BUNDLE_FILE: &str = "cpython-3.12.12/python.wasm";
#[cfg(test)]
pub const CPYTHON_SHIM_COMPONENT_FILE: &str = "cpython-3.12.12/plexi-python-shim.wasm";
pub const CPYTHON_BUNDLE_SHA256: &str =
    "62392f07fee032c22e3aa84be033c07105cd42424e5149058b9f5449a8deb272";
pub const CPYTHON_BUNDLE_CACHE_ENV: &str = "PLEXI_CPYTHON_BUNDLE_DIR";
pub const FETCH_CPYTHON_BUNDLE_COMMAND: &str = "just fetch-cpython-bundle";
#[cfg(test)]
pub const BUILD_CPYTHON_SHIM_COMMAND: &str = "just wasm-python-shim";

static CPYTHON_MODULE_CACHE: LazyLock<Mutex<HashMap<PathBuf, (WasmtimeEngine, Module)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn cached_cpython_module(path: &Path) -> Result<(WasmtimeEngine, Module), String> {
    let key = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let mut cache = CPYTHON_MODULE_CACHE
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if let Some(cached) = cache.get(&key) {
        return Ok(cached.clone());
    }
    let engine = WasmtimeEngine::default();
    let module = Module::from_file(&engine, path).map_err(|error| error.to_string())?;
    cache.insert(key, (engine.clone(), module.clone()));
    Ok((engine, module))
}

#[derive(Debug, Error)]
pub enum WasmPythonError {
    #[error("read manifest at {path}: {source}")]
    ReadManifest {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("parse manifest at {path}: {source}")]
    ParseManifest {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("runtime.python_compat requires a .py or .pyc entry, got '{entry}'")]
    InvalidEntry { entry: String },
    #[error("runtime.python_compat entry is missing: {path}")]
    MissingEntry { path: PathBuf },
    #[error("runtime.python_compat execution='{execution}' is not implemented yet")]
    UnsupportedExecution { execution: &'static str },
    #[error("CPython WASM bundle unavailable at {path}; run: {command}")]
    MissingBundle {
        path: PathBuf,
        command: &'static str,
    },
    #[error("CPython lifecycle shim component unavailable at {path}; run: {command}")]
    #[cfg(test)]
    MissingShimComponent {
        path: PathBuf,
        command: &'static str,
    },
    #[error("CPython WASM bundle hash is not pinned for {version}; run: {command}")]
    BundleHashUnpinned {
        version: &'static str,
        command: &'static str,
    },
    #[error("CPython WASM bundle hash mismatch at {path}: expected {expected}, got {actual}")]
    BundleHashMismatch {
        path: PathBuf,
        expected: &'static str,
        actual: String,
    },
    #[error("raw WASM module ABI mismatch at {path}: {reason}")]
    #[cfg(test)]
    RawModuleAbiMismatch { path: PathBuf, reason: String },
    #[error("load CPython lifecycle shim component at {path}: {message}")]
    #[cfg(test)]
    ShimComponentLoadFailure { path: PathBuf, message: String },
    #[error("CPython lifecycle shim call '{function}' failed at {path}: {message}")]
    #[cfg(test)]
    ShimLifecycleCallFailure {
        path: PathBuf,
        function: &'static str,
        message: String,
    },
    #[error("read CPython WASM bundle at {path}: {source}")]
    ReadBundle {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("bridge JSON error: {0}")]
    BridgeJson(String),
    #[error("start CPython WASM runtime: {0}")]
    RuntimeStart(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PythonLaunchConfig {
    pub app_id: String,
    pub app_dir: PathBuf,
    pub entry: PathBuf,
    pub module_name: String,
    pub launch_args: Vec<String>,
    pub workspace_root: PathBuf,
    pub capabilities: Vec<String>,
    pub allowed_hosts: Vec<String>,
}

impl PythonLaunchConfig {
    pub fn from_manifest_file(app_dir: &Path) -> Result<Option<Self>, WasmPythonError> {
        let manifest_path = app_dir.join("manifest.toml");
        let raw = std::fs::read_to_string(&manifest_path).map_err(|source| {
            WasmPythonError::ReadManifest {
                path: manifest_path.clone(),
                source,
            }
        })?;
        let manifest: AppManifest =
            toml::from_str(&raw).map_err(|source| WasmPythonError::ParseManifest {
                path: manifest_path,
                source,
            })?;

        if manifest.runtime.execution != RuntimeExecution::Local {
            return Err(WasmPythonError::UnsupportedExecution {
                execution: runtime_execution_label(manifest.runtime.execution),
            });
        }

        let entry = manifest.app.entry;
        if !(entry.ends_with(".py") || entry.ends_with(".pyc")) {
            return Err(WasmPythonError::InvalidEntry { entry });
        }
        let entry_path = app_dir.join(&entry);
        if !entry_path.is_file() {
            return Err(WasmPythonError::MissingEntry { path: entry_path });
        }
        let module_name = entry_path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("main")
            .to_string();

        Ok(Some(Self {
            app_id: manifest.app.id,
            app_dir: app_dir.to_path_buf(),
            entry: entry_path,
            module_name,
            launch_args: Vec::new(),
            workspace_root: app_dir.to_path_buf(),
            capabilities: manifest.app.capabilities.capabilities,
            allowed_hosts: manifest.app.capabilities.allowed_hosts,
        }))
    }
}

/// A live CPython interpreter. The owning thread retains the Wasmtime store;
/// lifecycle traffic crosses only the appendable stdin/drainable stdout pair.
pub struct WasmPythonRuntime {
    stdin: AppendableStdin,
    stdout: DrainableOutput,
    stderr: DrainableOutput,
    thread: Option<JoinHandle<Result<(), String>>>,
    partial_stdout: Vec<u8>,
    last_drain_bytes: usize,
    last_json_decode_time: std::time::Duration,
}

impl WasmPythonRuntime {
    pub fn launch(config: &PythonLaunchConfig) -> Result<Self, WasmPythonError> {
        let bundle = resolve_default_cpython_bundle()?;
        let stdlib = bundle
            .parent()
            .ok_or_else(|| {
                WasmPythonError::RuntimeStart("CPython bundle has no parent".to_string())
            })?
            .join("Lib");
        let sdk = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("sdk/python");
        let stdin = AppendableStdin::default();
        let stdout = DrainableOutput::default();
        let stderr = DrainableOutput::default();
        let thread_stdin = stdin.clone();
        let thread_stdout = stdout.clone();
        let thread_stderr = stderr.clone();
        let app_dir = config.app_dir.clone();
        let entry_name = config
            .entry
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| WasmPythonError::RuntimeStart("Python entry is not UTF-8".to_string()))?
            .to_string();
        let app_id = config.app_id.clone();

        let thread = std::thread::Builder::new()
            .name(format!("plexi-python-wasm-{app_id}"))
            .spawn(move || {
                let (engine, module) = cached_cpython_module(&bundle)?;
                let mut linker = Linker::<WasiP1Ctx>::new(&engine);
                p1::add_to_linker_sync(&mut linker, |ctx| ctx).map_err(|e| e.to_string())?;
                let mut builder = WasiCtxBuilder::new();
                builder
                    .stdin(thread_stdin)
                    .stdout(thread_stdout)
                    .stderr(thread_stderr)
                    .env("PYTHONPATH", "/sdk:/app")
                    .args(&[
                        "python",
                        "-u",
                        "-m",
                        "plexi_sdk._v3_process",
                        &format!("/app/{entry_name}"),
                    ]);
                builder
                    .preopened_dir(
                        &stdlib,
                        "/usr/local/lib/python3.12",
                        DirPerms::READ,
                        FilePerms::READ,
                    )
                    .map_err(|e| e.to_string())?
                    .preopened_dir(&sdk, "/sdk", DirPerms::READ, FilePerms::READ)
                    .map_err(|e| e.to_string())?
                    .preopened_dir(&app_dir, "/app", DirPerms::READ, FilePerms::READ)
                    .map_err(|e| e.to_string())?;
                let mut store = Store::new(&engine, builder.build_p1());
                let instance = linker
                    .instantiate(&mut store, &module)
                    .map_err(|e| e.to_string())?;
                let start = instance
                    .get_typed_func::<(), ()>(&mut store, "_start")
                    .map_err(|e| e.to_string())?;
                start.call(&mut store, ()).map_err(|e| e.to_string())
            })
            .map_err(|e| WasmPythonError::RuntimeStart(e.to_string()))?;
        log::info!("app::{app_id}: CPython WASM runtime started with read-only SDK and app mounts");
        Ok(Self {
            stdin,
            stdout,
            stderr,
            thread: Some(thread),
            partial_stdout: Vec::new(),
            last_drain_bytes: 0,
            last_json_decode_time: std::time::Duration::ZERO,
        })
    }

    pub fn send(&self, event: &Value) -> Result<(), WasmPythonError> {
        self.stdin.push_json_line(event)
    }

    pub fn drain_messages(&mut self) -> Result<Vec<Value>, WasmPythonError> {
        let drained = self.stdout.drain();
        self.last_drain_bytes = drained.len();
        self.partial_stdout.extend(drained);
        let mut messages = Vec::new();
        let decode_started = std::time::Instant::now();
        while let Some(newline) = self.partial_stdout.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = self.partial_stdout.drain(..=newline).collect();
            let line = &line[..line.len().saturating_sub(1)];
            if !line.is_empty() {
                messages.push(
                    serde_json::from_slice(line)
                        .map_err(|e| WasmPythonError::BridgeJson(e.to_string()))?,
                );
            }
        }
        self.last_json_decode_time = decode_started.elapsed();
        Ok(messages)
    }

    pub fn drain_stderr(&self) -> String {
        String::from_utf8_lossy(&self.stderr.drain()).into_owned()
    }
}

impl Drop for WasmPythonRuntime {
    fn drop(&mut self) {
        let _ = self.send(&json!({"type": "shutdown"}));
        self.stdin.close();
        if let Some(thread) = self.thread.take() {
            if let Err(error) = thread.join() {
                log::error!("CPython WASM runtime thread panicked: {error:?}");
            }
        }
    }
}

pub struct LivePythonPane {
    config: PythonLaunchConfig,
    runtime: WasmPythonRuntime,
    app_id: String,
    title: Option<String>,
    tree: Option<PythonUiTree>,
    pending_trees: HashMap<u64, PythonUiTree>,
    initialized: bool,
    ready: bool,
    frame_scheduler: PythonFrameScheduler,
    output_waker_installed: bool,
    wants_close: bool,
    error: Option<String>,
    timers: std::collections::HashMap<String, PythonTimer>,
    pending_timer_events: Vec<String>,
    viewport_size: Option<(f32, f32)>,
    perf_started_at: std::time::Instant,
    perf_frames: u64,
    perf_host_time: std::time::Duration,
    perf_guest_frames: u64,
    perf_guest_roundtrip: std::time::Duration,
    perf_json_decode: std::time::Duration,
    perf_tree_decode: std::time::Duration,
    perf_stdout_bytes: usize,
    perf_ui_render: std::time::Duration,
    perf_canvas_render: std::time::Duration,
    persisted_state: serde_json::Map<String, Value>,
    http_tx: std::sync::mpsc::Sender<(String, crate::host::services::HttpResponse)>,
    http_rx: std::sync::mpsc::Receiver<(String, crate::host::services::HttpResponse)>,
    pending_commands: Vec<crate::app::app_trait::AppCommand>,
}

#[derive(Debug)]
struct PythonUiTree {
    tree: UiTree,
    canvas_fits: HashMap<u32, super::wasm_render::CanvasFit>,
}

#[derive(Debug, Clone, Copy)]
struct PythonTimer {
    deadline: std::time::Instant,
    repeat_every: Option<std::time::Duration>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PythonSchedulerMode {
    Scheduled,
    Continuous { interval: std::time::Duration },
}

// Admit one transaction before its presentation deadline so timer jitter and
// guest round-trip do not turn a slightly late wake into a skipped frame.
const CONTINUOUS_FRAME_HEADROOM: std::time::Duration = std::time::Duration::from_millis(5);

#[derive(Debug)]
struct PythonFrameScheduler {
    mode: PythonSchedulerMode,
    next_deadline: std::time::Instant,
    render_requested: bool,
    pending: VecDeque<(u64, std::time::Instant)>,
    next_frame_id: u64,
}

impl PythonFrameScheduler {
    fn new(now: std::time::Instant) -> Self {
        Self {
            mode: PythonSchedulerMode::Scheduled,
            next_deadline: now,
            render_requested: true,
            pending: VecDeque::new(),
            next_frame_id: 0,
        }
    }

    fn request_render_at(&mut self, deadline: std::time::Instant) {
        if matches!(self.mode, PythonSchedulerMode::Continuous { .. }) || self.render_requested {
            self.next_deadline = self.next_deadline.min(deadline);
        } else {
            self.next_deadline = deadline;
        }
        self.render_requested = true;
    }

    fn request_render_after(&mut self, now: std::time::Instant, delay: std::time::Duration) {
        self.request_render_at(now + delay);
    }

    fn set_mode(&mut self, mode: Option<&str>, fps: Option<u64>, now: std::time::Instant) {
        self.mode = match mode {
            Some("continuous") => PythonSchedulerMode::Continuous {
                interval: scheduler_repaint_after(mode, fps),
            },
            _ => PythonSchedulerMode::Scheduled,
        };
        if let PythonSchedulerMode::Continuous { interval } = self.mode {
            self.next_deadline = self.next_deadline.min(now + interval);
        }
    }

    fn poll_render(&mut self, now: std::time::Instant) -> Option<u64> {
        let render_needed =
            matches!(self.mode, PythonSchedulerMode::Continuous { .. }) || self.render_requested;
        if !render_needed
            || self.pending.len() >= self.pipeline_capacity()
            || self.admission_deadline() > now
        {
            return None;
        }
        self.next_frame_id = self.next_frame_id.saturating_add(1);
        let frame_id = self.next_frame_id;
        self.pending.push_back((frame_id, now));
        self.render_requested = false;
        if let PythonSchedulerMode::Continuous { interval } = self.mode {
            self.next_deadline = advance_fixed_deadline(self.next_deadline, interval, now);
        }
        Some(frame_id)
    }

    fn complete_frame(&mut self, completed_frame_id: u64) -> Option<std::time::Instant> {
        let position = self
            .pending
            .iter()
            .position(|(frame_id, _)| *frame_id == completed_frame_id)?;
        self.pending.remove(position).map(|(_, sent_at)| sent_at)
    }

    fn next_repaint_deadline(&self, now: std::time::Instant) -> Option<std::time::Instant> {
        let render_needed =
            matches!(self.mode, PythonSchedulerMode::Continuous { .. }) || self.render_requested;
        if !render_needed {
            return None;
        }
        let admission_deadline = self.admission_deadline();
        if matches!(self.mode, PythonSchedulerMode::Continuous { .. }) && admission_deadline > now {
            return Some(admission_deadline);
        }
        if self.pending.len() >= self.pipeline_capacity() {
            return None;
        }
        Some(admission_deadline.max(now))
    }

    fn output_notifications_enabled(&self, now: std::time::Instant) -> bool {
        !(matches!(self.mode, PythonSchedulerMode::Continuous { .. })
            && self.admission_deadline() > now)
    }

    fn pipeline_capacity(&self) -> usize {
        match self.mode {
            PythonSchedulerMode::Continuous { .. } => 3,
            PythonSchedulerMode::Scheduled => 1,
        }
    }

    fn oldest_pending_frame_id(&self) -> Option<u64> {
        self.pending.front().map(|(frame_id, _)| *frame_id)
    }

    fn admission_deadline(&self) -> std::time::Instant {
        match self.mode {
            PythonSchedulerMode::Continuous { interval } => self
                .next_deadline
                .checked_sub(CONTINUOUS_FRAME_HEADROOM.min(interval / 2))
                .unwrap_or(self.next_deadline),
            PythonSchedulerMode::Scheduled => self.next_deadline,
        }
    }

    fn reset(&mut self, now: std::time::Instant) {
        *self = Self::new(now);
    }
}

fn python_state_path(config: &PythonLaunchConfig) -> PathBuf {
    config
        .workspace_root
        .join(".plexi")
        .join("app_states")
        .join(format!("{}.json", config.app_id))
}

fn load_python_state(config: &PythonLaunchConfig) -> serde_json::Map<String, Value> {
    let path = python_state_path(config);
    match std::fs::read(&path) {
        Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
            Ok(Value::Object(state)) => state,
            Ok(_) => {
                log::warn!(
                    "app::{}: state {} is not a JSON object",
                    config.app_id,
                    path.display()
                );
                serde_json::Map::new()
            }
            Err(error) => {
                log::warn!(
                    "app::{}: parse state {}: {error}",
                    config.app_id,
                    path.display()
                );
                serde_json::Map::new()
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => serde_json::Map::new(),
        Err(error) => {
            log::warn!(
                "app::{}: read state {}: {error}",
                config.app_id,
                path.display()
            );
            serde_json::Map::new()
        }
    }
}

impl LivePythonPane {
    pub fn launch(config: PythonLaunchConfig) -> Result<Self, WasmPythonError> {
        let app_id = config.app_id.clone();
        let persisted_state = load_python_state(&config);
        let (http_tx, http_rx) = std::sync::mpsc::channel();
        Ok(Self {
            runtime: WasmPythonRuntime::launch(&config)?,
            config,
            app_id,
            title: None,
            tree: None,
            pending_trees: HashMap::new(),
            initialized: false,
            ready: false,
            frame_scheduler: PythonFrameScheduler::new(std::time::Instant::now()),
            output_waker_installed: false,
            wants_close: false,
            error: None,
            timers: std::collections::HashMap::new(),
            pending_timer_events: Vec::new(),
            viewport_size: None,
            perf_started_at: std::time::Instant::now(),
            perf_frames: 0,
            perf_host_time: std::time::Duration::ZERO,
            perf_guest_frames: 0,
            perf_guest_roundtrip: std::time::Duration::ZERO,
            perf_json_decode: std::time::Duration::ZERO,
            perf_tree_decode: std::time::Duration::ZERO,
            perf_stdout_bytes: 0,
            perf_ui_render: std::time::Duration::ZERO,
            perf_canvas_render: std::time::Duration::ZERO,
            persisted_state,
            http_tx,
            http_rx,
            pending_commands: Vec::new(),
        })
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, colors: &crate::ui::theme::Colors) {
        let host_frame_started = std::time::Instant::now();
        if !self.output_waker_installed {
            let context = ui.ctx().clone();
            let viewport = ui.ctx().viewport_id();
            self.runtime.stdout.set_waker(Arc::new(move || {
                // A zero-delay egui request deliberately produces two paints.
                // Keep the wake immediate after prediction adjustment without
                // asking egui for its extra settling pass.
                context.request_repaint_after_for(std::time::Duration::from_nanos(1), viewport);
            }));
            self.output_waker_installed = true;
        }
        if let Some(error) = &self.error {
            ui.colored_label(colors.danger, error);
            return;
        }
        if !self.initialized {
            let size = ui.available_size();
            if !valid_python_viewport(size.x, size.y) {
                ui.spinner();
                self.record_render_perf(host_frame_started.elapsed());
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(5));
                return;
            }
            self.initialized = true;
            self.viewport_size = Some((size.x, size.y));
            if let Err(error) = self.runtime.send(&json!({
                "type": "init", "app_id": self.app_id,
                "workspace_root": self.config.workspace_root,
                "capabilities": self.config.capabilities, "state": self.persisted_state, "theme": {},
                "args": self.config.launch_args,
                "size": [size.x, size.y]
            })) {
                self.error = Some(error.to_string());
                return;
            }
        }
        let size = ui.available_size();
        let viewport_changed = self.viewport_size.is_some_and(|(width, height)| {
            (width - size.x).abs() > 0.5 || (height - size.y).abs() > 0.5
        });
        if viewport_changed && valid_python_viewport(size.x, size.y) {
            self.viewport_size = Some((size.x, size.y));
            if let Err(error) = self
                .runtime
                .send(&json!({"type": "resize", "width": size.x, "height": size.y}))
            {
                self.error = Some(error.to_string());
                return;
            }
            self.frame_scheduler
                .request_render_at(std::time::Instant::now());
        }
        self.drain_runtime();
        if !self.ready {
            ui.spinner();
            self.record_render_perf(host_frame_started.elapsed());
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(5));
            return;
        }
        self.fire_due_timers();
        let now = std::time::Instant::now();
        if let Some(frame_id) = self.frame_scheduler.poll_render(now) {
            let timer_ids = std::mem::take(&mut self.pending_timer_events);
            if let Err(error) = self.runtime.send(&python_render_event(frame_id, timer_ids)) {
                self.error = Some(error.to_string());
                return;
            }
            self.drain_runtime();
        }
        if let Some(tree) = &self.tree {
            let render_started = std::time::Instant::now();
            let result = super::wasm_render::render_ui_tree_with_canvas_fits(
                ui,
                &tree.tree,
                colors,
                None,
                Some(&tree.canvas_fits),
            );
            self.perf_ui_render += render_started.elapsed();
            self.perf_canvas_render += result.canvas_time;
            for action in result.actions {
                let _ = self
                    .runtime
                    .send(&json!({"type": "ui_action", "handler_id": action}));
            }
            for (handler_id, value) in result.value_changes {
                let _ = self.runtime.send(&json!({
                    "type": "text_submitted", "id": handler_id, "value": value
                }));
            }
        } else {
            ui.spinner();
        }
        self.record_render_perf(host_frame_started.elapsed());
        let now = std::time::Instant::now();
        let render_deadline = self.frame_scheduler.next_repaint_deadline(now);
        let timer_deadline = self.timers.values().map(|timer| timer.deadline).min();
        self.runtime
            .stdout
            .set_notifications_enabled(self.frame_scheduler.output_notifications_enabled(now));
        if let Some(next_wake) = render_deadline.into_iter().chain(timer_deadline).min() {
            let predicted_frame =
                std::time::Duration::from_secs_f32(ui.input(|input| input.predicted_dt));
            ui.ctx()
                .request_repaint_after(repaint_delay_until(next_wake, now, predicted_frame));
        }
    }

    fn record_render_perf(&mut self, host_time: std::time::Duration) {
        self.perf_frames += 1;
        self.perf_host_time += host_time;
        let elapsed = self.perf_started_at.elapsed();
        if elapsed >= std::time::Duration::from_secs(2) {
            let fps = self.perf_frames as f64 / elapsed.as_secs_f64();
            let avg_host_ms =
                self.perf_host_time.as_secs_f64() * 1000.0 / self.perf_frames.max(1) as f64;
            let guest_fps = self.perf_guest_frames as f64 / elapsed.as_secs_f64();
            let avg_roundtrip_ms = self.perf_guest_roundtrip.as_secs_f64() * 1000.0
                / self.perf_guest_frames.max(1) as f64;
            log::info!(
                "app::{}: CPython-WASM perf paint_fps={fps:.1} guest_fps={guest_fps:.1} avg_host_ms={avg_host_ms:.2} avg_roundtrip_ms={avg_roundtrip_ms:.2} json_ms={:.2} tree_ms={:.2} ui_ms={:.2} canvas_ms={:.2} stdout_kib={:.1}",
                self.app_id,
                self.perf_json_decode.as_secs_f64() * 1000.0,
                self.perf_tree_decode.as_secs_f64() * 1000.0,
                self.perf_ui_render.as_secs_f64() * 1000.0,
                self.perf_canvas_render.as_secs_f64() * 1000.0,
                self.perf_stdout_bytes as f64 / 1024.0,
            );
            self.perf_started_at = std::time::Instant::now();
            self.perf_frames = 0;
            self.perf_host_time = std::time::Duration::ZERO;
            self.perf_guest_frames = 0;
            self.perf_guest_roundtrip = std::time::Duration::ZERO;
            self.perf_json_decode = std::time::Duration::ZERO;
            self.perf_tree_decode = std::time::Duration::ZERO;
            self.perf_stdout_bytes = 0;
            self.perf_ui_render = std::time::Duration::ZERO;
            self.perf_canvas_render = std::time::Duration::ZERO;
        }
    }

    fn reset_perf_window(&mut self) {
        self.perf_started_at = std::time::Instant::now();
        self.perf_frames = 0;
        self.perf_host_time = std::time::Duration::ZERO;
        self.perf_guest_frames = 0;
        self.perf_guest_roundtrip = std::time::Duration::ZERO;
        self.perf_json_decode = std::time::Duration::ZERO;
        self.perf_tree_decode = std::time::Duration::ZERO;
        self.perf_stdout_bytes = 0;
        self.perf_ui_render = std::time::Duration::ZERO;
        self.perf_canvas_render = std::time::Duration::ZERO;
    }

    fn drain_runtime(&mut self) {
        while let Ok((request_id, response)) = self.http_rx.try_recv() {
            let _ = self.runtime.send(&json!({
                "type": "http_response", "request_id": request_id,
                "status": response.status, "body": response.body, "error": response.error,
                "headers": response.response_headers,
            }));
        }
        match self.runtime.drain_messages() {
            Ok(messages) => {
                self.perf_stdout_bytes += self.runtime.last_drain_bytes;
                self.perf_json_decode += self.runtime.last_json_decode_time;
                for message in messages {
                    let message_type = message.get("type").and_then(Value::as_str);
                    log::debug!(
                        "app::{}: CPython WASM message type={}",
                        self.app_id,
                        message_type.unwrap_or("<missing>")
                    );
                    match message_type {
                        Some("ready") => {
                            self.ready = true;
                            self.frame_scheduler
                                .request_render_at(std::time::Instant::now());
                            self.reset_perf_window();
                            log::info!("app::{}: CPython WASM bridge ready", self.app_id);
                        }
                        Some("component_tree") => {
                            if let Some(tree) = message.get("tree") {
                                let decode_started = std::time::Instant::now();
                                match decode_python_ui_tree_value(tree) {
                                    Ok(tree) => {
                                        let frame_id = message
                                            .get("frame_id")
                                            .and_then(Value::as_u64)
                                            .or_else(|| {
                                                self.frame_scheduler.oldest_pending_frame_id()
                                            });
                                        if let Some(frame_id) = frame_id {
                                            self.pending_trees.insert(frame_id, tree);
                                        }
                                    }
                                    Err(error) => {
                                        log::error!(
                                            "app::{}: decode CPython WASM component tree: {error}",
                                            self.app_id
                                        );
                                        self.error = Some(error.to_string());
                                    }
                                }
                                self.perf_tree_decode += decode_started.elapsed();
                            }
                        }
                        Some("set_title") => {
                            self.title = message
                                .get("title")
                                .and_then(Value::as_str)
                                .map(str::to_string)
                        }
                        Some("set_timer") => {
                            if let (Some(id), Some(after_ms)) = (
                                message.get("timer_id").and_then(Value::as_str),
                                message.get("after_ms").and_then(Value::as_u64),
                            ) {
                                let interval = std::time::Duration::from_millis(after_ms.max(1));
                                self.timers.insert(
                                    id.to_string(),
                                    PythonTimer {
                                        deadline: std::time::Instant::now() + interval,
                                        repeat_every: message
                                            .get("repeat")
                                            .and_then(Value::as_bool)
                                            .unwrap_or(false)
                                            .then_some(interval),
                                    },
                                );
                            }
                        }
                        Some("cancel_timer") => {
                            if let Some(id) = message.get("timer_id").and_then(Value::as_str) {
                                self.timers.remove(id);
                            }
                        }
                        Some("schedule_render") => {
                            let delay = std::time::Duration::from_millis(
                                message
                                    .get("after_ms")
                                    .and_then(Value::as_u64)
                                    .unwrap_or(16),
                            );
                            self.frame_scheduler
                                .request_render_after(std::time::Instant::now(), delay);
                        }
                        Some("set_scheduler_mode") => {
                            self.frame_scheduler.set_mode(
                                message.get("mode").and_then(Value::as_str),
                                message.get("fps").and_then(Value::as_u64),
                                std::time::Instant::now(),
                            );
                        }
                        Some("frame_done") => {
                            self.perf_guest_frames += 1;
                            let completed_frame = message.get("frame_id").and_then(Value::as_u64);
                            let completed = completed_frame.and_then(|frame_id| {
                                commit_python_frame(
                                    &mut self.frame_scheduler,
                                    &mut self.pending_trees,
                                    &mut self.tree,
                                    frame_id,
                                )
                            });
                            if let Some(sent_at) = completed {
                                self.perf_guest_roundtrip += sent_at.elapsed();
                            } else {
                                log::warn!(
                                    "app::{}: CPython WASM completed unknown frame {:?}",
                                    self.app_id,
                                    completed_frame
                                );
                            }
                        }
                        Some("close") | Some("close_self") => self.wants_close = true,
                        Some("save_app_state") => self.save_state(message.get("payload")),
                        Some("file_read") => self.handle_file_read(&message),
                        Some("file_write") => self.handle_file_write(&message),
                        Some("http_request") => self.handle_http_request(&message),
                        Some("capability_request") => self.handle_capability_request(&message),
                        Some("log") => log::info!(
                            "app::{}: {}",
                            self.app_id,
                            message
                                .get("message")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                        ),
                        Some("status_summary") => {}
                        _ => {
                            if let Some(command) = app_command_from_python_message(&message) {
                                self.pending_commands.push(command);
                            } else {
                                log::warn!(
                                    "app::{}: unhandled CPython WASM message: {message}",
                                    self.app_id
                                );
                            }
                        }
                    }
                }
            }
            Err(error) => {
                log::error!("app::{}: drain CPython WASM messages: {error}", self.app_id);
                self.error = Some(error.to_string());
            }
        }
        let stderr = self.runtime.drain_stderr();
        if !stderr.trim().is_empty() {
            log::error!(
                "app::{} CPython WASM stderr: {}",
                self.app_id,
                stderr.trim()
            );
        }
    }

    fn has_capability(&self, capability: &str) -> bool {
        self.config
            .capabilities
            .iter()
            .any(|item| item == capability)
    }

    fn workspace_path(&self, raw: &str, for_write: bool) -> Result<PathBuf, String> {
        let path = Path::new(raw);
        if path.is_absolute()
            || path
                .components()
                .any(|part| matches!(part, std::path::Component::ParentDir))
        {
            return Err(format!("path escapes workspace: {raw}"));
        }
        let root = self.config.workspace_root.canonicalize().map_err(|error| {
            format!(
                "canonicalize workspace {}: {error}",
                self.config.workspace_root.display()
            )
        })?;
        let candidate = self.config.workspace_root.join(path);
        let resolved = if for_write && !candidate.exists() {
            let parent = candidate
                .parent()
                .ok_or_else(|| "path has no parent".to_string())?;
            parent
                .canonicalize()
                .map(|parent| parent.join(candidate.file_name().unwrap_or_default()))
        } else {
            candidate.canonicalize()
        }
        .map_err(|error| format!("resolve workspace path {raw}: {error}"))?;
        if !resolved.starts_with(&root) {
            return Err(format!("path escapes workspace through symlink: {raw}"));
        }
        Ok(resolved)
    }

    fn handle_file_read(&mut self, message: &Value) {
        let result = if !self.has_capability("fs.read") {
            Err("missing capability fs.read".to_string())
        } else {
            message
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| "missing path".to_string())
                .and_then(|path| self.workspace_path(path, false))
                .and_then(|path| {
                    std::fs::read_to_string(&path)
                        .map_err(|error| format!("read {}: {error}", path.display()))
                })
        };
        let response = match result {
            Ok(content) => json!({"type": "file_read_result", "content": content}),
            Err(error) => json!({"type": "file_read_result", "error": error}),
        };
        let _ = self.runtime.send(&response);
    }

    fn handle_file_write(&mut self, message: &Value) {
        let result = if !self.has_capability("fs.write") {
            Err("missing capability fs.write".to_string())
        } else {
            message
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| "missing path".to_string())
                .and_then(|path| self.workspace_path(path, true))
                .and_then(|path| {
                    let content = message
                        .get("content")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    std::fs::write(&path, content)
                        .map_err(|error| format!("write {}: {error}", path.display()))
                })
        };
        let response = match result {
            Ok(()) => json!({"type": "file_write_result"}),
            Err(error) => json!({"type": "file_write_result", "error": error}),
        };
        let _ = self.runtime.send(&response);
    }

    fn handle_http_request(&mut self, message: &Value) {
        let request_id = message
            .get("request_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if !self.has_capability("net.http") {
            let _ = self.runtime.send(&json!({"type": "http_response", "request_id": request_id, "error": "missing capability net.http"}));
            return;
        }
        let method = message
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or("GET")
            .to_string();
        let url = message
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if !http_host_allowed(&url, &self.config.allowed_hosts) {
            let _ = self.runtime.send(&json!({"type": "http_response", "request_id": request_id, "error": "host is not in manifest allowed_hosts"}));
            return;
        }
        let headers = message
            .get("headers")
            .and_then(Value::as_object)
            .map(|values| {
                values
                    .iter()
                    .filter_map(|(key, value)| {
                        value.as_str().map(|value| (key.clone(), value.to_string()))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let body = message
            .get("body")
            .and_then(Value::as_str)
            .map(str::to_string);
        let tx = self.http_tx.clone();
        std::thread::spawn(move || {
            use crate::host::services::NetService;
            let response = crate::host::services::UreqNetService::new().http(
                &method,
                &url,
                &headers,
                body.as_deref(),
            );
            if tx.send((request_id, response)).is_err() {
                log::debug!("CPython WASM HTTP response dropped after pane closed");
            }
        });
    }

    fn handle_capability_request(&mut self, message: &Value) {
        let capability = message
            .get("capability")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let granted = self.has_capability(capability);
        let _ = self.runtime.send(
            &json!({"type": "capability_decision", "capability": capability, "granted": granted}),
        );
    }

    fn save_state(&mut self, payload: Option<&Value>) {
        let Some(payload) = payload.and_then(Value::as_object) else {
            return;
        };
        self.persisted_state = payload.clone();
        let path = python_state_path(&self.config);
        if let Some(parent) = path.parent() {
            if let Err(error) = std::fs::create_dir_all(parent) {
                log::error!(
                    "app::{}: create state dir {}: {error}",
                    self.app_id,
                    parent.display()
                );
                return;
            }
        }
        match serde_json::to_vec_pretty(payload)
            .map_err(std::io::Error::other)
            .and_then(|bytes| std::fs::write(&path, bytes))
        {
            Ok(()) => log::info!("app::{}: persisted WASM Python state", self.app_id),
            Err(error) => log::error!(
                "app::{}: write state {}: {error}",
                self.app_id,
                path.display()
            ),
        }
    }

    fn fire_due_timers(&mut self) {
        let now = std::time::Instant::now();
        let due: Vec<String> = self
            .timers
            .iter()
            .filter(|(_, timer)| timer.deadline <= now)
            .map(|(id, _)| id.clone())
            .collect();
        for id in due {
            let repeats = if let Some(timer) = self.timers.get_mut(&id) {
                if let Some(interval) = timer.repeat_every {
                    timer.deadline = advance_fixed_deadline(timer.deadline, interval, now);
                    true
                } else {
                    false
                }
            } else {
                continue;
            };
            if !repeats {
                self.timers.remove(&id);
            }
            self.pending_timer_events.push(id);
            self.frame_scheduler.request_render_at(now);
        }
    }

    pub fn handle_key(
        &mut self,
        input: &crate::app::input_router::PlexiInput,
    ) -> crate::app::app_trait::KeyDisposition {
        let events = python_key_events(input.events());
        if events.is_empty() {
            crate::app::app_trait::KeyDisposition::Passthrough
        } else {
            let _ = self.runtime.send(&json!({
                "type": "key_events",
                "events": events,
            }));
            crate::app::app_trait::KeyDisposition::Consumed
        }
    }

    pub fn wants_close(&self) -> bool {
        self.wants_close
    }

    pub fn queue_outbound_event(&mut self, event: crate::app_protocol::PlexiEvent) {
        match encode_python_host_event(event) {
            Ok(value) => {
                if let Err(error) = self.runtime.send(&value) {
                    log::error!(
                        "app::{}: queue host event to Python runtime: {error}",
                        self.app_id
                    );
                    self.error = Some(error.to_string());
                }
            }
            Err(error) => {
                log::error!(
                    "app::{}: serialize host event for Python runtime: {error}",
                    self.app_id
                );
                self.error = Some(error.to_string());
            }
        }
    }

    pub(crate) fn tool_event_sender(&self) -> crate::host::wasm_python::AppendableStdin {
        self.runtime.stdin.clone()
    }
    pub fn take_pending_commands(&mut self) -> Vec<crate::app::app_trait::AppCommand> {
        std::mem::take(&mut self.pending_commands)
    }
    pub fn display_name(&self) -> String {
        self.title.clone().unwrap_or_else(|| self.app_id.clone())
    }

    pub(crate) fn semantic_state(&self) -> crate::host::pane::SemanticPaneState {
        python_semantic_state(self.tree.as_ref())
    }
    #[cfg(test)]
    pub fn has_rendered_tree(&self) -> bool {
        self.tree.is_some()
    }
    #[cfg(test)]
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    pub fn relaunch(&mut self) -> Result<(), WasmPythonError> {
        self.runtime = WasmPythonRuntime::launch(&self.config)?;
        self.tree = None;
        self.pending_trees.clear();
        self.initialized = false;
        self.ready = false;
        self.frame_scheduler.reset(std::time::Instant::now());
        self.error = None;
        self.wants_close = false;
        self.timers.clear();
        self.pending_timer_events.clear();
        self.viewport_size = None;
        log::info!("app::{}: relaunched CPython WASM runtime", self.app_id);
        Ok(())
    }
}

fn encode_python_host_event(
    event: crate::app_protocol::PlexiEvent,
) -> Result<Value, serde_json::Error> {
    serde_json::to_value(event)
}

fn python_semantic_state(tree: Option<&PythonUiTree>) -> crate::host::pane::SemanticPaneState {
    let Some(tree) = tree else {
        return crate::host::pane::SemanticPaneState::empty("python-wasm");
    };
    let mut state = crate::host::pane::SemanticPaneState::from_wasm_tree(&tree.tree);
    state.runtime_kind = "python-wasm".to_string();
    state.expose_canvas_commands(&tree.tree);
    for node in &mut state.nodes {
        let Ok(id) = node.id.parse::<u32>() else {
            continue;
        };
        if node.role == "canvas" {
            let fit = tree.canvas_fits.get(&id).copied().unwrap_or_default();
            let fit = match fit {
                super::wasm_render::CanvasFit::Fill => "fill",
                super::wasm_render::CanvasFit::Contain => "contain",
            };
            node.value = Some(format!(
                "{} fit={fit}",
                node.value.as_deref().unwrap_or("canvas")
            ));
        }
    }
    state
}

fn python_key_name(key: egui::Key) -> String {
    match key {
        egui::Key::ArrowDown => "down".to_string(),
        egui::Key::ArrowUp => "up".to_string(),
        egui::Key::ArrowLeft => "left".to_string(),
        egui::Key::ArrowRight => "right".to_string(),
        egui::Key::Enter => "enter".to_string(),
        egui::Key::Backspace => "backspace".to_string(),
        egui::Key::Escape => "escape".to_string(),
        egui::Key::Space => "space".to_string(),
        other => format!("{other:?}").to_ascii_lowercase(),
    }
}

fn python_key_events(events: &[egui::Event]) -> Vec<Value> {
    events
        .iter()
        .filter_map(|event| {
            let egui::Event::Key {
                key,
                pressed,
                modifiers,
                ..
            } = event
            else {
                return None;
            };
            Some(json!({
                "key": python_key_name(*key),
                "pressed": pressed,
                "modifiers": {
                    "ctrl": modifiers.ctrl,
                    "shift": modifiers.shift,
                    "alt": modifiers.alt,
                    "meta": modifiers.mac_cmd || modifiers.command,
                }
            }))
        })
        .collect()
}

fn scheduler_repaint_after(mode: Option<&str>, fps: Option<u64>) -> std::time::Duration {
    match mode {
        Some("continuous") => {
            let fps = fps.unwrap_or(60).clamp(1, 240);
            std::time::Duration::from_nanos(1_000_000_000 / fps)
        }
        _ => std::time::Duration::from_millis(16),
    }
}

fn valid_python_viewport(width: f32, height: f32) -> bool {
    width.is_finite() && height.is_finite() && width > 1.0 && height > 1.0
}

fn advance_fixed_deadline(
    deadline: std::time::Instant,
    interval: std::time::Duration,
    now: std::time::Instant,
) -> std::time::Instant {
    let mut next = deadline + interval;
    if next <= now {
        let missed = now.duration_since(next).as_nanos() / interval.as_nanos() + 1;
        let missed = u32::try_from(missed).unwrap_or(u32::MAX);
        next = next
            .checked_add(interval * missed)
            .unwrap_or(now + interval);
    }
    next
}

fn repaint_delay_until(
    deadline: std::time::Instant,
    now: std::time::Instant,
    predicted_frame: std::time::Duration,
) -> std::time::Duration {
    // egui starts a repaint one predicted frame before the requested delay.
    // Add that estimate back so a 60 Hz app does not turn a 16.7 ms deadline
    // into an immediate, unbounded host repaint loop.
    deadline.saturating_duration_since(now) + predicted_frame
}

fn python_render_event(frame_id: u64, timer_ids: Vec<String>) -> Value {
    json!({
        "type": "render",
        "frame_id": frame_id,
        "timer_ids": timer_ids,
    })
}

fn commit_python_frame(
    scheduler: &mut PythonFrameScheduler,
    pending_trees: &mut HashMap<u64, PythonUiTree>,
    visible_tree: &mut Option<PythonUiTree>,
    frame_id: u64,
) -> Option<std::time::Instant> {
    let sent_at = scheduler.complete_frame(frame_id)?;
    if let Some(tree) = pending_trees.remove(&frame_id) {
        *visible_tree = Some(tree);
    }
    Some(sent_at)
}

fn http_host_allowed(raw_url: &str, allowed_hosts: &[String]) -> bool {
    if allowed_hosts.is_empty() {
        return true;
    }
    let host = url::Url::parse(raw_url)
        .ok()
        .filter(|url| matches!(url.scheme(), "http" | "https"))
        .and_then(|url| {
            url.host_str()
                .map(|host| host.trim_end_matches('.').to_ascii_lowercase())
        });
    host.as_deref().is_some_and(|host| {
        allowed_hosts.iter().any(|pattern| {
            let pattern = pattern.trim().trim_end_matches('.').to_ascii_lowercase();
            host == pattern || host.ends_with(&format!(".{pattern}"))
        })
    })
}

fn app_command_from_python_message(message: &Value) -> Option<crate::app::app_trait::AppCommand> {
    use crate::app::app_trait::AppCommand;
    let text = |key: &str| {
        message
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    match message.get("type").and_then(Value::as_str)? {
        "expose_tools" => serde_json::from_value(message.get("tools")?.clone())
            .ok()
            .map(|tools| AppCommand::ExposeTools {
                tools,
                pane_id: None,
            }),
        "tool_result" => Some(AppCommand::ToolResult {
            call_id: text("call_id"),
            output_json: message
                .get("output_json")
                .and_then(Value::as_str)
                .map(str::to_string),
            error: message
                .get("error")
                .and_then(Value::as_str)
                .map(str::to_string),
        }),
        "declare_event_streams" => {
            let streams = message
                .get("streams")?
                .as_array()?
                .iter()
                .map(|stream| {
                    let schema_json = stream.get("schema_json")?.as_str()?;
                    Some(crate::app_protocol::EventStreamDecl {
                        name: stream.get("name")?.as_str()?.to_string(),
                        schema: serde_json::from_str(schema_json).ok()?,
                        description: stream
                            .get("description")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                    })
                })
                .collect::<Option<Vec<_>>>()?;
            Some(AppCommand::AppEventRequest {
                request: crate::app_protocol::AppRequest::DeclareEventStreams { streams },
                pane_id: None,
            })
        }
        "emit_event" => {
            let actor =
                serde_json::from_value(message.get("actor").cloned().unwrap_or(Value::Null))
                    .ok()?;
            let suggested_trigger = message
                .get("suggested_trigger")
                .filter(|value| !value.is_null())
                .cloned()
                .map(serde_json::from_value)
                .transpose()
                .ok()?;
            let payload = message
                .get("payload_json")
                .and_then(Value::as_str)
                .map(serde_json::from_str)
                .transpose()
                .ok()?;
            Some(AppCommand::AppEventRequest {
                request: crate::app_protocol::AppRequest::EmitEvent {
                    event: text("event"),
                    actor,
                    actor_id: message
                        .get("actor_id")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    caused_by: message
                        .get("caused_by")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    summary: text("summary"),
                    resource_id: text("resource_id"),
                    resource_scope: message
                        .get("resource_scope")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    revision_after: text("revision_after"),
                    payload,
                    state_ref: message
                        .get("state_ref")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    revision_before: message
                        .get("revision_before")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    rollback_token: message
                        .get("rollback_token")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    changed_resources: message
                        .get("changed_resources")
                        .and_then(Value::as_array)
                        .map(|values| {
                            values
                                .iter()
                                .filter_map(Value::as_str)
                                .map(str::to_string)
                                .collect()
                        })
                        .unwrap_or_default(),
                    suggested_trigger,
                },
                pane_id: None,
            })
        }
        "subscribe_event_streams" => Some(AppCommand::AppEventRequest {
            request: crate::app_protocol::AppRequest::SubscribeAppEvents {
                request_id: text("request_id"),
                app_id: text("app_id"),
                event_names: message
                    .get("event_names")
                    .and_then(Value::as_array)
                    .map(|values| {
                        values
                            .iter()
                            .filter_map(Value::as_str)
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_default(),
                payload_mode: serde_json::from_value(message.get("payload_mode")?.clone()).ok()?,
                trigger_mode: serde_json::from_value(message.get("trigger_mode")?.clone()).ok()?,
                resource_id: message
                    .get("resource_id")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            },
            pane_id: None,
        }),
        "unsubscribe_event_streams" => Some(AppCommand::AppEventRequest {
            request: crate::app_protocol::AppRequest::UnsubscribeAppEvents {
                request_id: text("request_id"),
                subscription_id: text("subscription_id"),
            },
            pane_id: None,
        }),
        "notify" => Some(AppCommand::Notify(text("message"))),
        "spawn_app" => Some(AppCommand::SpawnApp {
            type_id: text("app_id"),
            layout: message
                .get("layout")
                .and_then(Value::as_str)
                .map(str::to_string),
            args: Vec::new(),
        }),
        "spawn_pane" => Some(AppCommand::SpawnPane {
            type_id: text("app_id"),
            layout: text("layout"),
            args: Vec::new(),
            from_pane_id: None,
            request_id: None,
            target_context: None,
        }),
        "focus_pane" => Some(AppCommand::ForwardPaneRequest {
            request: crate::app_protocol::AppRequest::FocusPane {
                pane_id: message
                    .get("pane_id")
                    .and_then(Value::as_u64)
                    .unwrap_or_default(),
            },
        }),
        "pipe_send" => Some(AppCommand::DeliverPipeMessage {
            sender_pane_id: 0,
            pipe_id: text("pipe_id"),
            payload: message.get("payload").cloned().unwrap_or(Value::Null),
        }),
        "pipe_open_directed" => Some(AppCommand::OpenDirectedPipe {
            sender_pane_id: 0,
            pipe_id: text("pipe_id"),
            target_pane_id: message
                .get("target_pane_id")
                .and_then(Value::as_u64)
                .unwrap_or_default(),
        }),
        "run_update" => Some(AppCommand::DeliverRunUpdate {
            originator_type_id: text("originator_type_id"),
            event: crate::app_protocol::PlexiEvent::Resume,
        }),
        "show_notification" => Some(AppCommand::ShowNotification {
            notify_id: text("notify_id"),
            sender_pane_id: 0,
            source_context_id: 0,
            level: text("level"),
            title: text("title"),
            body: text("body"),
            kind: crate::app_protocol::NotifyKind::Message,
            options: Vec::new(),
            input_prompt: None,
            required: false,
            priority: 0,
            scope: crate::app_protocol::NotifyScope::Global,
            image_inline: None,
            image_pipe_id: None,
            timeout_secs: None,
            on_dismiss: None,
        }),
        "insert_path_token" => Some(AppCommand::InsertPathToken {
            sender_pane_id: 0,
            terminal_pane_id: message
                .get("terminal_pane_id")
                .and_then(Value::as_u64)
                .unwrap_or_default(),
            path: text("path"),
            mode: crate::app_protocol::PathTokenMode::Append,
        }),
        "command_preview" => Some(AppCommand::RequestCommandPreview {
            sender_pane_id: 0,
            request_id: text("request_id"),
            terminal_pane_id: message
                .get("terminal_pane_id")
                .and_then(Value::as_u64)
                .unwrap_or_default(),
            command: text("command"),
        }),
        "query_context_state" => Some(AppCommand::QueryContextState {
            sender_pane_id: 0,
            context_id: message
                .get("context_id")
                .and_then(Value::as_u64)
                .unwrap_or_default(),
        }),
        _ => None,
    }
}

pub fn resolve_default_cpython_bundle() -> Result<PathBuf, WasmPythonError> {
    let cache_dir = std::env::var_os(CPYTHON_BUNDLE_CACHE_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(shared_wasm_bundle_dir);
    match resolve_cpython_bundle(cache_dir.clone()) {
        Ok(path) => Ok(path),
        Err(WasmPythonError::MissingBundle { .. }) => {
            let script =
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/fetch-cpython-bundle.sh");
            log::info!("python_compat: fetching verified CPython WASI bundle");
            let status = Command::new("bash")
                .arg(script)
                .env(CPYTHON_BUNDLE_CACHE_ENV, &cache_dir)
                .status()
                .map_err(|source| WasmPythonError::ReadBundle {
                    path: cache_dir.clone(),
                    source,
                })?;
            if !status.success() {
                return Err(WasmPythonError::RuntimeStart(
                    "fetch verified CPython WASI bundle failed".to_string(),
                ));
            }
            resolve_cpython_bundle(cache_dir)
        }
        Err(error) => Err(error),
    }
}

fn shared_wasm_bundle_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".plexi/wasm-bundles")
}

pub fn resolve_cpython_bundle(cache_dir: PathBuf) -> Result<PathBuf, WasmPythonError> {
    let path = cache_dir.join(CPYTHON_BUNDLE_FILE);
    log::info!(
        "python_compat: resolving CPython WASI bundle version={} cache_dir={} path={}",
        CPYTHON_BUNDLE_VERSION,
        cache_dir.display(),
        path.display()
    );
    if !path.is_file() {
        return Err(WasmPythonError::MissingBundle {
            path,
            command: FETCH_CPYTHON_BUNDLE_COMMAND,
        });
    }
    if CPYTHON_BUNDLE_SHA256.len() != 64 {
        return Err(WasmPythonError::BundleHashUnpinned {
            version: CPYTHON_BUNDLE_VERSION,
            command: FETCH_CPYTHON_BUNDLE_COMMAND,
        });
    }
    let bytes = std::fs::read(&path).map_err(|source| WasmPythonError::ReadBundle {
        path: path.clone(),
        source,
    })?;
    let actual = format!("{:x}", Sha256::digest(&bytes));
    if actual != CPYTHON_BUNDLE_SHA256 {
        return Err(WasmPythonError::BundleHashMismatch {
            path,
            expected: CPYTHON_BUNDLE_SHA256,
            actual,
        });
    }
    Ok(path)
}

#[cfg(test)]
pub fn resolve_cpython_shim_component(cache_dir: PathBuf) -> Result<PathBuf, WasmPythonError> {
    resolve_cpython_shim_component_path(cache_dir.join(CPYTHON_SHIM_COMPONENT_FILE))
}

#[cfg(test)]
fn resolve_cpython_shim_component_path(path: PathBuf) -> Result<PathBuf, WasmPythonError> {
    log::info!(
        "python_compat: resolving CPython lifecycle shim component path={}",
        path.display()
    );
    if !path.is_file() {
        return Err(WasmPythonError::MissingShimComponent {
            path,
            command: BUILD_CPYTHON_SHIM_COMMAND,
        });
    }
    Ok(path)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg(test)]
pub enum CpythonBundleAbi {
    RawWasiModule,
    LifecycleComponent,
}

#[cfg(test)]
pub fn inspect_cpython_bundle_abi(path: &Path) -> Result<CpythonBundleAbi, WasmPythonError> {
    log::info!(
        "python_compat: inspecting CPython WASI bundle abi version={} wasi_sdk={} path={}",
        CPYTHON_BUNDLE_VERSION,
        CPYTHON_WASI_SDK_VERSION,
        path.display()
    );
    if probe_lifecycle_component(path, "CPython bundle ABI probe").is_ok() {
        return Ok(CpythonBundleAbi::LifecycleComponent);
    }
    if is_core_wasm_module(path) {
        return Ok(CpythonBundleAbi::RawWasiModule);
    }
    Err(WasmPythonError::RawModuleAbiMismatch {
        path: path.to_path_buf(),
        reason: "artifact is neither a raw WASM module nor a lifecycle component accepted by Plexi"
            .to_string(),
    })
}

#[cfg(test)]
fn validate_cpython_bundle_abi(path: &Path) -> Result<(), WasmPythonError> {
    match inspect_cpython_bundle_abi(path)? {
        CpythonBundleAbi::LifecycleComponent => Ok(()),
        CpythonBundleAbi::RawWasiModule => Err(WasmPythonError::RawModuleAbiMismatch {
            path: path.to_path_buf(),
            reason: "upstream CPython artifact is a WASI command/runtime, not a component exporting plexi:app/lifecycle init/update/view; Plexi needs a shim component that embeds CPython and exports lifecycle".to_string(),
        }),
    }
}

#[cfg(test)]
pub fn probe_cpython_shim_component(path: &Path) -> Result<(), WasmPythonError> {
    probe_lifecycle_component(path, "Python Shim POC")
}

#[cfg(test)]
fn probe_lifecycle_component(path: &Path, expected_view_text: &str) -> Result<(), WasmPythonError> {
    let grants = WasmApp::inspect_required_grants(path)
        .map_err(|source| classify_component_load_error(path, source))?;
    let mut app = WasmApp::load_with_grants(
        "python-shim-probe",
        path,
        StateStore::ephemeral(),
        grants_with_state(grants),
    )
    .map_err(|source| classify_component_load_error(path, source))?;
    let snapshot = StateSnapshot {
        entries: Vec::new(),
    };
    let effects = app.init(&snapshot, (320.0, 240.0), &[]).map_err(|source| {
        WasmPythonError::ShimLifecycleCallFailure {
            path: path.to_path_buf(),
            function: "init",
            message: source.to_string(),
        }
    })?;
    if effects.is_empty() {
        return Err(WasmPythonError::ShimLifecycleCallFailure {
            path: path.to_path_buf(),
            function: "init",
            message: "shim init returned no effects".to_string(),
        });
    }
    let tree = app
        .view()
        .map_err(|source| WasmPythonError::ShimLifecycleCallFailure {
            path: path.to_path_buf(),
            function: "view",
            message: source.to_string(),
        })?;
    if !ui_tree_contains_text(&tree, expected_view_text) {
        return Err(WasmPythonError::ShimLifecycleCallFailure {
            path: path.to_path_buf(),
            function: "view",
            message: format!("view did not contain '{expected_view_text}'"),
        });
    }
    Ok(())
}

#[cfg(test)]
fn classify_component_load_error(path: &Path, source: wasmtime::Error) -> WasmPythonError {
    if is_core_wasm_module(path) {
        return WasmPythonError::RawModuleAbiMismatch {
            path: path.to_path_buf(),
            reason:
                "artifact is a raw WASM module; expected a component exporting plexi:app/lifecycle"
                    .to_string(),
        };
    }
    WasmPythonError::ShimComponentLoadFailure {
        path: path.to_path_buf(),
        message: source.to_string(),
    }
}

#[cfg(test)]
fn is_core_wasm_module(path: &Path) -> bool {
    wasmtime::Module::from_file(&wasmtime::Engine::default(), path).is_ok()
}

#[cfg(test)]
fn grants_with_state(mut grants: Grants) -> Grants {
    grants.state = true;
    grants
}

#[cfg(test)]
fn ui_tree_contains_text(tree: &UiTree, needle: &str) -> bool {
    tree.nodes.iter().any(|node| {
        matches!(
            &node.data,
            UiNodeData::Text(text) if text.text.contains(needle)
        )
    })
}

#[derive(Debug, Clone)]
#[cfg(test)]
pub enum PythonBridgeEffect {
    Host(Effect),
    SetState(Vec<(String, Vec<u8>)>),
}

#[cfg(test)]
pub fn init_bridge_arg(snapshot: &StateSnapshot, size: (f32, f32), args: &[String]) -> Value {
    json!({
        "state": encode_state(snapshot),
        "size": [size.0, size.1],
        "args": args,
    })
}

#[cfg(test)]
pub fn update_bridge_arg(
    snapshot: &StateSnapshot,
    event: &InputEvent,
) -> Result<Value, WasmPythonError> {
    Ok(json!({
        "state": encode_state(snapshot),
        "event": encode_input_event(event)?,
    }))
}

#[cfg(test)]
pub fn view_bridge_arg(snapshot: &StateSnapshot) -> Value {
    json!({ "state": encode_state(snapshot) })
}

#[cfg(test)]
pub fn encode_state(snapshot: &StateSnapshot) -> Value {
    let mut out = serde_json::Map::new();
    for (key, bytes) in &snapshot.entries {
        let encoded = if serde_json::from_slice::<Value>(bytes).is_ok() {
            BASE64.encode(bytes)
        } else {
            format!("b64:{}", BASE64.encode(bytes))
        };
        out.insert(key.clone(), Value::String(encoded));
    }
    Value::Object(out)
}

#[cfg(test)]
pub fn encode_input_event(event: &InputEvent) -> Result<Value, WasmPythonError> {
    let value = match event {
        InputEvent::Key(KeyEvent {
            key,
            modifiers,
            pressed,
        }) => json!({
            "type": "KeyEvent",
            "key": key,
            "modifiers": {
                "ctrl": modifiers.ctrl,
                "shift": modifiers.shift,
                "alt": modifiers.alt,
                "meta": modifiers.meta,
            },
            "pressed": pressed,
        }),
        InputEvent::UiAction(UiActionEvent { handler_id }) => {
            json!({ "type": "UiAction", "handler_id": handler_id })
        }
        InputEvent::UiValueChange(UiValueChangeEvent { handler_id, value }) => {
            json!({ "type": "UiValueChange", "handler_id": handler_id, "value": value })
        }
        InputEvent::Resize(size) => {
            json!({ "type": "Resize", "width": size.width, "height": size.height })
        }
        InputEvent::FocusGained => json!({ "type": "FocusGained" }),
        InputEvent::FocusLost => json!({ "type": "FocusLost" }),
        InputEvent::TimerFired(id) => json!({ "type": "TimerFired", "id": id }),
        InputEvent::HttpResponse(response) => json!({
            "type": "HttpResponse",
            "status": response.status,
            "headers": response.headers,
            "body": response.body,
        }),
        InputEvent::CapabilityGranted(name) => {
            json!({ "type": "CapabilityGranted", "name": name })
        }
        InputEvent::CapabilityDenied(name) => json!({ "type": "CapabilityDenied", "name": name }),
        other => {
            return Err(WasmPythonError::BridgeJson(format!(
                "input event not yet supported by Python bridge: {other:?}"
            )));
        }
    };
    Ok(value)
}

#[cfg(test)]
pub fn decode_effects(json_text: &str) -> Result<Vec<PythonBridgeEffect>, WasmPythonError> {
    let values = serde_json::from_str::<Vec<Value>>(json_text)
        .map_err(|e| WasmPythonError::BridgeJson(e.to_string()))?;
    values.into_iter().map(decode_effect).collect()
}

#[cfg(test)]
fn decode_effect(value: Value) -> Result<PythonBridgeEffect, WasmPythonError> {
    let kind = value
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| WasmPythonError::BridgeJson("effect missing string 'type'".to_string()))?;
    match kind {
        "SetState" | "PersistState" => decode_set_state(value).map(PythonBridgeEffect::SetState),
        "SetTitle" => Ok(PythonBridgeEffect::Host(Effect::SetTitle(required_string(
            &value, "title",
        )?))),
        "SetStatus" => Ok(PythonBridgeEffect::Host(Effect::SetStatus(
            required_string(&value, "text")?,
        ))),
        "CloseSelf" => Ok(PythonBridgeEffect::Host(Effect::CloseSelf)),
        "RequestCapability" => Ok(PythonBridgeEffect::Host(Effect::RequestCapability(
            required_string(&value, "name")?,
        ))),
        "SetTimer" => Ok(PythonBridgeEffect::Host(Effect::SetTimer(TimerEffect {
            id: required_u32(&value, "id")?,
            delay_ms: required_u32(&value, "delay_ms")?,
            repeat: value
                .get("repeat")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        }))),
        "CancelTimer" => Ok(PythonBridgeEffect::Host(Effect::CancelTimer(required_u32(
            &value, "id",
        )?))),
        "GetSystemStats" => Ok(PythonBridgeEffect::Host(Effect::GetSystemStats)),
        "FileRead" => Ok(PythonBridgeEffect::Host(Effect::FileRead(FileReadEffect {
            path: required_string(&value, "path")?,
        }))),
        "FileWrite" => Ok(PythonBridgeEffect::Host(Effect::FileWrite(
            FileWriteEffect {
                path: required_string(&value, "path")?,
                content: bytes_field(&value, "content")?,
            },
        ))),
        "HttpFetch" => Ok(PythonBridgeEffect::Host(Effect::HttpFetch(
            HttpFetchEffect {
                url: required_string(&value, "url")?,
                method: value
                    .get("method")
                    .and_then(Value::as_str)
                    .unwrap_or("GET")
                    .to_string(),
                headers: headers_field(&value, "headers")?,
                body: optional_bytes_field(&value, "body")?,
            },
        ))),
        other => Err(WasmPythonError::BridgeJson(format!(
            "Unknown effect type: {other}"
        ))),
    }
}

#[cfg(test)]
fn decode_set_state(value: Value) -> Result<Vec<(String, Vec<u8>)>, WasmPythonError> {
    let data = value
        .get("data")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            WasmPythonError::BridgeJson("SetState.data must be an object".to_string())
        })?;
    data.iter()
        .map(|(key, value)| {
            serde_json::to_vec(value)
                .map(|bytes| (key.clone(), bytes))
                .map_err(|e| WasmPythonError::BridgeJson(e.to_string()))
        })
        .collect()
}

#[cfg(test)]
pub fn decode_ui_tree(json_text: &str) -> Result<UiTree, WasmPythonError> {
    let value = serde_json::from_str::<Value>(json_text)
        .map_err(|e| WasmPythonError::BridgeJson(e.to_string()))?;
    decode_ui_tree_value(&value)
}

fn decode_ui_tree_value(value: &Value) -> Result<UiTree, WasmPythonError> {
    let root = required_u32(value, "root")?;
    let nodes = value
        .get("nodes")
        .and_then(Value::as_array)
        .ok_or_else(|| WasmPythonError::BridgeJson("ui tree missing nodes array".to_string()))?
        .iter()
        .map(decode_indexed_node)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(UiTree { root, nodes })
}

fn decode_python_ui_tree_value(value: &Value) -> Result<PythonUiTree, WasmPythonError> {
    let tree = decode_ui_tree_value(value)?;
    let mut canvas_fits = HashMap::new();
    for node in value
        .get("nodes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(data) = node.get("data") else {
            continue;
        };
        if !matches!(
            data.get("type").and_then(Value::as_str),
            Some("Canvas" | "canvas")
        ) {
            continue;
        }
        let id = required_u32(node, "id")?;
        let fit = match data.get("fit").and_then(Value::as_str).unwrap_or("fill") {
            "fill" => super::wasm_render::CanvasFit::Fill,
            "contain" => super::wasm_render::CanvasFit::Contain,
            other => {
                return Err(WasmPythonError::BridgeJson(format!(
                    "canvas fit must be 'fill' or 'contain', got {other:?}"
                )))
            }
        };
        canvas_fits.insert(id, fit);
    }
    Ok(PythonUiTree { tree, canvas_fits })
}

#[cfg(test)]
#[derive(Debug)]
struct NativePythonLifecycleOutput {
    init_json: String,
    update_json: String,
    view_json: String,
}

#[cfg(test)]
fn run_native_python_lifecycle_probe(
    sdk_dir: &Path,
    app_dir: &Path,
    module_name: &str,
    init_arg: &Value,
    update_arg: &Value,
    view_arg: &Value,
) -> Result<NativePythonLifecycleOutput, WasmPythonError> {
    let script = r#"
import json
import sys

sdk_dir, app_dir, module_name, init_arg, update_arg, view_arg = sys.argv[1:]
sys.path.insert(0, sdk_dir)
sys.path.insert(0, app_dir)

import plexi_sdk._v3_state as v3_state
from plexi_sdk._adapter import call_lifecycle, load_app

v3_state._host_log = lambda level, msg: None

load_app(module_name)
print(json.dumps({
    "init": call_lifecycle("init", init_arg),
    "update": call_lifecycle("update", update_arg),
    "view": call_lifecycle("view", view_arg),
}))
"#;
    let output = Command::new("python3")
        .arg("-c")
        .arg(script)
        .arg(sdk_dir)
        .arg(app_dir)
        .arg(module_name)
        .arg(init_arg.to_string())
        .arg(update_arg.to_string())
        .arg(view_arg.to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|source| WasmPythonError::ReadBundle {
            path: PathBuf::from("python3"),
            source,
        })?;
    if !output.status.success() {
        return Err(WasmPythonError::BridgeJson(format!(
            "native Python bridge probe failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    let value = serde_json::from_slice::<Value>(&output.stdout)
        .map_err(|e| WasmPythonError::BridgeJson(e.to_string()))?;
    Ok(NativePythonLifecycleOutput {
        init_json: required_string(&value, "init")?,
        update_json: required_string(&value, "update")?,
        view_json: required_string(&value, "view")?,
    })
}

fn decode_indexed_node(value: &Value) -> Result<IndexedNode, WasmPythonError> {
    let data = value
        .get("data")
        .ok_or_else(|| WasmPythonError::BridgeJson("indexed node missing data".to_string()))?;
    Ok(IndexedNode {
        id: required_u32(value, "id")?,
        key: required_string(value, "key")?,
        data: decode_node_data(data)?,
    })
}

fn decode_node_data(value: &Value) -> Result<UiNodeData, WasmPythonError> {
    let kind = value
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| WasmPythonError::BridgeJson("ui node missing string 'type'".to_string()))?;
    match kind {
        "Empty" => Ok(UiNodeData::Empty),
        "Text" | "text" | "label" => Ok(UiNodeData::Text(TextNode {
            text: required_string(value, "text")?,
            size: optional_f32(value, "size")?,
            bold: value.get("bold").and_then(Value::as_bool).unwrap_or(false),
            color: None,
            truncate: value
                .get("truncate")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            align: decode_alignment(
                value
                    .get("align")
                    .and_then(Value::as_str)
                    .unwrap_or("start"),
            )?,
        })),
        "AppBar" | "app_bar" | "app-bar" => Ok(UiNodeData::Text(TextNode {
            text: match value.get("subtitle").and_then(Value::as_str) {
                Some("") | None => required_string(value, "title")?,
                Some(subtitle) => format!("{} {}", required_string(value, "title")?, subtitle),
            },
            size: Some(16.0),
            bold: true,
            color: None,
            truncate: true,
            align: Alignment::Start,
        })),
        "Column" | "column" => Ok(UiNodeData::Column(ColumnNode {
            children: u32_list(value, "children")?,
            gap: optional_f32(value, "gap")?.unwrap_or(0.0),
            align: decode_alignment(
                value
                    .get("align")
                    .and_then(Value::as_str)
                    .unwrap_or("start"),
            )?,
            grow: value.get("grow").and_then(Value::as_bool).unwrap_or(false),
        })),
        "Button" | "button" => Ok(UiNodeData::Button(ButtonNode {
            label: required_string(value, "label")?,
            on_click: required_string(value, "on_click")?,
            style: decode_button_style(
                value
                    .get("style")
                    .and_then(Value::as_str)
                    .unwrap_or("secondary"),
            )?,
            disabled: value
                .get("disabled")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        })),
        "TextInput" | "text_input" | "text-input" => Ok(UiNodeData::TextInput(TextInputNode {
            value: value
                .get("value")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            placeholder: value
                .get("placeholder")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            on_change: value
                .get("on_change")
                .or_else(|| value.get("on-change"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            on_submit: value
                .get("on_submit")
                .or_else(|| value.get("on-submit"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            password: value
                .get("password")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        })),
        "Row" | "row" => Ok(UiNodeData::Row(RowNode {
            children: u32_list(value, "children")?,
            gap: optional_f32(value, "gap")?.unwrap_or(0.0),
            align: decode_alignment(
                value
                    .get("align")
                    .and_then(Value::as_str)
                    .unwrap_or("start"),
            )?,
            grow: value.get("grow").and_then(Value::as_bool).unwrap_or(false),
        })),
        "Divider" | "divider" => Ok(UiNodeData::Divider),
        "Space" | "spacer" => Ok(UiNodeData::Space(
            optional_f32(value, "size")?.unwrap_or(0.0),
        )),
        "ProgressBar" | "progress_bar" | "progress-bar" => {
            Ok(UiNodeData::ProgressBar(ProgressBarNode {
                value: optional_f32(value, "value")?.unwrap_or(0.0),
                max: optional_f32(value, "max")?.unwrap_or(1.0),
                color: None,
                label: value
                    .get("label")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            }))
        }
        "Badge" | "badge" => Ok(UiNodeData::Badge(
            super::wasm_app::bindings::plexi::platform::types::BadgeNode {
                text: required_string(value, "text")?,
                color: decode_badge_color(
                    value
                        .get("color")
                        .and_then(Value::as_str)
                        .unwrap_or("neutral"),
                )?,
            },
        )),
        "ListView" | "list_view" | "list-view" => Ok(UiNodeData::ListView(ListNode {
            items: u32_list(value, "items")?,
            selected: value
                .get("selected")
                .and_then(Value::as_u64)
                .map(u32::try_from)
                .transpose()
                .map_err(|_| {
                    WasmPythonError::BridgeJson("field 'selected' out of range".to_string())
                })?,
            on_select: value
                .get("on_select")
                .or_else(|| value.get("on-select"))
                .and_then(Value::as_str)
                .map(str::to_string),
        })),
        "Scroll" | "scroll" => Ok(UiNodeData::Scroll(ScrollNode {
            child: required_u32(value, "child")?,
            horizontal: value
                .get("horizontal")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        })),
        "Padding" | "padding" => Ok(UiNodeData::Padding(PaddingNode {
            child: required_u32(value, "child")?,
            top: optional_f32(value, "top")?.unwrap_or(0.0),
            right: optional_f32(value, "right")?.unwrap_or(0.0),
            bottom: optional_f32(value, "bottom")?.unwrap_or(0.0),
            left: optional_f32(value, "left")?.unwrap_or(0.0),
        })),
        "Canvas" | "canvas" => Ok(UiNodeData::Canvas(CanvasNode {
            width: optional_f32(value, "width")?.unwrap_or(640.0),
            height: optional_f32(value, "height")?.unwrap_or(360.0),
            grow: value.get("grow").and_then(Value::as_bool).unwrap_or(true),
            commands: canvas_commands(value, "commands")?,
        })),
        other => Err(WasmPythonError::BridgeJson(format!(
            "unsupported CPython-WASM UINode type: {other}"
        ))),
    }
}

fn canvas_commands(value: &Value, field: &str) -> Result<Vec<CanvasCommand>, WasmPythonError> {
    value
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| WasmPythonError::BridgeJson(format!("missing array field '{field}'")))?
        .iter()
        .map(decode_canvas_command)
        .collect()
}

fn decode_canvas_command(value: &Value) -> Result<CanvasCommand, WasmPythonError> {
    let kind = value.get("type").and_then(Value::as_str).ok_or_else(|| {
        WasmPythonError::BridgeJson("canvas command missing string 'type'".to_string())
    })?;
    match kind {
        "rect" | "Rect" => Ok(CanvasCommand::Rect(CanvasRect {
            x: optional_f32(value, "x")?.unwrap_or(0.0),
            y: optional_f32(value, "y")?.unwrap_or(0.0),
            width: optional_f32(value, "width")?
                .or(optional_f32(value, "w")?)
                .unwrap_or(0.0),
            height: optional_f32(value, "height")?
                .or(optional_f32(value, "h")?)
                .unwrap_or(0.0),
            fill: decode_color_field(value, "fill")?,
            radius: optional_f32(value, "radius")?.unwrap_or(0.0),
        })),
        "circle" | "Circle" => Ok(CanvasCommand::Circle(CanvasCircle {
            x: optional_f32(value, "cx")?
                .or(optional_f32(value, "x")?)
                .unwrap_or(0.0),
            y: optional_f32(value, "cy")?
                .or(optional_f32(value, "y")?)
                .unwrap_or(0.0),
            radius: optional_f32(value, "radius")?
                .or(optional_f32(value, "r")?)
                .unwrap_or(0.0),
            fill: decode_color_field(value, "fill")?,
        })),
        "line" | "Line" => Ok(CanvasCommand::Line(CanvasLine {
            x1: optional_f32(value, "x1")?.unwrap_or(0.0),
            y1: optional_f32(value, "y1")?.unwrap_or(0.0),
            x2: optional_f32(value, "x2")?.unwrap_or(0.0),
            y2: optional_f32(value, "y2")?.unwrap_or(0.0),
            width: optional_f32(value, "width")?.unwrap_or(1.0),
            color: decode_color_field(value, "color")?,
        })),
        "text" | "Text" => Ok(CanvasCommand::Text(CanvasText {
            x: optional_f32(value, "x")?.unwrap_or(0.0),
            y: optional_f32(value, "y")?.unwrap_or(0.0),
            text: required_string(value, "text")?,
            size: optional_f32(value, "size")?.unwrap_or(14.0),
            color: decode_color_field(value, "color")?,
            bold: value.get("bold").and_then(Value::as_bool).unwrap_or(false),
            align: decode_alignment(
                value
                    .get("align")
                    .and_then(Value::as_str)
                    .unwrap_or("start"),
            )?,
        })),
        other => Err(WasmPythonError::BridgeJson(format!(
            "unknown canvas command type: {other}"
        ))),
    }
}

fn decode_color_field(value: &Value, field: &str) -> Result<Color, WasmPythonError> {
    let Some(raw) = value.get(field) else {
        return Err(WasmPythonError::BridgeJson(format!(
            "missing color field '{field}'"
        )));
    };
    decode_color(raw)
}

fn decode_color(value: &Value) -> Result<Color, WasmPythonError> {
    if let Some(hex) = value.as_str() {
        return decode_hex_color(hex);
    }
    let r = required_u8(value, "r")?;
    let g = required_u8(value, "g")?;
    let b = required_u8(value, "b")?;
    let a = value
        .get("a")
        .map(|_| required_u8(value, "a"))
        .transpose()?
        .unwrap_or(255);
    Ok(Color { r, g, b, a })
}

fn decode_hex_color(hex: &str) -> Result<Color, WasmPythonError> {
    let value = hex.strip_prefix('#').unwrap_or(hex);
    let parse = |s: &str| {
        u8::from_str_radix(s, 16)
            .map_err(|e| WasmPythonError::BridgeJson(format!("invalid color '{hex}': {e}")))
    };
    match value.len() {
        6 => Ok(Color {
            r: parse(&value[0..2])?,
            g: parse(&value[2..4])?,
            b: parse(&value[4..6])?,
            a: 255,
        }),
        8 => Ok(Color {
            r: parse(&value[0..2])?,
            g: parse(&value[2..4])?,
            b: parse(&value[4..6])?,
            a: parse(&value[6..8])?,
        }),
        _ => Err(WasmPythonError::BridgeJson(format!(
            "invalid color '{hex}': expected #rrggbb or #rrggbbaa"
        ))),
    }
}

fn required_string(value: &Value, field: &str) -> Result<String, WasmPythonError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| WasmPythonError::BridgeJson(format!("missing string field '{field}'")))
}

fn required_u32(value: &Value, field: &str) -> Result<u32, WasmPythonError> {
    let n = value
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| WasmPythonError::BridgeJson(format!("missing u32 field '{field}'")))?;
    u32::try_from(n)
        .map_err(|_| WasmPythonError::BridgeJson(format!("field '{field}' out of u32 range")))
}

fn required_u8(value: &Value, field: &str) -> Result<u8, WasmPythonError> {
    let n = value
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| WasmPythonError::BridgeJson(format!("missing u8 field '{field}'")))?;
    u8::try_from(n)
        .map_err(|_| WasmPythonError::BridgeJson(format!("field '{field}' out of u8 range")))
}

fn optional_f32(value: &Value, field: &str) -> Result<Option<f32>, WasmPythonError> {
    match value.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_f64()
            .map(|n| n as f32)
            .ok_or_else(|| WasmPythonError::BridgeJson(format!("field '{field}' must be a number")))
            .map(Some),
    }
}

#[cfg(test)]
fn bytes_field(value: &Value, field: &str) -> Result<Vec<u8>, WasmPythonError> {
    match value.get(field) {
        Some(Value::String(s)) => Ok(s.as_bytes().to_vec()),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| {
                let n = item.as_u64().ok_or_else(|| {
                    WasmPythonError::BridgeJson(format!(
                        "field '{field}' byte array contains non-u64"
                    ))
                })?;
                u8::try_from(n).map_err(|_| {
                    WasmPythonError::BridgeJson(format!("field '{field}' byte out of range"))
                })
            })
            .collect(),
        _ => Err(WasmPythonError::BridgeJson(format!(
            "missing bytes field '{field}'"
        ))),
    }
}

#[cfg(test)]
fn optional_bytes_field(value: &Value, field: &str) -> Result<Option<Vec<u8>>, WasmPythonError> {
    if matches!(value.get(field), None | Some(Value::Null)) {
        return Ok(None);
    }
    bytes_field(value, field).map(Some)
}

#[cfg(test)]
fn headers_field(value: &Value, field: &str) -> Result<Vec<(String, String)>, WasmPythonError> {
    match value.get(field) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Object(map)) => Ok(map
            .iter()
            .map(|(key, value)| {
                value
                    .as_str()
                    .map(|text| (key.clone(), text.to_string()))
                    .ok_or_else(|| {
                        WasmPythonError::BridgeJson(format!(
                            "field '{field}' object values must be strings"
                        ))
                    })
            })
            .collect::<Result<Vec<_>, _>>()?),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| {
                let pair = item.as_array().ok_or_else(|| {
                    WasmPythonError::BridgeJson(format!(
                        "field '{field}' header entry must be an array"
                    ))
                })?;
                if pair.len() != 2 {
                    return Err(WasmPythonError::BridgeJson(format!(
                        "field '{field}' header entry must have two items"
                    )));
                }
                let key = pair[0].as_str().ok_or_else(|| {
                    WasmPythonError::BridgeJson(format!(
                        "field '{field}' header name must be a string"
                    ))
                })?;
                let val = pair[1].as_str().ok_or_else(|| {
                    WasmPythonError::BridgeJson(format!(
                        "field '{field}' header value must be a string"
                    ))
                })?;
                Ok((key.to_string(), val.to_string()))
            })
            .collect(),
        _ => Err(WasmPythonError::BridgeJson(format!(
            "field '{field}' must be an object or array"
        ))),
    }
}

fn u32_list(value: &Value, field: &str) -> Result<Vec<u32>, WasmPythonError> {
    value
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| WasmPythonError::BridgeJson(format!("missing array field '{field}'")))?
        .iter()
        .map(|item| {
            let n = item.as_u64().ok_or_else(|| {
                WasmPythonError::BridgeJson(format!("field '{field}' contains non-u64"))
            })?;
            u32::try_from(n)
                .map_err(|_| WasmPythonError::BridgeJson(format!("field '{field}' out of range")))
        })
        .collect()
}

fn decode_alignment(value: &str) -> Result<Alignment, WasmPythonError> {
    match value {
        "start" | "left_top" | "left_center" | "left_bottom" => Ok(Alignment::Start),
        "center" | "center_top" | "center_center" | "center_bottom" => Ok(Alignment::Center),
        "end" | "right_top" | "right_center" | "right_bottom" => Ok(Alignment::End),
        "stretch" => Ok(Alignment::Stretch),
        other => Err(WasmPythonError::BridgeJson(format!(
            "unknown alignment: {other}"
        ))),
    }
}

fn decode_button_style(value: &str) -> Result<ButtonStyle, WasmPythonError> {
    match value {
        "primary" => Ok(ButtonStyle::Primary),
        "secondary" => Ok(ButtonStyle::Secondary),
        "danger" => Ok(ButtonStyle::Danger),
        "ghost" => Ok(ButtonStyle::Ghost),
        other => Err(WasmPythonError::BridgeJson(format!(
            "unknown button style: {other}"
        ))),
    }
}

fn decode_badge_color(value: &str) -> Result<BadgeColor, WasmPythonError> {
    match value {
        "accent" => Ok(BadgeColor::Accent),
        "success" => Ok(BadgeColor::Success),
        "warning" => Ok(BadgeColor::Warning),
        "danger" => Ok(BadgeColor::Danger),
        "neutral" => Ok(BadgeColor::Neutral),
        other => Err(WasmPythonError::BridgeJson(format!(
            "unknown badge color: {other}"
        ))),
    }
}

fn runtime_execution_label(execution: RuntimeExecution) -> &'static str {
    match execution {
        RuntimeExecution::Local => "local",
        RuntimeExecution::Cloud => "cloud",
        RuntimeExecution::PreferredLocal => "preferred-local",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::wasm_app::bindings::plexi::platform::types::HttpResponse;
    use crate::host::wasm_app::Modifiers;
    use std::sync::Arc;
    use std::task::{Context, Wake, Waker};
    use tempfile::tempdir;
    use wasmtime::{Linker, Store};
    use wasmtime_wasi::p1::{self, WasiP1Ctx};
    use wasmtime_wasi::p2::pipe::MemoryOutputPipe;
    use wasmtime_wasi::{DirPerms, FilePerms, WasiCtxBuilder};

    struct NoopWake;
    impl Wake for NoopWake {
        fn wake(self: Arc<Self>) {}
    }

    #[test]
    fn python_keys_use_sdk_lowercase_names() {
        assert_eq!(python_key_name(egui::Key::ArrowDown), "down");
        assert_eq!(python_key_name(egui::Key::Escape), "escape");
    }

    #[test]
    fn python_key_tap_keeps_press_and_release_in_one_bridge_batch() {
        let raw = crate::app::key_str_to_egui_raw_input("right").expect("right key");

        let events = python_key_events(&raw.events);

        assert_eq!(events.len(), 2);
        assert_eq!(events[0]["key"], "right");
        assert_eq!(events[0]["pressed"], true);
        assert_eq!(events[1]["key"], "right");
        assert_eq!(events[1]["pressed"], false);
    }

    #[test]
    fn python_host_event_wire_delivers_cross_runtime_app_events() {
        let wire = encode_python_host_event(crate::app_protocol::PlexiEvent::AppEvent {
            subscription_id: "sub-1".to_string(),
            app_id: "wasm-counter".to_string(),
            event: "count.changed".to_string(),
            event_id: 7,
            resource_id: "counter-1".to_string(),
            trigger_mode: crate::app_protocol::TriggerMode::Conversation,
            summary: Some("Count changed".to_string()),
            payload: Some(json!({"count": 2})),
            state_ref: None,
            created_at: "2026-07-13T00:00:00Z".to_string(),
        })
        .unwrap();

        assert_eq!(wire["type"], "app_event");
        assert_eq!(wire["app_id"], "wasm-counter");
        assert_eq!(wire["event"], "count.changed");
        assert_eq!(wire["payload"]["count"], 2);
    }

    #[test]
    fn python_event_effects_use_shared_host_requests() {
        let subscribe = app_command_from_python_message(&json!({
            "type": "subscribe_event_streams",
            "request_id": "subscribe-1",
            "app_id": "wasm-counter",
            "event_names": ["count.changed"],
            "payload_mode": "full",
            "trigger_mode": "conversation",
            "resource_id": null,
        }))
        .expect("subscribe command");
        assert!(matches!(
            subscribe,
            crate::app::app_trait::AppCommand::AppEventRequest {
                request: crate::app_protocol::AppRequest::SubscribeAppEvents {
                    request_id,
                    app_id,
                    event_names,
                    ..
                },
                ..
            } if request_id == "subscribe-1"
                && app_id == "wasm-counter"
                && event_names == vec!["count.changed".to_string()]
        ));

        let emit = app_command_from_python_message(&json!({
            "type": "emit_event",
            "event": "note.saved",
            "actor": "app",
            "summary": "Saved note",
            "resource_id": "note-1",
            "revision_after": "rev-2",
            "payload_json": "{\"title\":\"Hello\"}",
            "changed_resources": ["note-1"],
        }))
        .expect("emit command");
        assert!(matches!(
            emit,
            crate::app::app_trait::AppCommand::AppEventRequest {
                request: crate::app_protocol::AppRequest::EmitEvent {
                    event,
                    payload: Some(payload),
                    ..
                },
                ..
            } if event == "note.saved" && payload["title"] == "Hello"
        ));
    }

    #[test]
    fn unsupported_sdk_node_is_rejected_explicitly() {
        let error = decode_node_data(&json!({"type": "Accordion"})).unwrap_err();
        assert!(error
            .to_string()
            .contains("unsupported CPython-WASM UINode type: Accordion"));
    }

    #[test]
    fn manifest_http_hosts_allow_exact_and_subdomains_only() {
        let hosts = vec!["api.example.com".to_string()];
        assert!(http_host_allowed("https://api.example.com/v1", &hosts));
        assert!(http_host_allowed("https://x.api.example.com/v1", &hosts));
        assert!(!http_host_allowed(
            "https://api.example.com.evil.test",
            &hosts
        ));
        assert!(!http_host_allowed("file:///tmp/secret", &hosts));
    }

    #[test]
    fn continuous_scheduler_preserves_sixty_fps_budget() {
        let interval = scheduler_repaint_after(Some("continuous"), Some(60));
        assert!(interval <= std::time::Duration::from_nanos(16_666_667));
        assert!(interval >= std::time::Duration::from_nanos(16_666_666));
    }

    #[test]
    fn python_runtime_waits_for_nonzero_viewport_before_init() {
        assert!(!valid_python_viewport(0.0, 100.0));
        assert!(!valid_python_viewport(640.0, 0.0));
        assert!(valid_python_viewport(640.0, 360.0));
    }

    #[test]
    fn fixed_deadline_skips_missed_intervals_without_round_trip_drift() {
        let start = std::time::Instant::now();
        let interval = std::time::Duration::from_millis(100);

        let next = advance_fixed_deadline(start, interval, start + interval * 3 + interval / 2);

        assert_eq!(next.duration_since(start), interval * 4);
    }

    #[test]
    fn scheduled_mode_allows_only_one_render_transaction() {
        let now = std::time::Instant::now();
        let mut scheduler = PythonFrameScheduler::new(now);

        let frame_id = scheduler.poll_render(now).expect("first frame");

        assert!(scheduler.poll_render(now).is_none());
        assert_eq!(scheduler.complete_frame(frame_id), Some(now));
    }

    #[test]
    fn scheduled_mode_uses_new_deadline_after_returning_to_idle() {
        let now = std::time::Instant::now();
        let delay = std::time::Duration::from_millis(100);
        let mut scheduler = PythonFrameScheduler::new(now);
        let frame_id = scheduler.poll_render(now).expect("first frame");
        scheduler.complete_frame(frame_id).expect("complete frame");

        scheduler.request_render_after(now, delay);

        assert_eq!(scheduler.next_repaint_deadline(now), Some(now + delay));
    }

    #[test]
    fn component_tree_becomes_visible_only_at_matching_frame_commit() {
        let now = std::time::Instant::now();
        let mut scheduler = PythonFrameScheduler::new(now);
        let frame_id = scheduler.poll_render(now).expect("first frame");
        let pending_tree = decode_python_ui_tree_value(
            &serde_json::from_str(
                r#"{"root":0,"nodes":[{"id":0,"key":"0","data":{"type":"Text","text":"new"}}]}"#,
            )
            .expect("valid JSON"),
        )
        .expect("pending tree");
        let mut pending = HashMap::from([(frame_id, pending_tree)]);
        let mut visible = None;

        assert!(visible.is_none());
        assert!(
            commit_python_frame(&mut scheduler, &mut pending, &mut visible, frame_id,).is_some()
        );
        assert!(pending.is_empty());
        assert!(visible.is_some());
    }

    #[test]
    fn continuous_scheduler_stops_repainting_when_guest_misses_deadline() {
        let now = std::time::Instant::now();
        let interval = scheduler_repaint_after(Some("continuous"), Some(60));
        let mut scheduler = PythonFrameScheduler::new(now);
        scheduler.set_mode(Some("continuous"), Some(60), now);
        scheduler.poll_render(now).expect("first frame");
        scheduler.poll_render(now + interval).expect("second frame");
        scheduler
            .poll_render(now + interval * 2)
            .expect("third frame");

        assert!(scheduler
            .next_repaint_deadline(now + interval * 3)
            .is_none());
        assert!(scheduler.output_notifications_enabled(now + interval * 3));
    }

    #[test]
    fn continuous_scheduler_coalesces_guest_wake_into_next_host_deadline() {
        let now = std::time::Instant::now();
        let mut scheduler = PythonFrameScheduler::new(now);
        scheduler.set_mode(Some("continuous"), Some(60), now);
        scheduler.poll_render(now).expect("first frame");

        assert!(scheduler.next_repaint_deadline(now).is_some());
        assert!(!scheduler.output_notifications_enabled(now));
    }

    #[test]
    fn continuous_scheduler_admits_one_frame_before_the_presentation_deadline() {
        let now = std::time::Instant::now();
        let interval = scheduler_repaint_after(Some("continuous"), Some(60));
        let mut scheduler = PythonFrameScheduler::new(now);
        scheduler.set_mode(Some("continuous"), Some(60), now);
        let first = scheduler.poll_render(now).expect("first frame");
        scheduler
            .complete_frame(first)
            .expect("complete first frame");

        let wake_at = now + interval - CONTINUOUS_FRAME_HEADROOM;
        assert_eq!(scheduler.next_repaint_deadline(now), Some(wake_at));
        scheduler.poll_render(wake_at).expect("second frame");
        assert!(scheduler.poll_render(now + interval).is_none());
    }

    #[test]
    fn continuous_scheduler_sends_one_transaction_per_host_tick() {
        let start = std::time::Instant::now();
        let interval = scheduler_repaint_after(Some("continuous"), Some(60));
        let mut scheduler = PythonFrameScheduler::new(start);
        scheduler.set_mode(Some("continuous"), Some(60), start);
        let mut frame_id = scheduler.poll_render(start).expect("first frame");

        for tick in 1..=120 {
            let now = start + interval * tick;
            assert_eq!(scheduler.complete_frame(frame_id), Some(now - interval));
            frame_id = scheduler.poll_render(now).expect("next frame");
            assert_eq!(
                scheduler.next_repaint_deadline(now),
                Some(now + interval - CONTINUOUS_FRAME_HEADROOM)
            );
            assert!(!scheduler.output_notifications_enabled(now));
        }
    }

    #[test]
    fn repaint_delay_compensates_for_egui_predicted_frame_subtraction() {
        let now = std::time::Instant::now();
        let interval = std::time::Duration::from_nanos(16_666_666);
        let predicted = std::time::Duration::from_nanos(16_666_666);

        assert_eq!(
            repaint_delay_until(now + interval, now, predicted),
            interval * 2
        );
    }

    #[test]
    fn render_transaction_carries_due_timer_events() {
        let event = python_render_event(7, vec!["drop".to_string()]);

        assert_eq!(event["frame_id"], 7);
        assert_eq!(event["timer_ids"], json!(["drop"]));
    }

    #[test]
    fn appendable_stdin_waits_then_delivers_appended_json_line() {
        let mut input = AppendableStdin::default();
        let waker = Waker::from(Arc::new(NoopWake));
        let mut context = Context::from_waker(&waker);
        let mut bytes = [0_u8; 64];
        let mut read = ReadBuf::new(&mut bytes);
        assert!(Pin::new(&mut input)
            .poll_read(&mut context, &mut read)
            .is_pending());

        input
            .push_json_line(&json!({"type": "render"}))
            .expect("append JSON");
        let mut read = ReadBuf::new(&mut bytes);
        assert!(Pin::new(&mut input)
            .poll_read(&mut context, &mut read)
            .is_ready());
        assert_eq!(read.filled(), b"{\"type\":\"render\"}\n");
    }

    #[test]
    fn drainable_output_preserves_stream_across_drains() {
        let output = DrainableOutput::default();
        let mut writer = output.clone();
        let waker = Waker::from(Arc::new(NoopWake));
        let mut context = Context::from_waker(&waker);
        assert!(Pin::new(&mut writer)
            .poll_write(&mut context, b"one\n")
            .is_ready());
        assert_eq!(output.drain(), b"one\n");
        assert!(Pin::new(&mut writer)
            .poll_write(&mut context, b"two\n")
            .is_ready());
        assert_eq!(output.drain(), b"two\n");
    }

    #[test]
    fn drainable_output_wakes_host_when_guest_writes() {
        let output = DrainableOutput::default();
        let woke = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let wake_flag = woke.clone();
        output.set_waker(Arc::new(move || {
            wake_flag.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }));
        let mut writer = output.clone();
        let waker = Waker::from(Arc::new(NoopWake));
        let mut context = Context::from_waker(&waker);

        assert!(Pin::new(&mut writer)
            .poll_write(&mut context, b"frame_done\n")
            .is_ready());
        assert_eq!(woke.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert!(Pin::new(&mut writer).poll_flush(&mut context).is_ready());
        assert_eq!(woke.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(Pin::new(&mut writer)
            .poll_write(&mut context, b"another_message\n")
            .is_ready());
        assert!(Pin::new(&mut writer).poll_flush(&mut context).is_ready());
        assert_eq!(woke.load(std::sync::atomic::Ordering::SeqCst), 1);
        output.drain();
        assert!(Pin::new(&mut writer)
            .poll_write(&mut context, b"next_frame\n")
            .is_ready());
        assert!(Pin::new(&mut writer).poll_flush(&mut context).is_ready());
        assert_eq!(woke.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[test]
    fn drainable_output_defers_wake_until_notifications_are_rearmed() {
        let output = DrainableOutput::default();
        let woke = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let wake_flag = woke.clone();
        output.set_waker(Arc::new(move || {
            wake_flag.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }));
        output.set_notifications_enabled(false);
        let mut writer = output.clone();
        let waker = Waker::from(Arc::new(NoopWake));
        let mut context = Context::from_waker(&waker);

        assert!(Pin::new(&mut writer)
            .poll_write(&mut context, b"frame_done\n")
            .is_ready());
        assert!(Pin::new(&mut writer).poll_flush(&mut context).is_ready());
        assert_eq!(woke.load(std::sync::atomic::Ordering::SeqCst), 0);

        output.set_notifications_enabled(true);
        assert_eq!(woke.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn persistent_cpython_runtime_handles_lifecycle_without_native_python() {
        let app = tempdir().expect("app dir");
        std::fs::write(
            app.path().join("main.py"),
            "from plexi_sdk.ui import Canvas, CanvasCircle, Column, Text\ndef init(size, args): return []\ndef update(event): return []\ndef view(): return Column([Text('wasm-python-live'), Canvas([CanvasCircle(41.0, 42.0, 8.0, '#abcdef')])])\n",
        )
        .expect("write app");
        let config = PythonLaunchConfig {
            app_id: "test.wasm-python".to_string(),
            app_dir: app.path().to_path_buf(),
            entry: app.path().join("main.py"),
            module_name: "main".to_string(),
            launch_args: Vec::new(),
            workspace_root: app.path().to_path_buf(),
            capabilities: Vec::new(),
            allowed_hosts: Vec::new(),
        };
        let mut runtime = WasmPythonRuntime::launch(&config).expect("launch CPython WASM");
        runtime
            .send(&json!({
                "type": "init", "app_id": config.app_id, "workspace_root": "/",
                "capabilities": [], "state": {}, "theme": {}
            }))
            .expect("send init");
        runtime
            .send(&json!({"type": "render", "frame_id": 1}))
            .expect("send render");

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let mut messages = Vec::new();
        while std::time::Instant::now() < deadline {
            messages.extend(runtime.drain_messages().expect("valid runtime JSON"));
            if messages
                .iter()
                .any(|message| message.get("type") == Some(&json!("frame_done")))
            {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(
            messages.iter().any(|message| {
                message.get("type") == Some(&json!("component_tree"))
                    && message.to_string().contains("wasm-python-live")
            }),
            "messages={messages:?}; stderr={}",
            runtime.drain_stderr()
        );
        let tree = messages
            .iter()
            .find(|message| message.get("type") == Some(&json!("component_tree")))
            .and_then(|message| message.get("tree"))
            .and_then(|tree| decode_ui_tree_value(tree).ok())
            .expect("decoded runtime tree");
        assert!(tree.nodes.iter().any(|node| matches!(
            node.data,
            UiNodeData::Canvas(ref canvas) if matches!(
                canvas.commands.first(),
                Some(CanvasCommand::Circle(CanvasCircle { x, y, .. }))
                    if *x == 41.0 && *y == 42.0
            )
        )));
    }

    fn python_shim_fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/wasm-fixtures/python-shim.wasm")
    }

    fn sysmon_fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/wasm-fixtures/sysmon.wasm")
    }

    #[test]
    fn cpython_wasi_executes_inside_wasmtime_without_native_python() {
        let bundle = resolve_default_cpython_bundle().expect("fetched CPython WASI bundle");
        let stdlib = bundle.parent().expect("bundle directory").join("Lib");
        let (engine, module) = cached_cpython_module(&bundle).expect("raw CPython WASI module");
        let mut linker = Linker::<WasiP1Ctx>::new(&engine);
        p1::add_to_linker_sync(&mut linker, |ctx| ctx).expect("WASI preview1 imports");
        let stdout = MemoryOutputPipe::new(4096);
        let stderr = MemoryOutputPipe::new(4096);
        let mut builder = WasiCtxBuilder::new();
        builder
            .preopened_dir(
                stdlib,
                "/usr/local/lib/python3.12",
                DirPerms::READ,
                FilePerms::READ,
            )
            .expect("mount stdlib")
            .stdout(stdout.clone())
            .stderr(stderr.clone())
            .args(&[
                "python".to_string(),
                "-c".to_string(),
                "print('cpython-in-wasmtime')".to_string(),
            ]);
        let mut store = Store::new(&engine, builder.build_p1());
        let instance = linker
            .instantiate(&mut store, &module)
            .expect("instantiate CPython WASI");
        let start = instance
            .get_typed_func::<(), ()>(&mut store, "_start")
            .expect("CPython _start");

        if let Err(error) = start.call(&mut store, ()) {
            panic!(
                "run CPython in Wasmtime: {error:#}; stderr={}",
                String::from_utf8_lossy(&stderr.contents())
            );
        }

        assert_eq!(
            String::from_utf8_lossy(&stdout.contents()).trim(),
            "cpython-in-wasmtime"
        );
    }

    fn tree_text(tree: &UiTree) -> String {
        tree.nodes
            .iter()
            .filter_map(|node| match &node.data {
                UiNodeData::Text(text) => Some(text.text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn manifest_python_compat_routes_to_launch_config() {
        let dir = tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("manifest.toml"),
            r#"
schema_version = 1

[app]
id = "hello-py"
type = "app"
name = "Hello Python"
entry = "main.py"
version = "0.1.0"

[runtime]
python_compat = true
"#,
        )
        .expect("manifest");
        std::fs::write(dir.path().join("main.py"), "def view(): pass\n").expect("entry");

        let config = PythonLaunchConfig::from_manifest_file(dir.path())
            .expect("route")
            .expect("python compat config");

        assert_eq!(config.module_name, "main");
        assert_eq!(config.app_id, "hello-py");
    }

    #[test]
    fn missing_bundle_returns_actionable_error() {
        let dir = tempdir().expect("tempdir");
        let err = resolve_cpython_bundle(dir.path().join("wasm-bundles")).unwrap_err();

        assert!(err.to_string().contains(FETCH_CPYTHON_BUNDLE_COMMAND));
    }

    #[test]
    fn missing_shim_component_returns_actionable_error() {
        let dir = tempdir().expect("tempdir");
        let err = resolve_cpython_shim_component(dir.path().join("wasm-bundles")).unwrap_err();

        assert!(matches!(err, WasmPythonError::MissingShimComponent { .. }));
        assert!(err.to_string().contains(BUILD_CPYTHON_SHIM_COMMAND));
    }

    #[test]
    fn bundle_hash_mismatch_is_typed() {
        let dir = tempdir().expect("tempdir");
        let bundle = dir.path().join(CPYTHON_BUNDLE_FILE);
        std::fs::create_dir_all(bundle.parent().expect("bundle parent")).expect("bundle dir");
        std::fs::write(&bundle, b"not-python").expect("bundle");

        let err = resolve_cpython_bundle(dir.path().to_path_buf()).unwrap_err();

        assert!(matches!(err, WasmPythonError::BundleHashMismatch { .. }));
    }

    #[test]
    fn raw_wasm_bundle_reports_unsupported_abi() {
        let dir = tempdir().expect("tempdir");
        let bundle = dir.path().join("python.wasm");
        std::fs::write(&bundle, b"\0asm\x01\0\0\0").expect("empty wasm module");

        let err = validate_cpython_bundle_abi(&bundle).unwrap_err();

        assert!(matches!(err, WasmPythonError::RawModuleAbiMismatch { .. }));
        assert!(err.to_string().contains("lifecycle"));
    }

    #[test]
    #[ignore = "requires `just fetch-cpython-bundle` and PLEXI_CPYTHON_BUNDLE_DIR pointing at that cache"]
    fn real_cpython_bundle_resolves_but_is_abi_blocked() {
        let bundle = resolve_default_cpython_bundle().expect("resolved CPython bundle");
        let err = validate_cpython_bundle_abi(&bundle).unwrap_err();

        eprintln!("{err}");
        assert!(matches!(err, WasmPythonError::RawModuleAbiMismatch { .. }));
    }

    #[test]
    fn python_shim_fixture_executes_lifecycle_json_bridge() {
        let mut app = WasmApp::load_with_grants(
            "python-shim-test",
            &python_shim_fixture(),
            StateStore::ephemeral(),
            Grants {
                state: true,
                ..Grants::default()
            },
        )
        .expect("load shim");
        let snapshot = StateSnapshot {
            entries: Vec::new(),
        };

        let init = app.init(&snapshot, (320.0, 240.0), &[]).expect("shim init");
        assert!(matches!(
            &init[0],
            Effect::SetTitle(title) if title == "Python Shim POC"
        ));
        let view = app.view().expect("shim view");
        assert!(tree_text(&view).contains("Count: 0"));

        let update = app
            .update(&InputEvent::UiAction(UiActionEvent {
                handler_id: "increment".to_string(),
            }))
            .expect("shim update");
        assert!(update.is_empty());
        let view = app.view().expect("updated shim view");
        assert!(tree_text(&view).contains("Count: 1"));
    }

    #[test]
    fn shim_lifecycle_contract_failure_is_typed() {
        let err = probe_cpython_shim_component(&sysmon_fixture()).unwrap_err();

        assert!(matches!(
            err,
            WasmPythonError::ShimLifecycleCallFailure {
                function: "view",
                ..
            } | WasmPythonError::ShimComponentLoadFailure { .. }
        ));
    }

    #[test]
    fn state_snapshot_encodes_json_bytes_and_raw_bytes() {
        let snapshot = StateSnapshot {
            entries: vec![
                ("count".to_string(), b"3".to_vec()),
                ("raw".to_string(), vec![0, 159, 146, 150]),
            ],
        };

        let encoded = encode_state(&snapshot);

        assert_eq!(encoded["count"], BASE64.encode(b"3"));
        assert_eq!(
            encoded["raw"],
            format!("b64:{}", BASE64.encode([0, 159, 146, 150]))
        );
    }

    #[test]
    fn key_event_encodes_sdk_v3_shape() {
        let event = InputEvent::Key(KeyEvent {
            key: "q".to_string(),
            modifiers: Modifiers {
                ctrl: false,
                shift: true,
                alt: false,
                meta: true,
            },
            pressed: true,
        });

        let encoded = encode_input_event(&event).expect("encode event");

        assert_eq!(encoded["type"], "KeyEvent");
        assert_eq!(encoded["modifiers"]["shift"], true);
        assert_eq!(encoded["modifiers"]["meta"], true);
    }

    #[test]
    fn effects_decode_host_effects_and_set_state_boundary() {
        let effects = decode_effects(
            r#"[
                {"type":"SetTitle","title":"hello"},
                {"type":"SetState","data":{"count":4}}
            ]"#,
        )
        .expect("effects");

        assert!(matches!(
            &effects[0],
            PythonBridgeEffect::Host(Effect::SetTitle(title)) if title == "hello"
        ));
        assert!(matches!(
            &effects[1],
            PythonBridgeEffect::SetState(entries) if entries == &vec![("count".to_string(), b"4".to_vec())]
        ));
    }

    #[test]
    fn effects_decode_http_fetch() {
        let effects = decode_effects(
            r#"[
                {
                    "type":"HttpFetch",
                    "url":"https://api.example.test/items",
                    "method":"POST",
                    "headers":{"Accept":"application/json"},
                    "body":[111,107]
                }
            ]"#,
        )
        .expect("effects");

        let PythonBridgeEffect::Host(Effect::HttpFetch(req)) = &effects[0] else {
            panic!("expected http fetch");
        };
        assert_eq!(req.url, "https://api.example.test/items");
        assert_eq!(req.method, "POST");
        assert_eq!(
            req.headers,
            vec![("Accept".to_string(), "application/json".to_string())]
        );
        assert_eq!(req.body, Some(b"ok".to_vec()));
    }

    #[test]
    fn http_response_event_encodes_sdk_v3_shape() {
        let encoded = encode_input_event(&InputEvent::HttpResponse(HttpResponse {
            status: 200,
            headers: vec![("content-type".to_string(), "application/json".to_string())],
            body: b"ok".to_vec(),
        }))
        .expect("event");

        assert_eq!(encoded["type"], "HttpResponse");
        assert_eq!(encoded["status"], 200);
        assert_eq!(encoded["body"], json!([111, 107]));
    }

    #[test]
    fn ui_tree_decodes_text_node() {
        let tree = decode_ui_tree(
            r#"{
                "root":0,
                "nodes":[
                    {"id":0,"key":"0","data":{"type":"Text","text":"ok","bold":true,"align":"center"}}
                ]
            }"#,
        )
        .expect("tree");

        match &tree.nodes[0].data {
            UiNodeData::Text(text) => assert_eq!(text.text, "ok"),
            other => panic!("expected text node, got {other:?}"),
        }
    }

    #[test]
    fn ui_tree_treats_null_optional_text_size_as_absent() {
        let tree = decode_ui_tree(
            r#"{
                "root":0,
                "nodes":[
                    {"id":0,"key":"0","data":{"type":"Text","text":"ok","size":null}}
                ]
            }"#,
        )
        .expect("tree");

        assert!(matches!(
            &tree.nodes[0].data,
            UiNodeData::Text(text) if text.size.is_none()
        ));
    }

    #[test]
    fn ui_tree_decodes_interactive_nodes() {
        let tree = decode_ui_tree(
            r#"{
                "root":0,
                "nodes":[
                    {"id":0,"key":"0","data":{"type":"Column","children":[1,2,3,4],"gap":4.0}},
                    {"id":1,"key":"0/input","data":{"type":"TextInput","value":"draft","placeholder":"New item","on_change":"draft","on_submit":"submit"}},
                    {"id":2,"key":"0/item","data":{"type":"Text","text":"Write tests"}},
                    {"id":3,"key":"0/list","data":{"type":"ListView","items":[2],"selected":0}},
                    {"id":4,"key":"0/progress","data":{"type":"ProgressBar","value":3.0,"max":5.0,"label":"3 / 5"}}
                ]
            }"#,
        )
        .expect("tree");

        assert!(matches!(tree.nodes[1].data, UiNodeData::TextInput(_)));
        assert!(matches!(tree.nodes[3].data, UiNodeData::ListView(_)));
        assert!(matches!(tree.nodes[4].data, UiNodeData::ProgressBar(_)));
    }

    #[test]
    fn ui_tree_decodes_canvas_node() {
        let tree = decode_ui_tree(
            r##"{
                "root":0,
                "nodes":[
                    {"id":0,"key":"0","data":{
                        "type":"Canvas",
                        "width":320.0,
                        "height":180.0,
                        "grow":true,
                        "commands":[
                            {"type":"rect","x":1.0,"y":2.0,"w":30.0,"h":40.0,"fill":"#112233","radius":2.0},
                            {"type":"circle","cx":41.0,"cy":42.0,"r":8.0,"fill":"#abcdef"},
                            {"type":"text","x":9.0,"y":10.0,"text":"ok","size":14.0,"color":"#ffffffcc","bold":true,"align":"center"}
                        ]
                    }}
                ]
            }"##,
        )
        .expect("tree");

        let UiNodeData::Canvas(canvas) = &tree.nodes[0].data else {
            panic!("expected canvas node");
        };
        assert_eq!(canvas.width, 320.0);
        assert_eq!(canvas.commands.len(), 3);
        assert!(matches!(canvas.commands[0], CanvasCommand::Rect(_)));
        assert!(matches!(
            canvas.commands[1],
            CanvasCommand::Circle(CanvasCircle { x, y, radius, .. })
                if x == 41.0 && y == 42.0 && radius == 8.0
        ));
        assert!(matches!(canvas.commands[2], CanvasCommand::Text(_)));
    }

    #[test]
    fn python_semantics_expose_decoded_tree_to_pane_state() {
        let tree = decode_python_ui_tree_value(&serde_json::from_str(
            r##"{
                "root":0,
                "nodes":[
                    {"id":0,"key":"0","data":{"type":"Column","children":[1,2],"gap":0.0}},
                    {"id":1,"key":"0/title","data":{"type":"Text","text":"Balls"}},
                    {"id":2,"key":"0/canvas","data":{"type":"Canvas","width":640.0,"height":360.0,"grow":true,"commands":[
                        {"type":"circle","x":41.0,"y":42.0,"radius":8.0,"fill":"#abcdef"}
                    ]}}
                ]
            }"##,
        )
        .expect("valid JSON"))
        .expect("tree");

        let state = python_semantic_state(Some(&tree));
        assert_eq!(state.runtime_kind, "python-wasm");
        assert_eq!(state.roots, ["0"]);
        assert_eq!(state.nodes.len(), 3);
        assert_eq!(state.nodes[1].label.as_deref(), Some("Balls"));
        assert_eq!(state.nodes[2].value.as_deref(), Some("640x360 fit=fill"));
        assert_eq!(
            state.nodes[2].canvas_commands,
            [json!({
                "type": "circle",
                "x": 41.0,
                "y": 42.0,
                "radius": 8.0,
                "fill": "#abcdef",
            })]
        );
    }

    #[test]
    fn decoder_accepts_the_complete_sdk_text_alignment_vocabulary() {
        let cases = [
            ("left_top", Alignment::Start),
            ("left_center", Alignment::Start),
            ("left_bottom", Alignment::Start),
            ("center_top", Alignment::Center),
            ("center_center", Alignment::Center),
            ("center_bottom", Alignment::Center),
            ("right_top", Alignment::End),
            ("right_center", Alignment::End),
            ("right_bottom", Alignment::End),
        ];

        for (sdk_value, expected) in cases {
            assert_eq!(decode_alignment(sdk_value).unwrap(), expected);
        }
    }

    #[test]
    fn native_python_bridge_runs_calc_lifecycle() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let sdk_dir = root.join("sdk/python");
        let app_dir = root.join("apps/calc");
        let snapshot = StateSnapshot {
            entries: Vec::new(),
        };
        let update_event = InputEvent::Key(KeyEvent {
            key: "7".to_string(),
            modifiers: Modifiers {
                ctrl: false,
                shift: false,
                alt: false,
                meta: false,
            },
            pressed: true,
        });

        let output = run_native_python_lifecycle_probe(
            &sdk_dir,
            &app_dir,
            "calc",
            &init_bridge_arg(&snapshot, (320.0, 240.0), &[]),
            &update_bridge_arg(&snapshot, &update_event).expect("update arg"),
            &view_bridge_arg(&snapshot),
        )
        .expect("native bridge probe");

        let init = decode_effects(&output.init_json).expect("init effects");
        let update = decode_effects(&output.update_json).expect("update effects");
        let view = decode_ui_tree(&output.view_json).expect("view tree");

        assert!(matches!(
            &init[0],
            PythonBridgeEffect::Host(Effect::SetTitle(title)) if title == "Calculator"
        ));
        assert!(matches!(
            &update[0],
            PythonBridgeEffect::SetState(entries) if entries.iter().any(|(key, value)| key == "display" && value == b"\"7\"")
        ));
        assert!(view
            .nodes
            .iter()
            .any(|node| matches!(&node.data, UiNodeData::Text(text) if text.text == "Calculator")));
    }

    #[test]
    fn native_python_bridge_runs_stats_lifecycle() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let sdk_dir = root.join("sdk/python");
        let app_dir = root.join("apps/stats");
        let snapshot = StateSnapshot {
            entries: Vec::new(),
        };
        let update_event = InputEvent::TimerFired(1);

        let output = run_native_python_lifecycle_probe(
            &sdk_dir,
            &app_dir,
            "stats",
            &init_bridge_arg(&snapshot, (480.0, 320.0), &[]),
            &update_bridge_arg(&snapshot, &update_event).expect("update arg"),
            &view_bridge_arg(&snapshot),
        )
        .expect("native bridge probe");

        let init = decode_effects(&output.init_json).expect("init effects");
        let update = decode_effects(&output.update_json).expect("update effects");
        let view = decode_ui_tree(&output.view_json).expect("view tree");

        assert!(init
            .iter()
            .any(|effect| matches!(effect, PythonBridgeEffect::Host(Effect::SetTitle(title)) if title == "Stats")));
        assert!(update
            .iter()
            .any(|effect| matches!(effect, PythonBridgeEffect::Host(Effect::SetStatus(_)))));
        assert!(view
            .nodes
            .iter()
            .any(|node| matches!(&node.data, UiNodeData::Text(text) if text.text.contains("No focus events"))));
    }

    fn native_python_app_view(app_name: &str, module_name: &str) -> UiTree {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let snapshot = StateSnapshot {
            entries: Vec::new(),
        };
        let output = run_native_python_lifecycle_probe(
            &root.join("sdk/python"),
            &root.join("apps").join(app_name),
            module_name,
            &init_bridge_arg(&snapshot, (480.0, 320.0), &[]),
            &update_bridge_arg(&snapshot, &InputEvent::TimerFired(1)).expect("update arg"),
            &view_bridge_arg(&snapshot),
        )
        .expect("native bridge probe");

        decode_ui_tree(&output.view_json).expect("view tree")
    }

    #[test]
    fn native_python_bridge_decodes_kraken_view() {
        let view = native_python_app_view("kraken", "main");

        assert!(view.nodes.iter().any(
            |node| matches!(&node.data, UiNodeData::Text(text) if text.text.contains("Kraken"))
        ));
    }

    #[test]
    fn native_python_bridge_decodes_logs_scroll_view() {
        let view = native_python_app_view("logs", "logs");

        assert!(view
            .nodes
            .iter()
            .any(|node| matches!(node.data, UiNodeData::Scroll(_))));
    }
}

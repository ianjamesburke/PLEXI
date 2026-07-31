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
    AppBarNode, BadgeColor, ButtonNode, ButtonStyle, CanvasCircle, CanvasCommand, CanvasLine,
    CanvasNode, CanvasRect, CanvasText, Color, ColumnNode, FooterKeyEntry, FooterKeysNode,
    IndexedNode, ListNode, PaddingNode, PinnedEdge, PinnedNode, ProgressBarNode, RowNode,
    ScrollNode, SpaceNode, SpinnerNode, TextInputNode, TextNode, UiNodeData, UiTree,
};
#[cfg(test)]
use super::wasm_app::bindings::plexi::platform::types::{
    DeclareToolsEffect, FileReadEffect, FileWriteEffect, HttpFetchEffect, InputEvent, KeyEvent,
    StateSnapshot, TimerEffect, ToolDecl, ToolResultEffect, UiActionEvent, UiValueChangeEvent,
};
use super::wasm_app::Alignment;
#[cfg(test)]
use super::wasm_app::{Effect, Grants, StateStore, WasmApp};
use super::wasm_frame::repaint_delay_until;

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

#[derive(Default)]
struct OutputState {
    bytes: Vec<u8>,
    closed: bool,
}

/// Cloneable WASI output drainable without closing the guest stream. A
/// dedicated decoder thread (stint 0438) blocks on `wait_and_drain`; the
/// guest's writes and an explicit `close` wake it through the paired condvar,
/// so JSON + tree decode runs off the paint thread.
#[derive(Clone, Default)]
pub struct DrainableOutput {
    state: Arc<Mutex<OutputState>>,
    signal: Arc<std::sync::Condvar>,
}

impl DrainableOutput {
    /// Non-blocking drain, used by the headless one-shot path and stderr.
    pub fn drain(&self) -> Vec<u8> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        std::mem::take(&mut state.bytes)
    }

    /// Block until the guest writes bytes or the stream is closed. Returns
    /// `None` once closed and fully drained so the decoder loop can exit.
    fn wait_and_drain(&self) -> Option<Vec<u8>> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if !state.bytes.is_empty() {
                return Some(std::mem::take(&mut state.bytes));
            }
            if state.closed {
                return None;
            }
            state = self.signal.wait(state).unwrap_or_else(|e| e.into_inner());
        }
    }

    fn push_bytes(&self, buf: &[u8]) {
        {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.bytes.extend_from_slice(buf);
        }
        self.signal.notify_all();
    }

    /// Signal end-of-stream so a blocked decoder thread can exit.
    fn close(&self) {
        {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.closed = true;
        }
        self.signal.notify_all();
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
        self.push_bytes(buf);
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        self.signal.notify_all();
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
    #[error("read persisted app state at {path}: {source}")]
    #[cfg(test)]
    ReadState {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("parse persisted app state at {path}: {source}")]
    #[cfg(test)]
    ParseState {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("persisted app state at {path} must be a JSON object, got {found}")]
    #[cfg(test)]
    StateNotObject { path: PathBuf, found: &'static str },
    #[error("invalid [state] declaration: {0}")]
    StateScopes(String),
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
    pub theme: std::collections::HashMap<String, String>,
    /// Validated `[state] scopes` from the manifest. Ordered; the first entry
    /// is the app's default scope. See `crate::host::state_scope`.
    pub state_scopes: Vec<crate::host::state_scope::StateScope>,
    /// Validated `[state] format` from the manifest. JSON unless the app
    /// declared `format = "markdown"`, in which case the host is
    /// format-blind: the file's bytes round-trip verbatim under a single
    /// `document` key.
    pub state_format: crate::host::state_scope::StateFormat,
    /// The root of the launching pane's context, used to seed the pane's live
    /// `context_root` (which the host refreshes every frame). State paths
    /// resolve against the live value at call time — never against
    /// `workspace_root`, which stays launch-captured and serves only the
    /// `fs.read`/`fs.write` jail.
    pub context_root: PathBuf,
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
                execution: manifest.runtime.execution.as_str(),
            });
        }

        let state_scopes = manifest
            .state_scopes()
            .map_err(WasmPythonError::StateScopes)?;
        let state_format = manifest
            .state_format()
            .map_err(WasmPythonError::StateScopes)?;

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
            theme: crate::ui::theme::colors_from_config(
                &crate::config::PlexiConfig::load_with_workspace(Some(app_dir)),
            )
            .to_theme_map(),
            state_scopes,
            state_format,
            context_root: app_dir.to_path_buf(),
        }))
    }
}

fn python_init_payload(
    config: &PythonLaunchConfig,
    state: Value,
    size: (f32, f32),
) -> Value {
    json!({
        "type": "init",
        "app_id": config.app_id,
        "workspace_root": config.workspace_root,
        "capabilities": config.capabilities,
        "state": state,
        "theme": config.theme,
        "args": config.launch_args,
        "size": [size.0, size.1],
    })
}

fn cache_python_theme_for_relaunch(
    config: &mut PythonLaunchConfig,
    event: &crate::app_protocol::PlexiEvent,
) {
    if let crate::app_protocol::PlexiEvent::Theme { colors } = event {
        config.theme.clone_from(colors);
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
        })
    }

    pub fn send(&self, event: &Value) -> Result<(), WasmPythonError> {
        self.stdin.push_json_line(event)
    }

    /// Non-blocking synchronous drain for the headless one-shot path. The live
    /// pane instead runs a dedicated decoder thread over `stdout`.
    pub fn drain_messages(&mut self) -> Result<Vec<Value>, WasmPythonError> {
        let drained = self.stdout.drain();
        self.partial_stdout.extend(drained);
        let mut messages = Vec::new();
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
        Ok(messages)
    }

    pub fn drain_stderr(&self) -> String {
        String::from_utf8_lossy(&self.stderr.drain()).into_owned()
    }

    /// Whether the guest thread has exited (crash, clean exit, or failed
    /// boot). Buffered stdout may still hold undrained messages after this
    /// turns true — drain once more before treating the guest as gone.
    pub fn is_finished(&self) -> bool {
        self.thread
            .as_ref()
            .is_none_or(std::thread::JoinHandle::is_finished)
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

const HEADLESS_DEFAULT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// One-shot headless run of a CPython-in-WASM app through the exact same
/// runtime the live host uses (`WasmPythonRuntime` running
/// `plexi_sdk._v3_process`) — for `plexi app check` / `plexi app render`.
/// Boots, waits for `ready`, sends one `render`, and returns the decoded
/// `UiTree`. Unlike the deleted native-subprocess checker, there is no
/// separate wire protocol to drift from what the live host executes.
pub fn run_headless_frame(
    config: &PythonLaunchConfig,
    size: (f32, f32),
    seed_state: Option<Value>,
) -> Result<UiTree, WasmPythonError> {
    let mut session = HeadlessPythonSession::launch(config, size, seed_state)?;
    session.render_frame(1)
}

/// Render once, then send a `ui_action` for each handler in order inside the
/// same guest session, rendering after every action. State accumulates across
/// clicks, like a real user driving the app. Stops at the first failed action
/// — the guest is dead or wedged and later clicks cannot succeed. Returns the
/// pre-action frame plus one `(handler_id, render result)` entry per
/// attempted action.
pub type UiActionOutcomes = (UiTree, Vec<(String, Result<UiTree, WasmPythonError>)>);

pub fn run_headless_ui_action_sequence(
    config: &PythonLaunchConfig,
    size: (f32, f32),
    seed_state: Option<Value>,
    handler_ids: &[String],
) -> Result<UiActionOutcomes, WasmPythonError> {
    let mut session = HeadlessPythonSession::launch(config, size, seed_state)?;
    let before = session.render_frame(1)?;
    let mut outcomes = Vec::with_capacity(handler_ids.len());
    for (index, handler_id) in handler_ids.iter().enumerate() {
        let result = match session
            .runtime
            .send(&json!({"type": "ui_action", "handler_id": handler_id}))
        {
            Ok(()) => session.render_frame(2 + index as u64),
            Err(e) => Err(e),
        };
        let failed = result.is_err();
        outcomes.push((handler_id.clone(), result));
        if failed {
            break;
        }
    }
    Ok((before, outcomes))
}

struct HeadlessPythonSession {
    runtime: WasmPythonRuntime,
    /// Previous decoded frame, so `tree_delta` frames (stint 0438) can be
    /// reconstructed the same way the live host does.
    last_tree: Option<PythonUiTree>,
}

impl HeadlessPythonSession {
    fn launch(
        config: &PythonLaunchConfig,
        size: (f32, f32),
        seed_state: Option<Value>,
    ) -> Result<Self, WasmPythonError> {
        let runtime = WasmPythonRuntime::launch(config)?;
        runtime.send(&python_init_payload(
            config,
            seed_state.unwrap_or(Value::Null),
            size,
        ))?;
        let mut session = Self {
            runtime,
            last_tree: None,
        };
        session.wait_for(|message| message.get("type").and_then(Value::as_str) == Some("ready"))?;
        Ok(session)
    }

    fn render_frame(&mut self, frame_id: u64) -> Result<UiTree, WasmPythonError> {
        self.runtime
            .send(&python_render_event(frame_id, Vec::new()))?;
        let mut tree_message = None;
        self.wait_for(|message| {
            let message_type = message.get("type").and_then(Value::as_str);
            if matches!(message_type, Some("component_tree" | "tree_delta")) {
                tree_message = Some(message.clone());
            }
            message_type == Some("frame_done")
        })?;
        let message = tree_message.ok_or_else(|| {
            WasmPythonError::BridgeJson("app emitted frame_done with no component_tree".to_string())
        })?;
        let tree = match message.get("type").and_then(Value::as_str) {
            Some("tree_delta") => {
                let base = self.last_tree.as_ref().ok_or_else(|| {
                    WasmPythonError::BridgeJson(
                        "tree_delta received before any full tree".to_string(),
                    )
                })?;
                let changed = message
                    .get("changed")
                    .and_then(Value::as_array)
                    .ok_or_else(|| {
                        WasmPythonError::BridgeJson(
                            "tree_delta missing 'changed' array".to_string(),
                        )
                    })?;
                apply_tree_delta(base, changed)?
            }
            _ => {
                let raw = message.get("tree").ok_or_else(|| {
                    WasmPythonError::BridgeJson("component_tree message missing 'tree'".to_string())
                })?;
                decode_python_ui_tree_value(raw)?
            }
        };
        self.last_tree = Some(tree.clone());
        Ok(tree.tree)
    }

    fn wait_for(&mut self, mut matches: impl FnMut(&Value) -> bool) -> Result<(), WasmPythonError> {
        let deadline = std::time::Instant::now() + HEADLESS_DEFAULT_TIMEOUT;
        loop {
            for message in self.runtime.drain_messages()? {
                if matches(&message) {
                    return Ok(());
                }
            }
            if self.runtime.is_finished() {
                // The guest may have flushed final messages between the drain
                // above and exiting — drain once more before declaring death.
                for message in self.runtime.drain_messages()? {
                    if matches(&message) {
                        return Ok(());
                    }
                }
                return Err(self.guest_death_error());
            }
            if std::time::Instant::now() > deadline {
                // A guest that crashed right at the deadline must still be
                // reported as a death, not a timeout: re-check the process-exit
                // signal before falling back to the timeout message. Under
                // parallel-test CPU contention the poll can reach the deadline
                // in the same window the guest thread is unwinding, and the
                // cause the caller sees must not flip based on that race.
                if self.runtime.is_finished() {
                    for message in self.runtime.drain_messages()? {
                        if matches(&message) {
                            return Ok(());
                        }
                    }
                    return Err(self.guest_death_error());
                }
                let stderr = self.runtime.drain_stderr();
                return Err(WasmPythonError::BridgeJson(format!(
                    "timed out waiting for app response after {HEADLESS_DEFAULT_TIMEOUT:?}{}",
                    if stderr.trim().is_empty() {
                        String::new()
                    } else {
                        format!("\n  stderr:\n{stderr}")
                    }
                )));
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    /// The error returned when the guest thread has exited before producing the
    /// awaited message. Drains stderr so the crash traceback rides along.
    fn guest_death_error(&self) -> WasmPythonError {
        let stderr = self.runtime.drain_stderr();
        WasmPythonError::BridgeJson(format!(
            "app exited before responding{}",
            if stderr.trim().is_empty() {
                String::new()
            } else {
                format!("\n  stderr:\n{stderr}")
            }
        ))
    }
}

pub struct LivePythonPane {
    config: PythonLaunchConfig,
    runtime: WasmPythonRuntime,
    app_id: String,
    title: Option<String>,
    tree: Option<Arc<PythonUiTree>>,
    pending_trees: HashMap<u64, Arc<PythonUiTree>>,
    initialized: bool,
    ready: bool,
    frame_scheduler: PythonFrameScheduler,
    /// Off-paint-thread JSON + tree decode (stint 0438). Recreated on
    /// `relaunch` since it is bound to the runtime's stdout.
    decoder: PythonOutputDecoder,
    /// Egui repaint target for the decoder, installed on the first `ui()` and
    /// shared with the decoder so a ready frame wakes the paint loop.
    repaint: RepaintHook,
    wants_close: bool,
    error: Option<String>,
    /// stderr accumulated from a Python traceback until its unindented
    /// exception line arrives. `DrainableOutput` may split one traceback
    /// across several non-blocking drains.
    pending_traceback_stderr: String,
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
    /// Per-scope in-memory copy of persisted state, keyed by declared scope.
    /// Each entry also caches the backing file's `(mtime, len)` from the last
    /// successful read/write so a persist can detect an external write that
    /// landed since (disk wins — see `save_state`).
    persisted_states: HashMap<crate::host::state_scope::StateScope, ScopeState>,
    /// The root of this pane's context, refreshed by the host on every `ui`
    /// pass. State paths resolve against this at persist time, so
    /// `plexi context set-root` redirects where context-scoped state lands
    /// without a relaunch.
    context_root: PathBuf,
    http_tx: std::sync::mpsc::Sender<(String, crate::host::services::HttpResponse)>,
    http_rx: std::sync::mpsc::Receiver<(String, crate::host::services::HttpResponse)>,
    /// File-picker backend (stint 0508). Native rfd dialog in production, a
    /// scripted queue under `PLEXI_PICKER_SCRIPT` / harness override so agent
    /// tests can drive the full pick → grant → read/write flow headlessly.
    picker: Arc<dyn crate::host::services::PickerService>,
    picker_tx: std::sync::mpsc::Sender<(String, crate::host::services::FilePickOutcome)>,
    picker_rx: std::sync::mpsc::Receiver<(String, crate::host::services::FilePickOutcome)>,
    /// Picker-granted fs roots (stint 0508): canonicalized paths the user
    /// picked through `OpenFilePicker`. `workspace_path` accepts absolute
    /// paths under these roots in addition to the workspace jail. Grants are
    /// per-pane, live for the pane's lifetime (deliberately kept across
    /// hot-reload `relaunch`), and are never persisted.
    granted_fs_roots: Vec<PathBuf>,
    pending_commands: Vec<crate::app::app_trait::AppCommand>,
    /// Stint 0426: `ui()`'s caller (`tiling.rs`/`render.rs`) removes a queued
    /// `PendingPaneClick` from `PlexiApp::pending_pane_clicks` *before*
    /// calling `ui()`, so this is the pane's only chance to consume it. `ui()`
    /// has three early-return branches (fatal error, not yet initialized, not
    /// yet `ready`) that ran before ever reaching the tree-render call — a
    /// click/node-click arriving on exactly one of those frames (subprocess
    /// startup, or a hot-reload `relaunch()`) was silently dropped forever
    /// with no error and no retry, even though `plexi pane click --node`
    /// reported `{"ok": true}`. Carry it here across the transient branches
    /// instead of discarding it, so it survives to the next frame that
    /// actually reaches the render call.
    pending_click_carry: Option<crate::host::pane::PendingPaneClick>,
    /// Line-buffered, traceback-aware classifier over this guest's stderr.
    /// Lives on the pane because a traceback spans several `drain_stderr`
    /// calls and its state must survive between them.
    stderr_classifier: GuestStderrClassifier,
}

#[derive(Debug, Clone)]
struct PythonUiTree {
    tree: UiTree,
    canvas_fits: HashMap<u32, super::wasm_render::CanvasFit>,
}

/// A decoded frame or bridge message handed from the decoder thread to the
/// paint thread (stint 0438). JSON + tree decode happen on the decoder thread;
/// the paint thread only picks up the ready result. Decode durations ride
/// along so the per-app perf log keeps reporting `json_ms` / `tree_ms`.
enum DecodedOutput {
    Tree {
        frame_id: Option<u64>,
        tree: Arc<PythonUiTree>,
        json_time: std::time::Duration,
        tree_time: std::time::Duration,
        bytes: usize,
    },
    Message {
        value: Value,
        json_time: std::time::Duration,
        bytes: usize,
    },
    /// A malformed full tree — a hard bug, not a recoverable desync. Surfaced to
    /// the pane as a fatal error the way the inline decode used to.
    DecodeError(String),
}

/// Where a decoder thread wakes the paint loop once a frame is ready. Installed
/// lazily on the first `ui()` call (egui's `Context` only exists then) and
/// shared across `relaunch` so a fresh decoder inherits it immediately.
type RepaintHook = Arc<Mutex<Option<(egui::Context, egui::ViewportId)>>>;

/// Owns the off-paint-thread decoder: the thread handle, the ready-frame queue,
/// and the `stdout` handle it drains (kept so `Drop` can wake and join it).
struct PythonOutputDecoder {
    rx: std::sync::mpsc::Receiver<DecodedOutput>,
    thread: Option<JoinHandle<()>>,
    stdout: DrainableOutput,
    /// Decoded outputs sent but not yet taken. `std::sync::mpsc::Receiver` has
    /// no length, and `needs_background_tick` must answer "is there work here"
    /// without consuming anything — an off-screen pane is asked every frame and
    /// a peek that took a message would drop it on the floor.
    queued: Arc<std::sync::atomic::AtomicUsize>,
}

impl PythonOutputDecoder {
    fn spawn(runtime: &WasmPythonRuntime, app_id: String, repaint: RepaintHook) -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        let stdout = runtime.stdout.clone();
        let stdin = runtime.stdin.clone();
        let thread_stdout = stdout.clone();
        let queued = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let thread_queued = queued.clone();
        let thread = std::thread::Builder::new()
            .name(format!("plexi-python-decode-{app_id}"))
            .spawn(move || {
                decode_loop(thread_stdout, stdin, app_id, tx, repaint, thread_queued);
            })
            .ok();
        Self {
            rx,
            thread,
            stdout,
            queued,
        }
    }

    /// Take one decoded output, keeping [`Self::queued`] in step with the
    /// channel. Every read of `rx` must go through here.
    fn try_recv(&self) -> Result<DecodedOutput, std::sync::mpsc::TryRecvError> {
        let output = self.rx.try_recv();
        if output.is_ok() {
            self.queued
                .fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
        }
        output
    }

    /// True when the guest has produced output the host has not read yet.
    fn has_queued_output(&self) -> bool {
        self.queued.load(std::sync::atomic::Ordering::Acquire) > 0
    }
}

impl Drop for PythonOutputDecoder {
    fn drop(&mut self) {
        self.stdout.close();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Read guest stdout, split JSON lines, decode trees (full or delta), and hand
/// ready frames to the paint thread. Deltas patch the previous decoded tree in
/// place; an unapplyable delta triggers a fail-loud `request_full_tree` resync
/// instead of painting a corrupt tree.
fn decode_loop(
    stdout: DrainableOutput,
    stdin: AppendableStdin,
    app_id: String,
    tx: std::sync::mpsc::Sender<DecodedOutput>,
    repaint: RepaintHook,
    queued: Arc<std::sync::atomic::AtomicUsize>,
) {
    let mut partial: Vec<u8> = Vec::new();
    let mut last_tree: Option<Arc<PythonUiTree>> = None;
    let mut awaiting_full = false;
    while let Some(bytes) = stdout.wait_and_drain() {
        partial.extend(bytes);
        let mut produced = false;
        while let Some(newline) = partial.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = partial.drain(..=newline).collect();
            let line = &line[..line.len().saturating_sub(1)];
            if line.is_empty() {
                continue;
            }
            let bytes = line.len() + 1;
            let json_started = std::time::Instant::now();
            let value: Value = match serde_json::from_slice(line) {
                Ok(value) => value,
                Err(error) => {
                    // Count before sending, never after: the paint side may take
                    // the message the instant it lands, and a decrement that
                    // beat its increment would underflow the unsigned counter
                    // into "always busy".
                    queued.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
                    if tx.send(DecodedOutput::DecodeError(error.to_string())).is_err() {
                        queued.fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
                    }
                    produced = true;
                    continue;
                }
            };
            let json_time = json_started.elapsed();
            let message_type = value.get("type").and_then(Value::as_str);
            let output = match message_type {
                Some("component_tree") => {
                    decode_full_tree(&value, json_time, bytes, &mut last_tree)
                }
                Some("tree_delta") => {
                    match decode_tree_delta(&value, json_time, bytes, &last_tree) {
                        Ok(output) => {
                            if let DecodedOutput::Tree { tree, .. } = &output {
                                last_tree = Some(tree.clone());
                            }
                            Some(output)
                        }
                        Err(reason) => {
                            last_tree = None;
                            if !awaiting_full {
                                awaiting_full = true;
                                log::warn!(
                                    "app::{app_id}: tree delta could not be applied ({reason}); requesting full-tree resync"
                                );
                                if let Err(error) =
                                    stdin.push_json_line(&json!({"type": "request_full_tree"}))
                                {
                                    log::error!(
                                        "app::{app_id}: failed to request full-tree resync: {error}"
                                    );
                                }
                            }
                            None
                        }
                    }
                }
                _ => Some(DecodedOutput::Message {
                    value,
                    json_time,
                    bytes,
                }),
            };
            if let Some(output) = output {
                if matches!(output, DecodedOutput::Tree { .. }) {
                    awaiting_full = false;
                }
                queued.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
                if tx.send(output).is_err() {
                    queued.fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
                    return; // paint side dropped the pane
                }
                produced = true;
            }
        }
        if produced {
            if let Some((context, viewport)) =
                repaint.lock().unwrap_or_else(|e| e.into_inner()).as_ref()
            {
                // A zero-delay egui request deliberately schedules an extra
                // settling paint; a 1ns request wakes immediately without it.
                context.request_repaint_after_for(std::time::Duration::from_nanos(1), *viewport);
            }
        }
    }
}

fn decode_full_tree(
    value: &Value,
    json_time: std::time::Duration,
    bytes: usize,
    last_tree: &mut Option<Arc<PythonUiTree>>,
) -> Option<DecodedOutput> {
    let Some(raw) = value.get("tree") else {
        return Some(DecodedOutput::DecodeError(
            "component_tree message missing 'tree'".to_string(),
        ));
    };
    let tree_started = std::time::Instant::now();
    match decode_python_ui_tree_value(raw) {
        Ok(tree) => {
            let tree = Arc::new(tree);
            *last_tree = Some(tree.clone());
            Some(DecodedOutput::Tree {
                frame_id: value.get("frame_id").and_then(Value::as_u64),
                tree,
                json_time,
                tree_time: tree_started.elapsed(),
                bytes,
            })
        }
        Err(error) => Some(DecodedOutput::DecodeError(error.to_string())),
    }
}

fn decode_tree_delta(
    value: &Value,
    json_time: std::time::Duration,
    bytes: usize,
    last_tree: &Option<Arc<PythonUiTree>>,
) -> Result<DecodedOutput, String> {
    let base = last_tree
        .as_ref()
        .ok_or_else(|| "tree_delta received before any full tree".to_string())?;
    let changed = value
        .get("changed")
        .and_then(Value::as_array)
        .ok_or_else(|| "tree_delta missing 'changed' array".to_string())?;
    let tree_started = std::time::Instant::now();
    let tree = apply_tree_delta(base, changed).map_err(|error| error.to_string())?;
    Ok(DecodedOutput::Tree {
        frame_id: value.get("frame_id").and_then(Value::as_u64),
        tree: Arc::new(tree),
        json_time,
        tree_time: tree_started.elapsed(),
        bytes,
    })
}

/// Apply a `tree_delta` to the previous decoded tree, patching only the named
/// arena slots (and, for canvas nodes, the named command indices) in place. Any
/// out-of-range index or a `commands_changed` patch on a non-canvas node is a
/// desync — returned as an error so the caller resyncs rather than corrupting
/// the tree.
fn apply_tree_delta(
    base: &PythonUiTree,
    changed: &[Value],
) -> Result<PythonUiTree, WasmPythonError> {
    let mut tree = base.tree.clone();
    let mut canvas_fits = base.canvas_fits.clone();
    for patch in changed {
        let id = required_u32(patch, "id")?;
        let index = id as usize;
        let slot = tree.nodes.get_mut(index).ok_or_else(|| {
            WasmPythonError::BridgeJson(format!(
                "tree_delta id {id} out of range (arena has {} nodes)",
                base.tree.nodes.len()
            ))
        })?;
        if let Some(commands_changed) = patch.get("commands_changed").and_then(Value::as_array) {
            let UiNodeData::Canvas(canvas) = &mut slot.data else {
                return Err(WasmPythonError::BridgeJson(format!(
                    "tree_delta commands_changed on non-canvas node {id}"
                )));
            };
            for entry in commands_changed {
                let pair = entry.as_array().ok_or_else(|| {
                    WasmPythonError::BridgeJson(
                        "commands_changed entry is not an [index, command] pair".to_string(),
                    )
                })?;
                let command_index = pair.first().and_then(Value::as_u64).ok_or_else(|| {
                    WasmPythonError::BridgeJson(
                        "commands_changed entry missing integer index".to_string(),
                    )
                })? as usize;
                let command_value = pair.get(1).ok_or_else(|| {
                    WasmPythonError::BridgeJson(
                        "commands_changed entry missing command payload".to_string(),
                    )
                })?;
                let command = canvas.commands.get_mut(command_index).ok_or_else(|| {
                    WasmPythonError::BridgeJson(format!(
                        "tree_delta command index {command_index} out of range on node {id}"
                    ))
                })?;
                *command = decode_canvas_command(command_value)?;
            }
            // The guest only emits commands_changed when every other field
            // (including `fit`) is unchanged, so canvas_fits stays valid.
        } else {
            let node = decode_indexed_node(patch)?;
            match canvas_fit_for_node(patch)? {
                Some(fit) => {
                    canvas_fits.insert(id, fit);
                }
                None => {
                    canvas_fits.remove(&id);
                }
            }
            *slot = node;
        }
    }
    Ok(PythonUiTree { tree, canvas_fits })
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

    /// A dead guest must not keep the host painting: drop pending frames and
    /// stop requesting renders, so `poll_render` and `next_repaint_deadline`
    /// both go quiet.
    fn stop(&mut self) {
        self.mode = PythonSchedulerMode::Scheduled;
        self.render_requested = false;
        self.pending.clear();
    }
}

/// One scope's in-memory state plus the file identity it was last synced
/// against. `(mtime, len)` are `None` until the backing file has been seen on
/// disk; `error` is set when the file exists but could not be decoded — the
/// scope then refuses to persist until an external read clears it, so a
/// corrupt file is never silently reset to `{}`.
#[derive(Debug, Clone, Default)]
struct ScopeState {
    values: serde_json::Map<String, Value>,
    mtime: Option<std::time::SystemTime>,
    len: Option<u64>,
    error: Option<String>,
}

/// Resolve one declared scope to its state file, against the pane's context
/// root *at call time*. The host owns path construction — see
/// `crate::host::state_scope` for the two rules. Validates the app id.
fn python_state_path(
    app_id: &str,
    scope: crate::host::state_scope::StateScope,
    format: crate::host::state_scope::StateFormat,
    context_root: &Path,
) -> Result<PathBuf, String> {
    crate::host::state_scope::state_file(scope, app_id, format, context_root)
}

#[cfg(test)]
fn json_value_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}
#[cfg(test)]
fn ensure_python_state_gitignore(config: &PythonLaunchConfig) {
    if let Err(error) =
        crate::workspace::secrets::ensure_app_state_gitignore(&config.workspace_root)
    {
        log::warn!(
            "app::{}: could not ensure {}/.plexi/.gitignore covers app_states/: {error}",
            config.app_id,
            config.workspace_root.display()
        );
    }
}

/// Encode a scope's values for disk in the app's declared format.
///
/// JSON is pretty-printed. Markdown keeps the host format-blind: the app's
/// `document` value (which must be a string) is written verbatim — no JSON
/// envelope, no escaping.
fn encode_state_file(
    values: &serde_json::Map<String, Value>,
    format: crate::host::state_scope::StateFormat,
) -> Result<Vec<u8>, String> {
    match format {
        crate::host::state_scope::StateFormat::Json => {
            serde_json::to_vec_pretty(&Value::Object(values.clone()))
                .map_err(|error| format!("serialize state JSON: {error}"))
        }
        crate::host::state_scope::StateFormat::Markdown => match values.get("document") {
            Some(Value::String(document)) => Ok(document.as_bytes().to_vec()),
            Some(other) => Err(format!(
                "markdown state requires a string 'document' value, got {}",
                match other {
                    Value::Null => "null",
                    Value::Bool(_) => "a bool",
                    Value::Number(_) => "a number",
                    Value::Array(_) => "an array",
                    Value::Object(_) => "an object",
                    Value::String(_) => unreachable!(),
                }
            )),
            None => {
                Err("markdown state requires a 'document' key carrying the file text".to_string())
            }
        },
    }
}

/// Decode a state file's bytes in the app's declared format. The inverse of
/// [`encode_state_file`]: markdown text arrives as `{"document": "<file text>"}`.
pub(crate) fn decode_state_file(
    bytes: &[u8],
    format: crate::host::state_scope::StateFormat,
) -> Result<serde_json::Map<String, Value>, String> {
    match format {
        crate::host::state_scope::StateFormat::Json => {
            match serde_json::from_slice::<Value>(bytes) {
                Ok(Value::Object(state)) => Ok(state),
                Ok(other) => Err(format!(
                    "state file is not a JSON object (got {})",
                    match other {
                        Value::Null => "null",
                        Value::Bool(_) => "a bool",
                        Value::Number(_) => "a number",
                        Value::String(_) => "a string",
                        Value::Array(_) => "an array",
                        Value::Object(_) => unreachable!(),
                    }
                )),
                Err(error) => Err(format!("parse state JSON: {error}")),
            }
        }
        crate::host::state_scope::StateFormat::Markdown => {
            let text = String::from_utf8(bytes.to_vec())
                .map_err(|error| format!("markdown state is not valid UTF-8: {error}"))?;
            let mut map = serde_json::Map::new();
            map.insert("document".to_string(), Value::String(text));
            Ok(map)
        }
    }
}

/// Read one scope's state file into a [`ScopeState`]. A missing file is an
/// empty map — indistinguishable from first launch by design. A file that
/// exists but fails to decode sets `error` and leaves `values` empty; the
/// caller decides whether to keep previously-known values.
fn read_python_state_file(
    app_id: &str,
    path: &Path,
    format: crate::host::state_scope::StateFormat,
) -> ScopeState {
    let (mtime, len) = stat_state_file(path);
    match std::fs::read(path) {
        Ok(bytes) => match decode_state_file(&bytes, format) {
            Ok(values) => ScopeState {
                values,
                mtime,
                len,
                error: None,
            },
            Err(error) => {
                log::error!(
                    "app::{app_id}: state {} is unreadable ({error}) — state is NOT reset; \
                     persists are blocked until the file is fixed",
                    path.display()
                );
                ScopeState {
                    values: serde_json::Map::new(),
                    mtime,
                    len,
                    error: Some(error),
                }
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => ScopeState::default(),
        Err(error) => {
            let message = format!("read state {}: {error}", path.display());
            log::error!("app::{app_id}: {message} — state is NOT reset");
            ScopeState {
                values: serde_json::Map::new(),
                mtime,
                len,
                error: Some(message),
            }
        }
    }
}

/// Stat a state file's `(mtime, len)` identity pair. `(None, None)` when the
/// file does not exist (or cannot be statted).
fn stat_state_file(path: &Path) -> (Option<std::time::SystemTime>, Option<u64>) {
    match std::fs::metadata(path) {
        Ok(meta) => (meta.modified().ok(), Some(meta.len())),
        Err(_) => (None, None),
    }
}

/// Test-only convenience wrapping `load_python_states` down to "the one
/// scope this config declares" — production code always goes through
/// `load_python_states`/`read_python_state_file`, which is scope-aware by
/// construction; this exists only so pre-scope-resolver tests didn't all
/// need rewriting to enumerate `config.state_scopes` themselves. See
/// `state_test_config`, which always declares exactly one scope.
#[cfg(test)]
fn load_python_state(
    config: &PythonLaunchConfig,
) -> Result<serde_json::Map<String, Value>, WasmPythonError> {
    ensure_python_state_gitignore(config);
    let scope = config
        .state_scopes
        .first()
        .copied()
        .unwrap_or(crate::host::state_scope::StateScope::Global);
    let path = python_state_path(&config.app_id, scope, config.state_format, &config.context_root)
        .expect("resolve state path");
    match read_python_state_bytes(&path)? {
        Some(bytes) => parse_python_state(&path, &bytes),
        None => Ok(serde_json::Map::new()),
    }
}

#[cfg(test)]
fn read_python_state_bytes(path: &Path) -> Result<Option<Vec<u8>>, WasmPythonError> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            match std::fs::symlink_metadata(path) {
                Ok(_) => Err(WasmPythonError::ReadState {
                    path: path.to_path_buf(),
                    source,
                }),
                Err(metadata_error)
                    if metadata_error.kind() == std::io::ErrorKind::NotFound =>
                {
                    Ok(None)
                }
                Err(source) => Err(WasmPythonError::ReadState {
                    path: path.to_path_buf(),
                    source,
                }),
            }
        }
        Err(source) => Err(WasmPythonError::ReadState {
            path: path.to_path_buf(),
            source,
        }),
    }
}

#[cfg(test)]
fn parse_python_state(
    path: &Path,
    bytes: &[u8],
) -> Result<serde_json::Map<String, Value>, WasmPythonError> {
    let value =
        serde_json::from_slice::<Value>(bytes).map_err(|source| WasmPythonError::ParseState {
            path: path.to_path_buf(),
            source,
        })?;
    match value {
        Value::Object(state) => Ok(state),
        other => Err(WasmPythonError::StateNotObject {
            path: path.to_path_buf(),
            found: json_value_kind(&other),
        }),
    }
}

/// Load every declared scope's state file. A missing file is an empty map —
/// indistinguishable from first launch by design.
fn load_python_states(
    config: &PythonLaunchConfig,
) -> HashMap<crate::host::state_scope::StateScope, ScopeState> {
    config
        .state_scopes
        .iter()
        .map(|&scope| {
            let state = match python_state_path(
                &config.app_id,
                scope,
                config.state_format,
                &config.context_root,
            ) {
                Ok(path) => read_python_state_file(&config.app_id, &path, config.state_format),
                Err(error) => {
                    log::error!(
                        "app::{}: cannot resolve state path for scope {}: {error}",
                        config.app_id,
                        scope.as_str()
                    );
                    ScopeState {
                        error: Some(error),
                        ..ScopeState::default()
                    }
                }
            };
            (scope, state)
        })
        .collect()
}

impl LivePythonPane {
    /// Send an event to the CPython subprocess. A failed send (broken pipe)
    /// means the app runtime is dead — say so in the log instead of failing
    /// silent, so a hung app is diagnosable from plexi.log.
    fn send_to_runtime(&self, event: &Value) {
        if let Err(error) = self.runtime.send(event) {
            log::warn!(
                "app::{}: send to CPython runtime failed — runtime likely dead: {error}",
                self.app_id
            );
        }
    }

    pub fn launch(config: PythonLaunchConfig) -> Result<Self, WasmPythonError> {
        let app_id = config.app_id.clone();
        let persisted_states = load_python_states(&config);
        let context_root = config.context_root.clone();
        let (http_tx, http_rx) = std::sync::mpsc::channel();
        let (picker_tx, picker_rx) = std::sync::mpsc::channel();
        let runtime = WasmPythonRuntime::launch(&config)?;
        let repaint: RepaintHook = Arc::new(Mutex::new(None));
        let decoder = PythonOutputDecoder::spawn(&runtime, app_id.clone(), repaint.clone());
        Ok(Self {
            runtime,
            config,
            app_id,
            title: None,
            tree: None,
            pending_trees: HashMap::new(),
            initialized: false,
            ready: false,
            frame_scheduler: PythonFrameScheduler::new(std::time::Instant::now()),
            decoder,
            repaint,
            wants_close: false,
            error: None,
            pending_traceback_stderr: String::new(),
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
            persisted_states,
            context_root,
            http_tx,
            http_rx,
            picker: crate::host::services::default_picker_service(),
            picker_tx,
            picker_rx,
            granted_fs_roots: Vec::new(),
            pending_commands: Vec::new(),
            pending_click_carry: None,
            stderr_classifier: GuestStderrClassifier::new(),
        })
    }

    /// Refresh the live context root this pane resolves state paths against.
    /// Called by the host before every `ui` pass.
    pub fn set_context_root(&mut self, root: &Path) {
        if self.context_root != root {
            log::info!(
                "app::{}: context root changed {} -> {} — state scope resolution follows",
                self.app_id,
                self.context_root.display(),
                root.display()
            );
            self.context_root = root.to_path_buf();
        }
    }

    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        colors: &crate::ui::theme::Colors,
        pending_click: Option<crate::host::pane::PendingPaneClick>,
        pane_key: u64,
    ) {
        let host_frame_started = std::time::Instant::now();
        // Stint 0426: the caller already removed `pending_click` from
        // `PlexiApp::pending_pane_clicks` before this call — it is not
        // re-queued there. A fresh click for this frame wins over a carried
        // one from a prior frame that couldn't be delivered yet (an app is
        // never sent two independent clicks close enough together for both
        // to still be meaningful); the carried one is dropped in that case.
        let pending_click = pending_click.or_else(|| self.pending_click_carry.take());
        if pending_click.is_some() {
            log::info!(
                "app::{}: pending pane click/node-click entering render (carried={})",
                self.app_id,
                self.pending_click_carry.is_none()
            );
        }
        {
            let mut repaint = self.repaint.lock().unwrap_or_else(|e| e.into_inner());
            if repaint.is_none() {
                *repaint = Some((ui.ctx().clone(), ui.ctx().viewport_id()));
            }
        }
        if let Some(error) = &self.error {
            if pending_click.is_some() {
                // Fatal and does not self-heal without a relaunch (which
                // resets pending_click_carry) — fail loud instead of
                // silently swallowing the click forever.
                log::error!(
                    "app::{}: dropping pending pane click — pane is in a fatal error state: {error}",
                    self.app_id
                );
            }
            ui.colored_label(colors.danger, error);
            return;
        }
        if !self.initialized {
            let size = ui.available_size();
            if !valid_python_viewport(size.x, size.y) {
                if pending_click.is_some() {
                    log::info!(
                        "app::{}: deferring pending pane click — viewport not yet valid",
                        self.app_id
                    );
                    self.pending_click_carry = pending_click;
                }
                ui.centered_and_justified(|ui| {
                    ui.add(egui::Spinner::new());
                });
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
                "capabilities": self.config.capabilities,
                "state": self.default_scope_state(),
                "states": self.states_json(),
                "state_scopes": self.scope_names(),
                "theme": {},
                "args": self.config.launch_args,
                "size": [size.x, size.y]
            })) {
                self.error = Some(error.to_string());
                if pending_click.is_some() {
                    log::error!(
                        "app::{}: dropping pending pane click — init send failed: {error}",
                        self.app_id
                    );
                }
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
        if let Some(error) = &self.error {
            // Fatal state: stop the frame scheduler so a dead guest no longer
            // drives host repaints, and lead with the exception line.
            self.frame_scheduler.stop();
            ui.colored_label(colors.danger, error);
            return;
        }
        if self.runtime.is_finished() {
            self.frame_scheduler.stop();
            // No guest is left to service timers; firing them would send to a
            // dead runtime and misreport a clean exit as a failure.
            self.timers.clear();
            self.pending_timer_events.clear();
            if self.tree.is_none() {
                ui.colored_label(colors.danger, "app exited before rendering a frame");
                return;
            }
        }
        if !self.ready {
            if pending_click.is_some() {
                log::info!(
                    "app::{}: deferring pending pane click — pane not yet ready (subprocess startup or hot-reload relaunch)",
                    self.app_id
                );
                self.pending_click_carry = pending_click;
            }
            ui.centered_and_justified(|ui| {
                ui.add(egui::Spinner::new());
            });
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
                if pending_click.is_some() {
                    log::error!(
                        "app::{}: dropping pending pane click — render request send failed: {error}",
                        self.app_id
                    );
                }
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
                pending_click,
                Some(crate::ui::focus::SurfaceKey::Pane(pane_key)),
            );
            self.perf_ui_render += render_started.elapsed();
            self.perf_canvas_render += result.canvas_time;
            for action in result.actions {
                log::info!(
                    "app::{}: emitted UI action at Python host boundary handler={action}",
                    self.app_id
                );
                self.send_to_runtime(&json!({"type": "ui_action", "handler_id": action}));
            }
            for (handler_id, value) in result.value_changes {
                self.send_to_runtime(&json!({
                    "type": "text_submitted", "id": handler_id, "value": value
                }));
            }
            for click in result.canvas_clicks {
                self.send_to_runtime(&json!({
                    "type": "mouse",
                    "x": click.x,
                    "y": click.y,
                    "button": click.button,
                    "pressed": click.pressed,
                }));
            }
        } else {
            if pending_click.is_some() {
                log::info!(
                    "app::{}: deferring pending pane click — no committed tree yet",
                    self.app_id
                );
                self.pending_click_carry = pending_click;
            }
            ui.centered_and_justified(|ui| {
                ui.add(egui::Spinner::new());
            });
        }
        self.record_render_perf(host_frame_started.elapsed());
        let now = std::time::Instant::now();
        let render_deadline = self.frame_scheduler.next_repaint_deadline(now);
        let timer_deadline = self.timers.values().map(|timer| timer.deadline).min();
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
            self.send_to_runtime(&json!({
                "type": "http_response", "request_id": request_id,
                "status": response.status, "body": response.body, "error": response.error,
                "truncated": response.truncated, "headers": response.response_headers,
            }));
        }
        while let Ok((request_id, outcome)) = self.picker_rx.try_recv() {
            match outcome {
                crate::host::services::FilePickOutcome::Picked(paths) => {
                    let granted = self.register_picked_grants(&paths);
                    if granted.is_empty() {
                        log::error!(
                            "app::{}: file pick {request_id}: no picked path could be granted; cancelling",
                            self.app_id
                        );
                        self.queue_outbound_event(
                            crate::app_protocol::PlexiEvent::FilePickCancelled { request_id },
                        );
                    } else {
                        let paths = granted
                            .iter()
                            .map(|path| path.display().to_string())
                            .collect();
                        self.queue_outbound_event(crate::app_protocol::PlexiEvent::FilePicked {
                            request_id,
                            paths,
                        });
                    }
                }
                crate::host::services::FilePickOutcome::Cancelled => {
                    log::info!("app::{}: file pick {request_id} cancelled", self.app_id);
                    self.queue_outbound_event(
                        crate::app_protocol::PlexiEvent::FilePickCancelled { request_id },
                    );
                }
            }
        }
        loop {
            match self.decoder.try_recv() {
                Ok(DecodedOutput::Tree {
                    frame_id,
                    tree,
                    json_time,
                    tree_time,
                    bytes,
                }) => {
                    self.perf_json_decode += json_time;
                    self.perf_tree_decode += tree_time;
                    self.perf_stdout_bytes += bytes;
                    let frame_id =
                        frame_id.or_else(|| self.frame_scheduler.oldest_pending_frame_id());
                    if let Some(frame_id) = frame_id {
                        self.pending_trees.insert(frame_id, tree);
                    }
                }
                Ok(DecodedOutput::Message {
                    value,
                    json_time,
                    bytes,
                }) => {
                    self.perf_json_decode += json_time;
                    self.perf_stdout_bytes += bytes;
                    self.handle_message(value);
                }
                Ok(DecodedOutput::DecodeError(error)) => {
                    log::error!("app::{}: decode CPython WASM message: {error}", self.app_id);
                    self.error = Some(error);
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    log::error!("app::{}: CPython WASM decoder thread ended", self.app_id);
                    break;
                }
            }
        }
        let stderr = self.runtime.drain_stderr();
        let mut records = self.stderr_classifier.push(&stderr);
        if self.runtime.is_finished() {
            // Nothing more will terminate a half-line, so report it now
            // rather than losing the last line of a crash.
            records.extend(self.stderr_classifier.flush());
        }
        for record in records {
            self.log_guest_stderr(&record);
        }
        // The classifier above is for the host log; fatal-state detection
        // still runs off the raw stderr blob so a traceback that dies at
        // import surfaces through `self.error` (stint 0638).
        self.record_runtime_stderr(&stderr);
        if !self.pending_traceback_stderr.is_empty() {
            // The runtime may have exited after the last stderr push; that
            // finalizes a trailing partial line.
            self.refresh_guest_error();
        }
    }

    /// One host record per guest stderr line, each carrying the `app::<id>`
    /// prefix so the whole traceback is greppable by app (stint 0643).
    fn log_guest_stderr(&self, record: &GuestStderrRecord) {
        match record.kind {
            GuestStderrKind::BenignWasiStartup => log::debug!(
                "app::{} CPython WASM stderr (benign WASI startup noise): {}",
                self.app_id,
                record.line
            ),
            GuestStderrKind::TracebackException => log::error!(
                "app::{} CPython WASM stderr exception: {}",
                self.app_id,
                record.line
            ),
            GuestStderrKind::TracebackExceptionDetail => log::error!(
                "app::{} CPython WASM stderr exception (cont): {}",
                self.app_id,
                record.line
            ),
            GuestStderrKind::Payload
            | GuestStderrKind::TracebackHeader
            | GuestStderrKind::TracebackFrame
            | GuestStderrKind::TracebackChainSeparator => {
                log::error!("app::{} CPython WASM stderr: {}", self.app_id, record.line)
            }
        }
    }

    /// Poll runtime output from the host logic pass as well as from `ui()`.
    /// eframe suppresses `ui()` for an occluded host, but pane-state IPC must
    /// still observe a guest's fatal traceback before serving its response.
    pub(crate) fn poll_runtime_state(&mut self) {
        self.drain_runtime();
    }

    /// Handle one non-tree bridge message. Tree framing (`component_tree` /
    /// `tree_delta`) is resolved on the decoder thread and never reaches here.
    fn handle_message(&mut self, message: Value) {
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
            Some("save_app_state") => {
                self.save_state(message.get("scope"), message.get("payload"))
            }
            Some("file_read") => self.handle_file_read(&message),
            Some("file_write") => self.handle_file_write(&message),
            Some("open_file_picker") => self.handle_open_file_picker(&message),
            Some("read_host_log") => self.handle_read_host_log(&message),
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

    fn has_capability(&self, capability: &str) -> bool {
        self.config
            .capabilities
            .iter()
            .any(|item| item == capability)
    }

    fn workspace_path(&self, raw: &str, for_write: bool) -> Result<PathBuf, String> {
        resolve_app_fs_path(
            &self.config.workspace_root,
            &self.granted_fs_roots,
            raw,
            for_write,
        )
    }

    /// Service one `open_file_picker` request (stint 0508). The dialog (or
    /// scripted queue) runs on a background thread; the outcome re-enters the
    /// pane through `picker_rx` in `drain_runtime`, where grants are
    /// registered before `FilePicked` reaches the app.
    fn handle_open_file_picker(&mut self, message: &Value) {
        use crate::host::services::FilePickRequest;
        let request_id = message
            .get("request_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if request_id.is_empty() {
            log::error!(
                "app::{}: open_file_picker missing request_id; cancelling",
                self.app_id
            );
            self.queue_outbound_event(crate::app_protocol::PlexiEvent::FilePickCancelled {
                request_id,
            });
            return;
        }
        if !self.has_capability("fs.pick") {
            log::info!(
                "app::{}: open_file_picker {request_id} denied: missing capability fs.pick",
                self.app_id
            );
            self.queue_outbound_event(crate::app_protocol::PlexiEvent::FilePickCancelled {
                request_id,
            });
            return;
        }
        let filter: Vec<String> = message
            .get("filter")
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        let multiple = message
            .get("multiple")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let mode = match message.get("mode") {
            None | Some(Value::Null) => crate::app_protocol::FilePickerMode::default(),
            Some(value) => match serde_json::from_value(value.clone()) {
                Ok(mode) => mode,
                Err(error) => {
                    log::error!(
                        "app::{}: open_file_picker {request_id}: invalid mode {value}: {error}; cancelling",
                        self.app_id
                    );
                    self.queue_outbound_event(
                        crate::app_protocol::PlexiEvent::FilePickCancelled { request_id },
                    );
                    return;
                }
            },
        };
        log::info!(
            "app::{}: open_file_picker {request_id} mode={mode:?} multiple={multiple} filter={filter:?}",
            self.app_id
        );
        let picker = Arc::clone(&self.picker);
        let tx = self.picker_tx.clone();
        std::thread::spawn(move || {
            let outcome = picker.pick(&FilePickRequest {
                filter,
                multiple,
                mode,
            });
            if tx.send((request_id, outcome)).is_err() {
                log::debug!("CPython WASM picker outcome dropped after pane closed");
            }
        });
    }

    /// Register picked paths as per-pane fs grants and return the canonical
    /// paths to deliver to the app. Paths that cannot be resolved (deleted
    /// between pick and delivery, unreachable parent) are skipped loudly.
    fn register_picked_grants(&mut self, paths: &[PathBuf]) -> Vec<PathBuf> {
        let mut granted = Vec::new();
        for path in paths {
            match canonicalize_picked_path(path) {
                Ok(resolved) => {
                    log::info!(
                        "app::{}: fs grant registered for picked path {}",
                        self.app_id,
                        resolved.display()
                    );
                    self.granted_fs_roots.push(resolved.clone());
                    granted.push(resolved);
                }
                Err(error) => {
                    log::error!(
                        "app::{}: picked path {} not granted: {error}",
                        self.app_id,
                        path.display()
                    );
                }
            }
        }
        granted
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
                    std::fs::read(&path)
                        .map_err(|error| format!("read {}: {error}", path.display()))
                        .and_then(|bytes| {
                            if bytes.len() > crate::host::MAX_FILE_IO_BYTES {
                                Err(format!(
                                    "read {}: file is {} bytes, over the {}-byte per-call file I/O limit",
                                    path.display(),
                                    bytes.len(),
                                    crate::host::MAX_FILE_IO_BYTES
                                ))
                            } else {
                                Ok(bytes)
                            }
                        })
                })
        };
        // Bytes cross the JSON bridge base64-encoded so the round trip is
        // binary-exact; the SDK runtime decodes `content_b64` back to bytes.
        let response = match result {
            Ok(bytes) => {
                log::info!(
                    "app::{}: file_read {:?} -> {} bytes",
                    self.app_id,
                    message.get("path").and_then(Value::as_str).unwrap_or("?"),
                    bytes.len()
                );
                json!({"type": "file_read_result", "content_b64": BASE64.encode(bytes)})
            }
            Err(error) => json!({"type": "file_read_result", "error": error}),
        };
        self.send_to_runtime(&response);
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
                    decode_file_write_content(message).and_then(|bytes| {
                        log::info!(
                            "app::{}: file_write {} ({} bytes, binary={})",
                            self.app_id,
                            path.display(),
                            bytes.len(),
                            message.get("content_b64").is_some()
                        );
                        std::fs::write(&path, bytes)
                            .map_err(|error| format!("write {}: {error}", path.display()))
                    })
                })
        };
        let response = match result {
            Ok(()) => json!({"type": "file_write_result"}),
            Err(error) => json!({"type": "file_write_result", "error": error}),
        };
        self.send_to_runtime(&response);
    }

    /// Serve a capability-gated `read_host_log` request (stint 0444). The host
    /// owns log-path resolution: it tails its own channel log
    /// (`crate::platform::logging::log_path()`), so the sandboxed app never
    /// names or opens the file and the WASI mounts stay limited to the SDK and
    /// app dir. The resolved path is echoed back so the app can show it in its
    /// empty/error state. Both success and failure surface distinctly — a log
    /// the app cannot reach must never render as a silent blank pane.
    fn handle_read_host_log(&mut self, message: &Value) {
        let path = crate::platform::logging::log_path();
        let result = if !self.has_capability("logs.read") {
            Err("missing capability logs.read".to_string())
        } else {
            let max_bytes = message
                .get("max_bytes")
                .and_then(Value::as_u64)
                .map(|value| value as usize)
                .unwrap_or(DEFAULT_HOST_LOG_TAIL_BYTES);
            read_host_log_tail(&path, max_bytes)
        };
        let response = match result {
            Ok(content) => json!({
                "type": "host_log_result",
                "content": content,
                "path": path.display().to_string(),
            }),
            Err(error) => json!({
                "type": "host_log_result",
                "error": error,
                "path": path.display().to_string(),
            }),
        };
        if let Err(error) = self.runtime.send(&response) {
            log::error!("app::{}: send host_log_result: {error}", self.app_id);
        }
    }

    fn handle_http_request(&mut self, message: &Value) {
        let request_id = message
            .get("request_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if !self.has_capability("net.http") {
            self.send_to_runtime(&json!({"type": "http_response", "request_id": request_id, "error": "missing capability net.http"}));
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
        if !crate::host::services::http_host_allowed(&url, &self.config.allowed_hosts) {
            self.send_to_runtime(&json!({"type": "http_response", "request_id": request_id, "error": "host is not in manifest allowed_hosts"}));
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
                crate::host::services::DEFAULT_MAX_HTTP_BODY_BYTES,
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
        self.send_to_runtime(
            &json!({"type": "capability_decision", "capability": capability, "granted": granted}),
        );
    }

    /// The app's default state scope: the first declared entry. The scope
    /// list is validated non-empty at manifest parse; an empty list here can
    /// only come from a hand-built test config and falls back to global.
    fn default_scope(&self) -> crate::host::state_scope::StateScope {
        self.config
            .state_scopes
            .first()
            .copied()
            .unwrap_or(crate::host::state_scope::StateScope::Global)
    }

    fn scope_names(&self) -> Vec<&'static str> {
        self.config
            .state_scopes
            .iter()
            .map(|scope| scope.as_str())
            .collect()
    }

    fn default_scope_state(&self) -> serde_json::Map<String, Value> {
        self.persisted_states
            .get(&self.default_scope())
            .map(|state| state.values.clone())
            .unwrap_or_default()
    }

    fn states_json(&self) -> Value {
        Value::Object(
            self.config
                .state_scopes
                .iter()
                .map(|scope| {
                    (
                        scope.as_str().to_string(),
                        Value::Object(
                            self.persisted_states
                                .get(scope)
                                .map(|state| state.values.clone())
                                .unwrap_or_default(),
                        ),
                    )
                })
                .collect(),
        )
    }

    /// Every declared scope's state file, resolved against the *current*
    /// context root. The watcher registration in `pane_ops::create` re-syncs
    /// against this each drain pass, so `plexi context set-root` follows.
    pub fn state_paths(&self) -> Vec<(crate::host::state_scope::StateScope, PathBuf)> {
        self.config
            .state_scopes
            .iter()
            .filter_map(|&scope| {
                match python_state_path(
                    &self.app_id,
                    scope,
                    self.config.state_format,
                    &self.context_root,
                ) {
                    Ok(path) => Some((scope, path)),
                    Err(error) => {
                        log::error!(
                            "app::{}: cannot resolve state path for scope {}: {error}",
                            self.app_id,
                            scope.as_str()
                        );
                        None
                    }
                }
            })
            .collect()
    }

    /// Tell the runtime a scope's state changed outside its own persist flow
    /// (an external edit, or a persist that lost to one). The SDK replaces
    /// the scope's values wholesale and dispatches `events.StateChanged`.
    fn send_state_changed(
        &self,
        scope: crate::host::state_scope::StateScope,
        values: &serde_json::Map<String, Value>,
        error: Option<&str>,
    ) {
        self.send_to_runtime(&json!({
            "type": "state_changed",
            "scope": scope.as_str(),
            "payload": Value::Object(values.clone()),
            "error": error,
            "source": "external",
        }));
    }

    /// Re-read one scope's state file after an external change notification.
    ///
    /// No-ops when the file's `(mtime, len)` identity matches the cached pair
    /// — this is what suppresses watcher echoes of our own atomic writes. On
    /// a real change the on-disk values replace the scope's values wholesale
    /// (disk wins; deleted keys vanish) and the app is notified via
    /// `state_changed`. On a decode failure the previous values are kept, the
    /// scope's `error` is set (blocking persists), and the app is notified
    /// with the error attached.
    pub fn apply_external_state(&mut self, scope: crate::host::state_scope::StateScope) {
        if !self.config.state_scopes.contains(&scope) {
            log::warn!(
                "app::{}: external state change for undeclared scope '{}' ignored",
                self.app_id,
                scope.as_str()
            );
            return;
        }
        let path = match python_state_path(
            &self.app_id,
            scope,
            self.config.state_format,
            &self.context_root,
        ) {
            Ok(path) => path,
            Err(error) => {
                log::error!(
                    "app::{}: cannot resolve state path for scope {}: {error}",
                    self.app_id,
                    scope.as_str()
                );
                return;
            }
        };
        let (mtime, len) = stat_state_file(&path);
        let cached = self.persisted_states.entry(scope).or_default();
        let unchanged_existing = mtime.is_some() && mtime == cached.mtime && len == cached.len;
        let still_absent = mtime.is_none() && cached.mtime.is_none() && cached.error.is_none();
        if unchanged_existing || still_absent {
            log::debug!(
                "app::{}: external state notification for scope {} matches cached identity — \
                 self-write echo, ignoring",
                self.app_id,
                scope.as_str()
            );
            return;
        }
        let fresh = read_python_state_file(&self.app_id, &path, self.config.state_format);
        match &fresh.error {
            None => {
                log::info!(
                    "app::{}: state scope={} changed on disk ({}) — replacing in-memory state \
                     and notifying app",
                    self.app_id,
                    scope.as_str(),
                    path.display()
                );
                *cached = fresh;
                let values = cached.values.clone();
                self.send_state_changed(scope, &values, None);
            }
            Some(error) => {
                log::error!(
                    "app::{}: external change to state scope={} ({}) is unreadable: {error} — \
                     keeping previous values; persists blocked until the file is fixed",
                    self.app_id,
                    scope.as_str(),
                    path.display()
                );
                cached.error = Some(error.clone());
                cached.mtime = fresh.mtime;
                cached.len = fresh.len;
                let values = cached.values.clone();
                let message = error.clone();
                self.send_state_changed(scope, &values, Some(&message));
            }
        }
    }

    fn save_state(&mut self, scope_raw: Option<&Value>, payload: Option<&Value>) {
        let Some(payload) = payload.and_then(Value::as_object) else {
            return;
        };
        // Missing scope on the wire means the app's default scope. A scope
        // the app did not declare is an error — never a silent fallback to
        // another scope's file.
        let scope = match scope_raw.and_then(Value::as_str) {
            None => self.default_scope(),
            Some(raw) => match crate::host::state_scope::StateScope::parse(raw) {
                Ok(scope) => scope,
                Err(error) => {
                    log::error!(
                        "app::{}: save_app_state rejected — {error}; state NOT persisted",
                        self.app_id
                    );
                    return;
                }
            },
        };
        if !self.config.state_scopes.contains(&scope) {
            log::error!(
                "app::{}: save_app_state rejected — scope '{}' is not declared in \
                 [state] scopes {:?}; state NOT persisted",
                self.app_id,
                scope.as_str(),
                self.scope_names()
            );
            return;
        }
        if let Some(error) = self
            .persisted_states
            .get(&scope)
            .and_then(|state| state.error.as_ref())
        {
            log::error!(
                "app::{}: save_app_state refused — scope '{}' has an unresolved file error \
                 ({error}); fix the file on disk, a successful re-read clears this",
                self.app_id,
                scope.as_str()
            );
            return;
        }
        let path = match python_state_path(
            &self.app_id,
            scope,
            self.config.state_format,
            &self.context_root,
        ) {
            Ok(path) => path,
            Err(error) => {
                log::error!(
                    "app::{}: save_app_state rejected — {error}; state NOT persisted",
                    self.app_id
                );
                return;
            }
        };
        // Read-back-before-write (disk wins): if the file changed since we
        // last synced with it, an external writer got there first — drop
        // this persist, reload from disk, and tell the app so it can
        // re-apply its change on top of the fresh state.
        let (disk_mtime, disk_len) = stat_state_file(&path);
        let cached_identity = self
            .persisted_states
            .get(&scope)
            .map(|state| (state.mtime, state.len))
            .unwrap_or((None, None));
        if disk_mtime.is_some() && (disk_mtime, disk_len) != cached_identity {
            log::warn!(
                "app::{}: state scope={} ({}) changed on disk since load — reloading instead \
                 of overwriting; persist dropped",
                self.app_id,
                scope.as_str(),
                path.display()
            );
            let fresh = read_python_state_file(&self.app_id, &path, self.config.state_format);
            let error = fresh.error.clone();
            let values = fresh.values.clone();
            self.persisted_states.insert(scope, fresh);
            self.send_state_changed(scope, &values, error.as_deref());
            return;
        }
        let bytes = match encode_state_file(payload, self.config.state_format) {
            Ok(bytes) => bytes,
            Err(error) => {
                log::error!(
                    "app::{}: save_app_state rejected — {error}; state NOT persisted",
                    self.app_id
                );
                return;
            }
        };
        if let Err(error) =
            crate::host::state_scope::assert_within_scope(&path, scope, &self.context_root)
        {
            log::error!(
                "app::{}: save_app_state rejected — {error}; state NOT persisted",
                self.app_id
            );
            return;
        }
        if scope == crate::host::state_scope::StateScope::Context {
            // A user must never be able to accidentally commit their app
            // state with a project (standing ruling; personal local data).
            if let Err(error) =
                crate::workspace::secrets::ensure_app_state_gitignore(&self.context_root)
            {
                log::warn!(
                    "app::{}: could not ensure {}/.plexi/.gitignore covers app_states/: {error}",
                    self.app_id,
                    self.context_root.display()
                );
            }
        }
        match crate::host::state_scope::atomic_write(&path, &bytes) {
            Ok(()) => {
                // Re-stat AFTER the rename so the cached identity is the
                // file we just produced — statting before the rename would
                // make every self-write look external and loop reloads.
                let (mtime, len) = stat_state_file(&path);
                self.persisted_states.insert(
                    scope,
                    ScopeState {
                        values: payload.clone(),
                        mtime,
                        len,
                        error: None,
                    },
                );
                log::info!(
                    "app::{}: persisted state scope={} format={} to {}",
                    self.app_id,
                    scope.as_str(),
                    self.config.state_format.as_str(),
                    path.display()
                );
            }
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
            for event in &events {
                log::info!(
                    "app::{}: received key at Python host boundary key={} pressed={}",
                    self.app_id,
                    event["key"].as_str().unwrap_or("<missing>"),
                    event["pressed"].as_bool().unwrap_or(false),
                );
            }
            if let Err(error) = self.runtime.send(&json!({
                "type": "key_events",
                "events": events,
            })) {
                log::error!(
                    "app::{}: send key events to CPython WASM: {error}",
                    self.app_id
                );
                self.error = Some(error.to_string());
            }
            crate::app::app_trait::KeyDisposition::Consumed
        }
    }

    pub fn wants_close(&self) -> bool {
        self.wants_close
    }

    pub fn queue_outbound_event(&mut self, event: crate::app_protocol::PlexiEvent) {
        cache_python_theme_for_relaunch(&mut self.config, &event);
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

    /// Service the guest while the pane is not rendering (stint 0688).
    ///
    /// `ui` used to be the only caller of `drain_runtime`, so a pane in an
    /// inactive context or under an occluded window read nothing back from its
    /// subprocess. That is invisible for anything the guest only says while
    /// being painted, and fatal for anything it says on its own: an assistant
    /// `ToolCall` is written straight into the guest's stdin and answered
    /// straight back out, with no render involved on either side, so every tool
    /// call to a pane that was not painting sat in the decoder channel until
    /// the broker's 30s timeout fired.
    ///
    /// This drains only. It deliberately does not call `fire_due_timers`:
    /// timers push onto `pending_timer_events`, which is flushed solely by the
    /// render request in `ui`, so firing them here would grow that queue
    /// without bound behind an off-screen repeating timer. Delivering timers to
    /// an unpainted pane is a separate capability, not part of unwedging the
    /// command path.
    pub fn background_tick(&mut self) {
        let before = self.pending_commands.len();
        self.drain_runtime();
        let produced = self.pending_commands.len().saturating_sub(before);
        if produced > 0 {
            log::info!(
                "app::{}: serviced {produced} command(s) off screen — pane is not rendering",
                self.app_id
            );
        }
    }

    /// Whether [`Self::background_tick`] can currently make progress. Cheap and
    /// non-consuming — the host asks this of every off-screen pane every frame.
    pub fn needs_background_tick(&self) -> bool {
        self.decoder.has_queued_output() || !self.pending_commands.is_empty()
    }
    pub fn display_name(&self) -> String {
        self.title.clone().unwrap_or_else(|| self.app_id.clone())
    }

    pub(crate) fn semantic_state(&self) -> crate::host::pane::SemanticPaneState {
        python_semantic_state(self.tree.as_deref())
    }
    pub(crate) fn lifecycle(&self) -> (&'static str, Option<&str>) {
        python_lifecycle(
            self.error.as_deref(),
            self.runtime.is_finished(),
            self.tree.is_some(),
        )
    }

    fn record_runtime_stderr(&mut self, stderr: &str) {
        if self.pending_traceback_stderr.is_empty()
            && !stderr.contains("Traceback (most recent call last):")
        {
            return;
        }
        self.pending_traceback_stderr.push_str(stderr);
        self.refresh_guest_error();
    }

    fn refresh_guest_error(&mut self) {
        let Some(exception) =
            pending_traceback_exception(&self.pending_traceback_stderr, self.runtime.is_finished())
        else {
            return;
        };
        if self.error.as_deref() != Some(exception) {
            let exception = exception.to_string();
            log::info!(
                "app::{}: guest entered failed state: {exception}",
                self.app_id
            );
            self.error = Some(exception);
        }
    }

    #[cfg(test)]
    pub fn has_rendered_tree(&self) -> bool {
        self.tree.is_some()
    }
    #[cfg(test)]
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    #[cfg(test)]
    pub fn record_runtime_stderr_for_test(&mut self, stderr: &str) {
        self.record_runtime_stderr(stderr);
    }
    pub fn relaunch(&mut self) -> Result<(), WasmPythonError> {
        // Drop the old decoder (closes its stdout, joins the thread) before the
        // old runtime is replaced, then bind a fresh decoder to the new
        // runtime's stdout. The shared repaint hook carries over so the new
        // decoder can wake the paint loop immediately.
        let runtime = WasmPythonRuntime::launch(&self.config)?;
        self.decoder =
            PythonOutputDecoder::spawn(&runtime, self.app_id.clone(), self.repaint.clone());
        self.runtime = runtime;
        self.tree = None;
        self.pending_trees.clear();
        self.initialized = false;
        self.ready = false;
        self.frame_scheduler.reset(std::time::Instant::now());
        self.error = None;
        self.pending_traceback_stderr.clear();
        self.wants_close = false;
        self.timers.clear();
        self.pending_timer_events.clear();
        self.viewport_size = None;
        // A fresh runtime is a fresh stderr stream: carrying the old guest's
        // half-line or traceback state into it would misclassify the new
        // guest's first lines.
        self.stderr_classifier = GuestStderrClassifier::new();
        if let Some(dropped) = self.pending_click_carry.take() {
            // A node-targeted click's arena id belongs to the tree it was
            // resolved against — that tree is gone after relaunch, so
            // replaying it against the new one would hit the wrong node (or
            // nothing at all). Fail loud instead of a silent stale-node
            // click, matching the same policy as the other drop points above.
            log::error!(
                "app::{}: dropping pending pane click across hot-reload relaunch (target={:?}) — the tree it targeted no longer exists",
                self.app_id,
                dropped.target
            );
        }
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

/// Stint 0417: CPython-in-WASI prints `Could not find platform dependent
/// libraries <exec_prefix>` to stderr on every guest boot even when the
/// runtime starts and runs fine — WASI has no real filesystem layout for
/// CPython's `sysconfig` probe to find. Logging that line at ERROR trains
/// agents and humans to ignore real guest tracebacks in the same stream, so
/// it is demoted to DEBUG per line.
const BENIGN_WASI_STARTUP_SUBSTRINGS: &[&str] = &["Could not find platform dependent libraries"];
const TRACEBACK_HEADER: &str = "Traceback (most recent call last):";
/// The two sentences CPython prints between a chained pair of tracebacks.
/// Each is followed by a fresh `Traceback` header, so they close the block
/// they follow rather than continuing it.
const TRACEBACK_CHAIN_SEPARATORS: &[&str] = &[
    "During handling of the above exception, another exception occurred:",
    "The above exception was the direct cause of the following exception:",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GuestStderrKind {
    BenignWasiStartup,
    Payload,
    TracebackHeader,
    TracebackFrame,
    TracebackChainSeparator,
    /// The line carrying the exception type and message — what a reader is
    /// actually looking for, and the only kind that gets the `exception:`
    /// marker in the log.
    TracebackException,
    /// Continuation lines of a multi-line exception message. Attributed to
    /// the exception but deliberately not marked as one: an exception has
    /// exactly one `exception:` line, however many lines its message spans.
    TracebackExceptionDetail,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GuestStderrRecord {
    kind: GuestStderrKind,
    line: String,
}

/// Longest unterminated stderr fragment held back waiting for its newline.
const MAX_BUFFERED_STDERR_LINE: usize = 64 * 1024;

/// Where the stderr stream currently sits inside a CPython traceback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TracebackState {
    Outside,
    /// Inside the indented frame block that follows a `Traceback` header.
    Frames,
    /// Past the exception line, consuming the rest of its message.
    ExceptionMessage,
}

/// Classifies a guest's stderr into one host log record per line.
///
/// Streaming and stateful on purpose. `drain_stderr` hands back whatever bytes
/// happen to have arrived, so a single traceback routinely spans several
/// drains and a drain can even split a line mid-way. Any rule that reads a
/// drain as a self-contained block — "the exception is the last line I can
/// see" — therefore marks whichever frame line ended the *chunk*, and the real
/// exception arrives later unmarked. Roles are decided from each line's own
/// shape against carried-over state instead: a frame line is indented, an
/// exception line is not, which is what structurally distinguishes them.
struct GuestStderrClassifier {
    /// Bytes after the last newline, held until the rest of the line arrives.
    partial: String,
    state: TracebackState,
}

impl GuestStderrClassifier {
    fn new() -> Self {
        Self {
            partial: String::new(),
            state: TracebackState::Outside,
        }
    }

    /// Classify every complete line in `chunk`, holding back a trailing
    /// unterminated one for the next drain.
    fn push(&mut self, chunk: &str) -> Vec<GuestStderrRecord> {
        self.partial.push_str(chunk);
        let mut records = Vec::new();
        while let Some(newline) = self.partial.find('\n') {
            let line: String = self.partial.drain(..=newline).collect();
            if let Some(record) = self.classify(line.trim_end()) {
                records.push(record);
            }
        }
        // A guest writing without newlines must not buffer forever: flush the
        // oversized fragment as its own record rather than holding the log
        // hostage to a line that never terminates.
        if self.partial.len() > MAX_BUFFERED_STDERR_LINE {
            records.extend(self.flush());
        }
        records
    }

    /// Emit the buffered unterminated line. Called once the guest is gone, so
    /// a crash whose last line lacks a trailing newline is still reported.
    fn flush(&mut self) -> Option<GuestStderrRecord> {
        let line = std::mem::take(&mut self.partial);
        self.classify(line.trim_end())
    }

    fn classify(&mut self, line: &str) -> Option<GuestStderrRecord> {
        if line.trim().is_empty() {
            // A blank line closes a traceback block — CPython separates a
            // chained traceback from the next with one. It carries nothing a
            // reader needs, so it steers state and is never logged.
            if self.state == TracebackState::ExceptionMessage {
                self.state = TracebackState::Outside;
            }
            return None;
        }
        let trimmed = line.trim();
        let indented = line.starts_with([' ', '\t']);
        let kind = if BENIGN_WASI_STARTUP_SUBSTRINGS
            .iter()
            .any(|needle| trimmed.contains(needle))
        {
            GuestStderrKind::BenignWasiStartup
        } else if trimmed == TRACEBACK_HEADER {
            self.state = TracebackState::Frames;
            GuestStderrKind::TracebackHeader
        } else if TRACEBACK_CHAIN_SEPARATORS.contains(&trimmed) {
            self.state = TracebackState::Outside;
            GuestStderrKind::TracebackChainSeparator
        } else {
            match self.state {
                // The frame block ends at the first line that is not indented,
                // and in CPython's format that line is the exception. Shape,
                // not position: `File "x.py", line 1, in f` and the source and
                // caret lines under it are indented, `RuntimeError: boom` is
                // not — which also lands a bare `KeyboardInterrupt` correctly.
                TracebackState::Frames if !indented => {
                    self.state = TracebackState::ExceptionMessage;
                    GuestStderrKind::TracebackException
                }
                TracebackState::Frames => GuestStderrKind::TracebackFrame,
                TracebackState::ExceptionMessage => GuestStderrKind::TracebackExceptionDetail,
                // A compile-time `SyntaxError` has no `Traceback` header at
                // all: it opens straight into an indented `File` line. Keying
                // on the frame shape picks that up, and picks a traceback up
                // mid-stream if its header was already consumed.
                TracebackState::Outside if indented && is_traceback_frame_line(trimmed) => {
                    self.state = TracebackState::Frames;
                    GuestStderrKind::TracebackFrame
                }
                TracebackState::Outside => GuestStderrKind::Payload,
            }
        };
        Some(GuestStderrRecord {
            kind,
            line: trimmed.to_string(),
        })
    }
}

/// `File "<path>", line <n>[, in <name>]` — the one shape every CPython
/// traceback frame opens with, quoted path or angle-bracketed frozen module.
fn is_traceback_frame_line(trimmed: &str) -> bool {
    trimmed.starts_with("File ") && trimmed.contains(", line ")
}

/// Liveness has exactly one authoritative source: the runtime thread. A
/// committed tree only refines a live guest into starting vs running — it must
/// never decide dead vs alive, or a guest that dies after its first frame
/// keeps reporting `running` from its stale tree.
fn python_lifecycle(
    error: Option<&str>,
    runtime_finished: bool,
    has_frame: bool,
) -> (&'static str, Option<&str>) {
    match (error, runtime_finished, has_frame) {
        (Some(error), _, _) => ("failed", Some(error)),
        (None, true, _) => ("exited", None),
        (None, false, true) => ("running", None),
        (None, false, false) => ("starting", None),
    }
}

/// Extract the useful final exception line from accumulated traceback stderr.
/// The full stderr remains in the host log; this compact line is what the
/// failed pane and pane-state callers should lead with. While the guest is
/// alive only complete lines count — `DrainableOutput` may split a line across
/// drains; once the runtime has exited the buffer is final as-is.
fn pending_traceback_exception(buffer: &str, runtime_finished: bool) -> Option<&str> {
    let complete = if runtime_finished {
        buffer
    } else {
        &buffer[..=buffer.rfind('\n')?]
    };
    traceback_exception_line(complete)
}

/// The last exception-record line following a traceback header wins: chained
/// tracebacks ("During handling of the above exception, ...") put the
/// authoritative exception at the end.
fn traceback_exception_line(stderr: &str) -> Option<&str> {
    let mut after_header = false;
    let mut exception = None;
    for line in stderr.lines() {
        if line.starts_with("Traceback (most recent call last):") {
            after_header = true;
        } else if after_header && is_exception_record_line(line) {
            exception = Some(line.trim_end());
        }
    }
    exception
}

/// A Python exception record prints unindented as `Name: message` or a bare
/// `Name`, where `Name` is a (dotted) identifier. Frame lines are indented,
/// and chaining prose ("During handling of the above exception, ...") has
/// spaces before any colon, so neither can match.
fn is_exception_record_line(line: &str) -> bool {
    if line.starts_with(char::is_whitespace) {
        return false;
    }
    let head = line.split(':').next().unwrap_or_default();
    let mut chars = head.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
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
        egui::Key::Num0 => "0".to_string(),
        egui::Key::Num1 => "1".to_string(),
        egui::Key::Num2 => "2".to_string(),
        egui::Key::Num3 => "3".to_string(),
        egui::Key::Num4 => "4".to_string(),
        egui::Key::Num5 => "5".to_string(),
        egui::Key::Num6 => "6".to_string(),
        egui::Key::Num7 => "7".to_string(),
        egui::Key::Num8 => "8".to_string(),
        egui::Key::Num9 => "9".to_string(),
        egui::Key::Slash => "/".to_string(),
        egui::Key::Minus => "-".to_string(),
        egui::Key::Equals => "=".to_string(),
        egui::Key::Backslash => "\\".to_string(),
        egui::Key::Semicolon => ";".to_string(),
        egui::Key::Quote => "'".to_string(),
        egui::Key::Backtick => "`".to_string(),
        egui::Key::Comma => ",".to_string(),
        egui::Key::Period => ".".to_string(),
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
            // Command chords are host input, never guest input. This matches
            // the WASM runtime and remains a defense-in-depth guard if host
            // routing order changes. Bare Escape is also reserved for the host
            // CloseApp binding; forwarding it would let `handle_key` claim it.
            if modifiers.command || *key == egui::Key::Escape {
                return None;
            }
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

fn python_render_event(frame_id: u64, timer_ids: Vec<String>) -> Value {
    json!({
        "type": "render",
        "frame_id": frame_id,
        "timer_ids": timer_ids,
    })
}

fn commit_python_frame(
    scheduler: &mut PythonFrameScheduler,
    pending_trees: &mut HashMap<u64, Arc<PythonUiTree>>,
    visible_tree: &mut Option<Arc<PythonUiTree>>,
    frame_id: u64,
) -> Option<std::time::Instant> {
    let sent_at = scheduler.complete_frame(frame_id)?;
    if let Some(tree) = pending_trees.remove(&frame_id) {
        *visible_tree = Some(tree);
    }
    Some(sent_at)
}

/// Resolve an app-supplied relative path inside `root`, rejecting absolute
/// paths, `..` components, and symlink escapes. `for_write` permits a
/// not-yet-existing final component as long as its parent resolves inside the
/// jail. Shared by `file_read` and `file_write`; 0508's granted scopes will
/// extend the allowed roots here once they land.
fn resolve_jailed_path(root: &Path, raw: &str, for_write: bool) -> Result<PathBuf, String> {
    let path = Path::new(raw);
    if path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return Err(format!("path escapes workspace: {raw}"));
    }
    let canonical_root = root
        .canonicalize()
        .map_err(|error| format!("canonicalize workspace {}: {error}", root.display()))?;
    let candidate = root.join(path);
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
    if !resolved.starts_with(&canonical_root) {
        return Err(format!("path escapes workspace through symlink: {raw}"));
    }
    Ok(resolved)
}

/// Extract the payload of a `file_write` message: binary via `content_b64`
/// (base64, the binary-safe bridge encoding) or text via `content`. Exactly one
/// must be present; both silent-empty fallbacks and oversize payloads are
/// rejected with named errors.
fn decode_file_write_content(message: &Value) -> Result<Vec<u8>, String> {
    let bytes = match (
        message.get("content_b64").and_then(Value::as_str),
        message.get("content").and_then(Value::as_str),
    ) {
        (Some(_), Some(_)) => {
            return Err("file_write carries both content and content_b64; send exactly one".into())
        }
        (Some(b64), None) => BASE64
            .decode(b64)
            .map_err(|error| format!("file_write content_b64 is not valid base64: {error}"))?,
        (None, Some(text)) => text.as_bytes().to_vec(),
        (None, None) => {
            return Err("file_write missing content (text) or content_b64 (binary)".into())
        }
    };
    if bytes.len() > crate::host::MAX_FILE_IO_BYTES {
        return Err(format!(
            "file_write payload is {} bytes, over the {}-byte per-call file I/O limit",
            bytes.len(),
            crate::host::MAX_FILE_IO_BYTES
        ));
    }
    Ok(bytes)
}

/// Default tail window for a `read_host_log` request: the last 256 KiB. Bounded
/// so a multi-megabyte channel log never crosses the JSON bridge whole; the
/// logs app renders far fewer lines than this holds.
const DEFAULT_HOST_LOG_TAIL_BYTES: usize = 256 * 1024;

/// Read the tail of the host channel log for the capability-gated
/// `read_host_log` effect (stint 0444). Seeks to the last `max_bytes` and drops
/// the partial leading line so the app only ever parses whole records. Returns
/// a typed error string (never an empty success) when the log cannot be reached
/// so the app can render the failure instead of a blank pane.
fn read_host_log_tail(path: &Path, max_bytes: usize) -> Result<String, String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file =
        std::fs::File::open(path).map_err(|error| format!("open {}: {error}", path.display()))?;
    let len = file
        .metadata()
        .map_err(|error| format!("stat {}: {error}", path.display()))?
        .len();
    let max = max_bytes as u64;
    let trimmed_partial = len > max;
    if trimmed_partial {
        file.seek(SeekFrom::End(-(max as i64)))
            .map_err(|error| format!("seek {}: {error}", path.display()))?;
    }
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)
        .map_err(|error| format!("read {}: {error}", path.display()))?;
    let mut text = String::from_utf8_lossy(&buf).into_owned();
    if trimmed_partial {
        if let Some(newline) = text.find('\n') {
            text.drain(..=newline);
        }
    }
    Ok(text)
}

/// Resolve an app-supplied fs path against the workspace jail and picker
/// grants (stint 0508). This is the single chokepoint for CPython-WASM app fs
/// access: relative paths stay jailed to `workspace_root` via
/// `resolve_jailed_path` (stint 0509) exactly as before; absolute paths are
/// accepted only when they resolve under a picker-granted root, with the same
/// canonicalize + symlink-escape discipline, so a symlink inside either scope
/// cannot reach outside it.
fn resolve_app_fs_path(
    workspace_root: &Path,
    granted_roots: &[PathBuf],
    raw: &str,
    for_write: bool,
) -> Result<PathBuf, String> {
    let path = Path::new(raw);
    if path.is_absolute() {
        let resolved = resolve_concrete_path(path.to_path_buf(), for_write)?;
        if granted_roots.iter().any(|root| resolved.starts_with(root)) {
            return Ok(resolved);
        }
        return Err(format!(
            "absolute path is outside every picker-granted scope: {raw}"
        ));
    }
    resolve_jailed_path(workspace_root, raw, for_write)
}

/// Canonicalize a candidate path; for a write target that does not exist yet,
/// canonicalize its parent and re-attach the file name (same discipline the
/// workspace jail has always used for new files).
fn resolve_concrete_path(candidate: PathBuf, for_write: bool) -> Result<PathBuf, String> {
    let display = candidate.display().to_string();
    if for_write && !candidate.exists() {
        let parent = candidate
            .parent()
            .ok_or_else(|| format!("path has no parent: {display}"))?;
        let name = candidate
            .file_name()
            .ok_or_else(|| format!("path has no file name: {display}"))?
            .to_os_string();
        parent.canonicalize().map(|parent| parent.join(name))
    } else {
        candidate.canonicalize()
    }
    .map_err(|error| format!("resolve path {display}: {error}"))
}

/// Canonicalize a picker-returned path for grant registration. Save-as
/// targets may not exist yet, so a missing path resolves through its parent.
fn canonicalize_picked_path(path: &Path) -> Result<PathBuf, String> {
    resolve_concrete_path(path.to_path_buf(), true)
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
            title: text("title"),
            body: text("body"),
            kind: crate::app_protocol::NotifyKind::Message,
            options: Vec::new(),
            input_prompt: None,
            required: false,
            // The bridge message carries no scope, so it takes the shared
            // default rather than an invented one.
            scope: crate::app_protocol::NotifyScope::default(),
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
        // v3.7 tool protocol. The SDK names these `ExposeTools`/`ToolResult`
        // after the wire commands; the WIT effect variants are
        // `declare-tools`/`tool-result`. Schemas cross the bridge as JSON
        // objects and are re-serialized into the WIT string fields.
        "ExposeTools" => Ok(PythonBridgeEffect::Host(Effect::DeclareTools(
            DeclareToolsEffect {
                tools: value
                    .get("tools")
                    .and_then(Value::as_array)
                    .ok_or_else(|| {
                        WasmPythonError::BridgeJson(
                            "ExposeTools missing array 'tools'".to_string(),
                        )
                    })?
                    .iter()
                    .map(decode_tool_decl)
                    .collect::<Result<Vec<_>, _>>()?,
            },
        ))),
        "ToolResult" => Ok(PythonBridgeEffect::Host(Effect::ToolResult(
            ToolResultEffect {
                call_id: required_string(&value, "call_id")?,
                output_json: value
                    .get("output_json")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                error: value.get("error").and_then(Value::as_str).map(str::to_string),
            },
        ))),
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
        if let Some(fit) = canvas_fit_for_node(node)? {
            canvas_fits.insert(required_u32(node, "id")?, fit);
        }
    }
    Ok(PythonUiTree { tree, canvas_fits })
}

/// The `CanvasFit` an encoded node maps to, or `None` when it is not a canvas.
/// Shared by the full decode and the delta full-node-replace path so both keep
/// `canvas_fits` consistent.
fn canvas_fit_for_node(
    node: &Value,
) -> Result<Option<super::wasm_render::CanvasFit>, WasmPythonError> {
    let Some(data) = node.get("data") else {
        return Ok(None);
    };
    if !matches!(
        data.get("type").and_then(Value::as_str),
        Some("Canvas" | "canvas")
    ) {
        return Ok(None);
    }
    let fit = match data.get("fit").and_then(Value::as_str).unwrap_or("fill") {
        "fill" => super::wasm_render::CanvasFit::Fill,
        "contain" => super::wasm_render::CanvasFit::Contain,
        other => {
            return Err(WasmPythonError::BridgeJson(format!(
                "canvas fit must be 'fill' or 'contain', got {other:?}"
            )))
        }
    };
    Ok(Some(fit))
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
        "AppBar" | "app_bar" | "app-bar" => Ok(UiNodeData::AppBar(AppBarNode {
            title: required_string(value, "title")?,
            subtitle: value
                .get("subtitle")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        })),
        "FooterKeys" | "footer_keys" | "footer-keys" => {
            Ok(UiNodeData::FooterKeys(FooterKeysNode {
                entries: value
                    .get("entries")
                    .and_then(Value::as_array)
                    .ok_or_else(|| {
                        WasmPythonError::BridgeJson("missing array field 'entries'".to_string())
                    })?
                    .iter()
                    .map(decode_footer_key_entry)
                    .collect::<Result<Vec<_>, _>>()?,
                divider: value
                    .get("divider")
                    .and_then(Value::as_bool)
                    .unwrap_or(true),
            }))
        }
        "Pinned" | "pinned" => Ok(UiNodeData::Pinned(PinnedNode {
            edge: decode_pinned_edge(
                value
                    .get("edge")
                    .and_then(Value::as_str)
                    .unwrap_or("bottom"),
            )?,
            child: required_u32(value, "child")?,
        })),
        "Spinner" | "spinner" => Ok(UiNodeData::Spinner(SpinnerNode {
            label: value
                .get("label")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        })),
        "Column" | "column" => Ok(UiNodeData::Column(ColumnNode {
            children: u32_list(value, "children")?,
            // Default inter-child spacing when the app declares none. Absence
            // means "use the good default"; an explicit `gap: 0.0` still wins
            // and packs children flush (stint 0445).
            gap: optional_f32(value, "gap")?.unwrap_or(crate::ui::style::SPACE_MD),
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
            autofocus: value
                .get("autofocus")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        })),
        "Row" | "row" => Ok(UiNodeData::Row(RowNode {
            children: u32_list(value, "children")?,
            // Rows sit tighter than columns by default; absence uses the token,
            // an explicit `gap: 0.0` still packs children flush (stint 0445).
            gap: optional_f32(value, "gap")?.unwrap_or(crate::ui::style::SPACE_SM),
            align: decode_alignment(
                value
                    .get("align")
                    .and_then(Value::as_str)
                    .unwrap_or("start"),
            )?,
            grow: value.get("grow").and_then(Value::as_bool).unwrap_or(false),
        })),
        "Divider" | "divider" => Ok(UiNodeData::Divider),
        "Space" | "spacer" => Ok(UiNodeData::Space(SpaceNode {
            size: optional_f32(value, "size")?.unwrap_or(0.0),
            grow: value.get("grow").and_then(Value::as_bool).unwrap_or(false),
        })),
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

/// Decode one SDK `AiTool` payload into the WIT `tool-decl` record.
///
/// The Python side carries `input_schema`/`output_schema` as JSON objects while
/// WIT carries them as strings, so both are re-serialized here.
#[cfg(test)]
fn decode_tool_decl(value: &Value) -> Result<ToolDecl, WasmPythonError> {
    let schema = |field: &str| -> Result<String, WasmPythonError> {
        let schema = value.get(field).ok_or_else(|| {
            WasmPythonError::BridgeJson(format!("tool missing object field '{field}'"))
        })?;
        if !schema.is_object() {
            return Err(WasmPythonError::BridgeJson(format!(
                "tool field '{field}' must be a JSON object"
            )));
        }
        serde_json::to_string(schema).map_err(|e| WasmPythonError::BridgeJson(e.to_string()))
    };
    Ok(ToolDecl {
        name: required_string(value, "name")?,
        description: required_string(value, "description")?,
        input_schema_json: schema("input_schema")?,
        output_schema_json: schema("output_schema")?,
        timeout_ms: value.get("timeout_ms").and_then(Value::as_u64),
        read_only: value
            .get("read_only")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
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

fn decode_footer_key_entry(value: &Value) -> Result<FooterKeyEntry, WasmPythonError> {
    let keys = value
        .get("keys")
        .and_then(Value::as_array)
        .ok_or_else(|| WasmPythonError::BridgeJson("missing array field 'keys'".to_string()))?
        .iter()
        .map(|k| {
            k.as_str().map(str::to_string).ok_or_else(|| {
                WasmPythonError::BridgeJson("footer key entry 'keys' must be strings".to_string())
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(FooterKeyEntry {
        keys,
        description: value
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
    })
}

fn decode_pinned_edge(value: &str) -> Result<PinnedEdge, WasmPythonError> {
    match value {
        "bottom" => Ok(PinnedEdge::Bottom),
        "top" => Ok(PinnedEdge::Top),
        "left" => Ok(PinnedEdge::Left),
        "right" => Ok(PinnedEdge::Right),
        other => Err(WasmPythonError::BridgeJson(format!(
            "unknown pinned edge: {other}"
        ))),
    }
}

/// Decodes a badge color role. Accepts the canonical set (accent, success,
/// warning, danger, neutral) plus the theme's status-role aliases
/// (red/green/yellow — see `sdk/python/plexi_sdk/_theme.py`). There is
/// intentionally no "blue" role; `accent` is the accent/blue-ish role.
fn decode_badge_color(value: &str) -> Result<BadgeColor, WasmPythonError> {
    match value {
        "accent" => Ok(BadgeColor::Accent),
        "success" | "green" => Ok(BadgeColor::Success),
        "warning" | "yellow" => Ok(BadgeColor::Warning),
        "danger" | "red" => Ok(BadgeColor::Danger),
        "neutral" => Ok(BadgeColor::Neutral),
        other => Err(WasmPythonError::BridgeJson(format!(
            "unknown badge color: {other}"
        ))),
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

    fn state_test_config(workspace_root: &Path, app_id: &str) -> PythonLaunchConfig {
        PythonLaunchConfig {
            app_id: app_id.to_string(),
            app_dir: workspace_root.to_path_buf(),
            entry: workspace_root.join("main.py"),
            module_name: "main".to_string(),
            launch_args: Vec::new(),
            workspace_root: workspace_root.to_path_buf(),
            capabilities: Vec::new(),
            allowed_hosts: Vec::new(),
            theme: crate::ui::theme::colors_from_config(
                &crate::config::PlexiConfig::default(),
            )
            .to_theme_map(),
            // These tests exercise root-scoped addressing (same root -> same
            // file, different root -> different file), which is exactly
            // `StateScope::Context`'s contract — `Global` would ignore the
            // root entirely and defeat the premise of every test here.
            state_scopes: vec![crate::host::state_scope::StateScope::Context],
            state_format: crate::host::state_scope::StateFormat::Json,
            context_root: workspace_root.to_path_buf(),
        }
    }

    /// Test-only convenience: resolve the state path this fixture's single
    /// declared scope (`Context`, see `state_test_config`) addresses. Real
    /// callers use `python_state_path(app_id, scope, context_root)` directly
    /// once they have resolved which scope they mean; tests here only ever
    /// care about "the one scope this config declares".
    fn python_state_path_for_config(config: &PythonLaunchConfig) -> PathBuf {
        let scope = config
            .state_scopes
            .first()
            .copied()
            .unwrap_or(crate::host::state_scope::StateScope::Global);
        super::python_state_path(
            &config.app_id,
            scope,
            config.state_format,
            &config.context_root,
        )
        .expect("resolve state path")
    }

    #[test]
    fn python_init_payload_carries_the_active_host_theme() {
        let workspace = tempdir().expect("workspace");
        let mut config = state_test_config(workspace.path(), "test.theme-init");
        config.theme = std::collections::HashMap::from([
            ("fg".to_string(), "#123456".to_string()),
            ("bg".to_string(), "#abcdef".to_string()),
        ]);

        let payload = python_init_payload(&config, json!({}), (480.0, 320.0));

        assert_eq!(payload["theme"], json!(config.theme));
        assert!(
            payload["theme"]
                .as_object()
                .is_some_and(|theme| !theme.is_empty())
        );
    }

    #[test]
    fn theme_event_updates_the_cached_python_relaunch_theme() {
        let workspace = tempdir().expect("workspace");
        let mut config = state_test_config(workspace.path(), "test.theme-relaunch");
        let colors = std::collections::HashMap::from([(
            "fg".to_string(),
            "#123456".to_string(),
        )]);

        cache_python_theme_for_relaunch(
            &mut config,
            &crate::app_protocol::PlexiEvent::Theme {
                colors: colors.clone(),
            },
        );

        assert_eq!(config.theme, colors);
    }

    /// Stint 0678 audit evidence (Q2/Q5): two live instances of one app under
    /// the same root address one file, and the loser is the instance that
    /// persists *last* with the older in-memory map — not the one that wrote
    /// the stale bytes first. Replays the exact host sequence: each instance
    /// loads at launch (`load_python_state`), then persists its own map through
    /// the same writer `save_state` uses (`write_python_state_atomic`).
    ///
    /// Assertion discipline for every `audit_0678_*` test: see the comment on
    /// the `audit_0678` module below.
    #[test]
    fn audit_0678_second_instance_clobbers_the_first_instances_items() {
        // Isolate `config_dir()` so the global-state fallback in
        // `load_python_state` resolves inside a tempdir, never the real profile.
        let profile = tempdir().expect("profile");
        let _profile = crate::config::set_test_profile_dir(profile.path().join("profile"));
        let workspace = tempdir().expect("workspace");
        let first = state_test_config(workspace.path(), "todo");
        let second = state_test_config(workspace.path(), "todo");
        assert_eq!(
            python_state_path_for_config(&first),
            python_state_path_for_config(&second),
            "instance identity does not enter the state path — only app_id and workspace_root do"
        );

        // Both instances launch against an empty store and hold their own copy.
        let mut first_state = load_python_state(&first).expect("first launch load");
        let second_state = load_python_state(&second).expect("second launch load");
        assert!(first_state.is_empty() && second_state.is_empty());

        // The user adds an item in the first instance; it persists.
        first_state.insert("items".to_string(), json!(["buy milk"]));
        let path = python_state_path_for_config(&first);
        std::fs::create_dir_all(path.parent().expect("state parent")).expect("mkdir");
        crate::host::state_scope::atomic_write(
            &path,
            &serde_json::to_vec_pretty(&first_state).expect("serialize"),
        )
        .expect("first persist");
        assert_eq!(
            load_python_state(&second).expect("readback")["items"],
            json!(["buy milk"])
        );

        // The second instance persists anything at all — a draft keystroke is
        // enough — and writes the empty item list it has held since launch.
        crate::host::state_scope::atomic_write(
            &path,
            &serde_json::to_vec_pretty(&second_state).expect("serialize"),
        )
        .expect("second persist");

        let survivor = load_python_state(&first).expect("post-clobber load");
        assert!(
            survivor.get("items").is_none(),
            "the second instance's launch-time map overwrote the item: {survivor:?}"
        );
    }

    /// Stint 0678 audit evidence (Q4/Q5): state is addressed by
    /// `workspace_root`, so the same app id launched under a different root
    /// reads a different file and sees nothing. Nothing merges the two, and
    /// nothing warns — this is the "write to a path nobody reads" shape.
    #[test]
    fn audit_0678_state_written_under_one_root_is_invisible_under_another() {
        let profile = tempdir().expect("profile");
        let _profile = crate::config::set_test_profile_dir(profile.path().join("profile"));
        let root_a = tempdir().expect("root a");
        let root_b = tempdir().expect("root b");
        let under_a = state_test_config(root_a.path(), "todo");
        let under_b = state_test_config(root_b.path(), "todo");

        let path_a = python_state_path_for_config(&under_a);
        std::fs::create_dir_all(path_a.parent().expect("state parent")).expect("mkdir");
        crate::host::state_scope::atomic_write(&path_a, br#"{"items":["buy milk"]}"#).expect("persist under A");

        assert_eq!(
            load_python_state(&under_a).expect("load under A")["items"],
            json!(["buy milk"])
        );
        assert!(
            load_python_state(&under_b).expect("load under B").is_empty(),
            "a launch rooted elsewhere sees an empty store, not the items"
        );
        assert!(
            !python_state_path_for_config(&under_b).exists(),
            "reading under root B does not create or migrate anything"
        );
    }

    /// Stint 0678 audit evidence that needs a *live* app: a real CPython-WASM
    /// pane launched through the production path, so the copy of the root that
    /// actually owns the state path — `PythonLaunchConfig::workspace_root` —
    /// is the one under test.
    ///
    /// **Assertion discipline.** Two earlier audit tests in this set passed
    /// whether or not the defect existed. Both had the same shape: they
    /// asserted an *implementation site's current value* — a private helper
    /// returning `None`, a struct field that had not changed — instead of an
    /// invariant. A fix is free to add a new code path and leave that site
    /// exactly as it was, so such a test cannot tell a fixed system from a
    /// broken one, and its green tick is worth nothing to the doc that cites
    /// it.
    ///
    /// Every test here therefore asserts the **pair a fix must reconcile**:
    /// the stale value *and* its disagreement with the live source of truth it
    /// is supposed to equal. A fix must change one half or the other, so the
    /// pair cannot survive one. `assert_state_address_lost` is the sanctioned
    /// form for the address shape, and refuses to pass unless bytes genuinely
    /// exist to be stranded — an empty fixture cannot fake a loss.
    mod audit_0678 {
        use super::*;

        /// Fails unless an app's persisted bytes are genuinely stranded:
        /// `live` must hold real bytes, and `reachable` — the address the
        /// system can still resolve after the operation under test — must be
        /// a different, empty location. Fixing the defect makes the two agree,
        /// which fails this assertion by design.
        fn assert_state_address_lost(live: &Path, reachable: &Path, what: &str) {
            assert!(
                live.is_file(),
                "audit precondition failed for {what}: no bytes at {} to lose",
                live.display()
            );
            assert_ne!(
                reachable,
                live,
                "{what}: the system still resolves the address the bytes are at — \
                 the defect this test documents no longer reproduces"
            );
            assert!(
                !reachable.is_file(),
                "{what}: the address the system resolves holds bytes of its own at {}",
                reachable.display()
            );
        }

        /// A minimal but real Python app: a manifest the registry accepts and a
        /// `.py` entry, which is all `LivePythonPane::launch` needs to start a
        /// runtime. The guest never has to render for the addressing questions
        /// this module asks. Declares `context` scope explicitly — every test
        /// in this module is about context-root-relative addressing, which
        /// only `context` scope resolves against; the manifest-omitted
        /// default is `global` (home-dir-relative, root-independent) and
        /// would make every address assertion here vacuous.
        fn write_python_app(parent: &Path, app_id: &str) -> PathBuf {
            let app_dir = parent.join(app_id);
            std::fs::create_dir_all(&app_dir).expect("app dir");
            std::fs::write(
                app_dir.join("manifest.toml"),
                format!(
                    "schema_version = 1\n\n[app]\nid = \"{app_id}\"\ntype = \"app\"\n\
                     name = \"{app_id}\"\nversion = \"0.0.1\"\nentry = \"main.py\"\n\
                     \n[runtime]\npython_compat = true\n\
                     \n[state]\nscopes = [\"context\"]\n"
                ),
            )
            .expect("write manifest");
            std::fs::write(
                app_dir.join("main.py"),
                "def init(size, args): return []\ndef update(event): return []\n\
                 def view(): return None\n",
            )
            .expect("write entry");
            app_dir
        }

        /// The address the *running* instance writes to, read off its own
        /// `PythonLaunchConfig` rather than reconstructed from a pane field.
        fn live_state_address(harness: &crate::testing::HostHarness, pane_id: u64) -> PathBuf {
            let win = &harness.app.windows[harness.app.active_window];
            let pane = win
                .panes
                .get(&pane_id)
                .and_then(crate::host::pane::Pane::as_app)
                .expect("the launched pane is an app pane");
            match &pane.runtime {
                crate::host::pane::AppRuntime::Python(live) => python_state_path_for_config(&live.config),
                other => panic!("expected a CPython-WASM runtime, got {}", other.type_id()),
            }
        }

        fn seed_items(address: &Path, items: &serde_json::Value) {
            std::fs::create_dir_all(address.parent().expect("state parent")).expect("mkdir");
            crate::host::state_scope::atomic_write(
                address,
                &serde_json::to_vec_pretty(&serde_json::json!({ "items": items }))
                    .expect("serialize"),
            )
            .expect("the app persists an item");
        }

        /// Q5, first shape — the read that never happens. A saved workspace
        /// records an app pane under `AppRuntime::type_id()` (the runtime
        /// kind), so nothing in the record names the app whose state file the
        /// pane's bytes are in. Restoring cannot re-address that file no
        /// matter how it is implemented: the identity is simply not in the
        /// record. Asserted at that boundary rather than against the current
        /// restore helper, because a fix is expected to introduce a new
        /// restore path and leave the helper alone.
        #[test]
        fn audit_0678_a_saved_app_pane_cannot_re_address_its_own_state() {
            let mut harness = crate::testing::HostHarness::new();
            let root = tempdir().expect("workspace root");
            let app_dir = write_python_app(root.path(), "todo");

            let pane_id = harness
                .app
                .launch_app_by_path_with_layout_no_review_modal(
                    &app_dir.to_string_lossy(),
                    None,
                    Some(root.path().to_path_buf()),
                    &[],
                )
                .expect("launch the app")
                .expect("a Python launch returns a pane id");

            let live_address = live_state_address(&harness, pane_id);
            seed_items(&live_address, &serde_json::json!(["buy milk"]));

            harness.app.save_workspace();
            let saved = crate::workspace::WorkspaceFile::load().expect("saved workspace");
            let record = saved
                .windows
                .iter()
                .flat_map(|window| &window.panes)
                .find(|pane| pane.id == pane_id)
                .expect("the app pane is in the saved workspace");
            let recorded_id = record
                .app_id
                .clone()
                .expect("an app pane records some app id");

            // The record's cwd is right; its identity is not. Everything a
            // restorer could address from it lands somewhere else.
            let reachable = python_state_path_for_config(&state_test_config(&record.cwd, &recorded_id));
            assert_state_address_lost(
                &live_address,
                &reachable,
                "workspace save of a live CPython-WASM app pane",
            );

            // The other half of the pair: the identity that *would* re-address
            // the file is on the pane the whole time, and is not what was
            // written. A fix that records it flips this and the assertion above
            // together.
            let manifest_id = harness.app.windows[harness.app.active_window]
                .panes
                .get(&pane_id)
                .and_then(crate::host::pane::Pane::as_app)
                .expect("app pane")
                .manifest_id
                .clone();
            assert_ne!(
                recorded_id, manifest_id,
                "the saved record names the runtime kind, not the app — \
                 `manifest_id` is on the pane and carries the state file's name"
            );
        }

        /// Q3 — `set-root` after launch. The context's root moves; the running
        /// app's address does not, because it was captured into
        /// `PythonLaunchConfig` at launch and nothing revisits it. Asserted as
        /// a pair: the address is *unchanged*, and it *disagrees* with the
        /// root the context now has. Any addressing fix — call-time resolution
        /// against the live root, or a scope that drops the root entirely —
        /// breaks one half or the other.
        #[test]
        fn audit_0678_set_context_root_leaves_a_running_app_at_the_old_address() {
            let mut harness = crate::testing::HostHarness::new();
            let root_a = tempdir().expect("root a");
            let root_b = tempdir().expect("root b");
            let context_id = harness.app.router.get(0).context_id;
            harness
                .app
                .set_context_root(root_a.path().to_path_buf(), Some(context_id));

            // The launch root is the context root — what `resolve_new_pane_cwd`
            // yields for an app the registry has no workspace root for.
            let app_dir = write_python_app(root_a.path(), "todo");
            let pane_id = harness
                .app
                .launch_app_by_path_with_layout_no_review_modal(
                    &app_dir.to_string_lossy(),
                    None,
                    Some(root_a.path().to_path_buf()),
                    &[],
                )
                .expect("launch the app")
                .expect("a Python launch returns a pane id");
            let at_launch = live_state_address(&harness, pane_id);
            assert_eq!(
                at_launch,
                python_state_path_for_config(&state_test_config(root_a.path(), "todo")),
                "precondition: the running app is addressed under the context root"
            );
            seed_items(&at_launch, &serde_json::json!(["buy milk"]));

            harness
                .app
                .set_context_root(root_b.path().to_path_buf(), Some(context_id));

            assert_eq!(
                live_state_address(&harness, pane_id),
                at_launch,
                "the running app still writes its launch-time address"
            );
            assert_state_address_lost(
                &at_launch,
                &python_state_path_for_config(&state_test_config(root_b.path(), "todo")),
                "set_context_root while the app is running",
            );
        }

        // Q5, third shape — this documented a read that resolved to an implicit
        // second candidate (a channel-neutral global path) when the workspace
        // address had nothing yet, so a launch could be silently seeded from
        // bytes a later write would never touch. Stints 0651/0652 replaced
        // that implicit fallback with explicit `[state] scopes`: the state
        // module's design principle is "fail loud, no silent fallback" (see
        // `state_scope::parse_scopes`) — an app that wants both a global and a
        // context copy declares both scopes and the host never guesses which
        // one it meant. There is no implicit second candidate left to
        // reproduce this shape against.
    }

    #[test]
    fn python_state_load_failures_are_typed() {
        let workspace = tempdir().expect("workspace");
        let config = state_test_config(workspace.path(), "todo");
        let canonical = python_state_path_for_config(&config);
        std::fs::create_dir_all(canonical.parent().expect("canonical parent")).expect("mkdir");

        std::fs::create_dir(&canonical).expect("unreadable state path");
        assert!(matches!(
            load_python_state(&config),
            Err(WasmPythonError::ReadState { .. })
        ));
        std::fs::remove_dir(&canonical).expect("remove state dir");

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("missing-target", &canonical)
                .expect("dangling canonical symlink");
            assert!(matches!(
                load_python_state(&config),
                Err(WasmPythonError::ReadState { .. })
            ));
            std::fs::remove_file(&canonical).expect("remove dangling symlink");
        }

        std::fs::write(&canonical, b"{not-json").expect("invalid json");
        assert!(matches!(
            load_python_state(&config),
            Err(WasmPythonError::ParseState { .. })
        ));
        // `LivePythonPane::launch` no longer round-trips this error: stints
        // 0651/0652 replaced the single-scope `load_python_state` on the
        // launch path with `load_python_states`, which loads every declared
        // scope independently and treats a corrupt or unreadable scope file
        // the same as a missing one — log a warning, seed that scope empty —
        // rather than failing the whole launch over one bad file. See
        // `load_python_states`'s doc comment. The typed-error path above
        // (`load_python_state`) still exists and is still exercised for
        // direct callers; only the launch-time behavior changed.

        std::fs::write(&canonical, b"[1,2,3]").expect("non-object json");
        assert!(matches!(
            load_python_state(&config),
            Err(WasmPythonError::StateNotObject { .. })
        ));
    }

    #[test]
    fn python_state_atomic_write_replaces_complete_file_without_temp_leak() {
        let workspace = tempdir().expect("workspace");
        let path = workspace.path().join("todo.json");
        std::fs::write(&path, br#"{"version":"old"}"#).expect("seed");

        crate::host::state_scope::atomic_write(&path, br#"{"version":"new"}"#).expect("atomic replace");

        assert_eq!(
            std::fs::read(&path).expect("state"),
            br#"{"version":"new"}"#
        );
        let entries: Vec<_> = std::fs::read_dir(workspace.path())
            .expect("read dir")
            .map(|entry| entry.expect("entry").file_name())
            .collect();
        assert_eq!(entries, [std::ffi::OsString::from("todo.json")]);
    }

    // ── resolve_app_fs_path (stint 0508: workspace jail + picker grants) ──

    /// Relative paths keep the exact pre-grant jail behavior.
    #[test]
    fn resolve_app_fs_path_keeps_relative_workspace_jail() {
        let workspace = tempdir().expect("workspace");
        std::fs::write(workspace.path().join("note.txt"), "hi").expect("seed");

        let resolved = resolve_app_fs_path(workspace.path(), &[], "note.txt", false)
            .expect("workspace-relative read resolves");
        assert_eq!(
            resolved,
            workspace.path().canonicalize().unwrap().join("note.txt")
        );
        let escape = resolve_app_fs_path(workspace.path(), &[], "../outside.txt", false);
        assert!(escape.is_err(), "parent-dir escape must stay rejected");
    }

    /// Absolute paths are rejected outright when no grant covers them — even
    /// paths inside the workspace itself must go through the relative jail.
    #[test]
    fn resolve_app_fs_path_rejects_absolute_paths_without_grants() {
        let workspace = tempdir().expect("workspace");
        let outside = tempdir().expect("outside");
        let secret = outside.path().join("secret.txt");
        std::fs::write(&secret, "secret").expect("seed");

        let denied =
            resolve_app_fs_path(workspace.path(), &[], &secret.to_string_lossy(), false);
        assert!(denied.is_err(), "ungranted absolute path must be rejected");

        let workspace_file = workspace.path().join("inside.txt");
        std::fs::write(&workspace_file, "inside").expect("seed inside");
        let denied = resolve_app_fs_path(
            workspace.path(),
            &[],
            &workspace_file.to_string_lossy(),
            false,
        );
        assert!(
            denied.is_err(),
            "absolute form of a workspace file must still go through the relative jail"
        );
    }

    /// A file grant covers exactly that file; a sibling in the same directory
    /// stays rejected.
    #[test]
    fn resolve_app_fs_path_accepts_granted_file_only() {
        let workspace = tempdir().expect("workspace");
        let outside = tempdir().expect("outside");
        let picked = outside.path().join("picked.txt");
        let sibling = outside.path().join("sibling.txt");
        std::fs::write(&picked, "picked").expect("seed picked");
        std::fs::write(&sibling, "sibling").expect("seed sibling");
        let grants = vec![canonicalize_picked_path(&picked).expect("grant")];

        let allowed =
            resolve_app_fs_path(workspace.path(), &grants, &picked.to_string_lossy(), false)
                .expect("granted file readable");
        assert_eq!(allowed, picked.canonicalize().unwrap());
        let denied =
            resolve_app_fs_path(workspace.path(), &grants, &sibling.to_string_lossy(), false);
        assert!(denied.is_err(), "sibling of granted file must be rejected");
    }

    /// A folder grant covers the subtree, including new write targets, while
    /// paths outside the folder stay rejected.
    #[test]
    fn resolve_app_fs_path_folder_grant_covers_subtree() {
        let workspace = tempdir().expect("workspace");
        let outside = tempdir().expect("outside");
        let folder = outside.path().join("project");
        std::fs::create_dir(&folder).expect("mkdir");
        std::fs::write(folder.join("inner.txt"), "inner").expect("seed");
        let grants = vec![canonicalize_picked_path(&folder).expect("grant")];

        let read = resolve_app_fs_path(
            workspace.path(),
            &grants,
            &folder.join("inner.txt").to_string_lossy(),
            false,
        );
        assert!(read.is_ok(), "file under granted folder readable: {read:?}");
        let write = resolve_app_fs_path(
            workspace.path(),
            &grants,
            &folder.join("new.txt").to_string_lossy(),
            true,
        );
        assert!(write.is_ok(), "new file under granted folder writable: {write:?}");
        let denied = resolve_app_fs_path(
            workspace.path(),
            &grants,
            &outside.path().join("evil.txt").to_string_lossy(),
            true,
        );
        assert!(denied.is_err(), "outside the granted folder must be rejected");
    }

    /// A save-as grant targets a file that does not exist yet: the write
    /// resolves through the canonicalized parent, and reading it back after
    /// the write succeeds through the same grant.
    #[test]
    fn resolve_app_fs_path_save_grant_round_trips_new_file() {
        let workspace = tempdir().expect("workspace");
        let outside = tempdir().expect("outside");
        let target = outside.path().join("exported.txt");
        let grants = vec![canonicalize_picked_path(&target).expect("grant")];

        let write =
            resolve_app_fs_path(workspace.path(), &grants, &target.to_string_lossy(), true)
                .expect("save-as target writable before it exists");
        std::fs::write(&write, "exported").expect("write");
        let read =
            resolve_app_fs_path(workspace.path(), &grants, &target.to_string_lossy(), false)
                .expect("granted save-as target readable after write");
        assert_eq!(std::fs::read_to_string(read).unwrap(), "exported");
    }

    /// A symlink under a granted folder that points outside the grant is
    /// rejected: canonicalization resolves the target before the scope check.
    #[test]
    fn resolve_app_fs_path_symlink_cannot_escape_grant() {
        let workspace = tempdir().expect("workspace");
        let outside = tempdir().expect("outside");
        let folder = outside.path().join("granted");
        std::fs::create_dir(&folder).expect("mkdir");
        let secret = outside.path().join("secret.txt");
        std::fs::write(&secret, "secret").expect("seed");
        std::os::unix::fs::symlink(&secret, folder.join("link.txt")).expect("symlink");
        let grants = vec![canonicalize_picked_path(&folder).expect("grant")];

        let denied = resolve_app_fs_path(
            workspace.path(),
            &grants,
            &folder.join("link.txt").to_string_lossy(),
            false,
        );
        assert!(
            denied.is_err(),
            "symlink escaping the granted folder must be rejected: {denied:?}"
        );
    }

    /// Feed stderr through one classifier the way the pane does — one call
    /// per `drain_stderr` — then flush the trailing partial line.
    fn classify_stderr(chunks: &[&str]) -> Vec<GuestStderrRecord> {
        let mut classifier = GuestStderrClassifier::new();
        let mut records: Vec<_> = chunks
            .iter()
            .flat_map(|chunk| classifier.push(chunk))
            .collect();
        records.extend(classifier.flush());
        records
    }

    fn exception_lines(records: &[GuestStderrRecord]) -> Vec<&str> {
        records
            .iter()
            .filter(|record| record.kind == GuestStderrKind::TracebackException)
            .map(|record| record.line.as_str())
            .collect()
    }

    /// Stint 0417: the benign CPython WASI startup line classifies as noise so
    /// it doesn't drown real guest tracebacks — but only that line, and never
    /// at the cost of the traceback it is concatenated in front of.
    #[test]
    fn guest_stderr_isolates_benign_wasi_startup_noise_from_the_payload() {
        let records = classify_stderr(&[
            "  Could not find platform dependent libraries <exec_prefix>  \n\
             Could not find platform dependent libraries <exec_prefix>\n\
             \n\
             Traceback (most recent call last):\n\
             \x20 File \"logs.py\", line 10, in <module>\n\
             NameError: x\n",
        ]);
        assert_eq!(
            records
                .iter()
                .filter(|record| record.kind == GuestStderrKind::BenignWasiStartup)
                .count(),
            2
        );
        assert_eq!(exception_lines(&records), vec!["NameError: x"]);
        // Every record is one physical line: a continuation with no host
        // prefix is exactly what stint 0643 exists to prevent.
        assert!(records.iter().all(|record| !record.line.contains('\n')));
    }

    /// Stint 0643 regression. The marker must land on the exception, not on
    /// whichever frame line happened to end a drain. Real repro shape: a
    /// `RuntimeError` under the `runpy` bootstrap frames, arriving in the two
    /// chunks the pane actually saw — the header and first frame in one
    /// drain, the rest in the next.
    #[test]
    fn guest_stderr_marks_the_exception_not_the_first_frame_across_drains() {
        let records = classify_stderr(&[
            "Traceback (most recent call last):\n\
             \x20 File \"<frozen runpy>\", line 198, in _run_module_as_main\n",
            "  File \"<frozen runpy>\", line 88, in _run_code\n\
             \x20 File \"/app/main.py\", line 7, in <module>\n\
             \x20   raise RuntimeError(\"PR2530_TRACEBACK_SENTINEL\")\n\
             RuntimeError: PR2530_TRACEBACK_SENTINEL\n",
        ]);
        assert_eq!(
            exception_lines(&records),
            vec!["RuntimeError: PR2530_TRACEBACK_SENTINEL"]
        );
        assert!(records
            .iter()
            .filter(|record| record.line.starts_with("File "))
            .all(|record| record.kind == GuestStderrKind::TracebackFrame));
    }

    /// A chained traceback has two exceptions and must mark both — and must
    /// not mark the sentence between them.
    #[test]
    fn guest_stderr_marks_both_halves_of_a_chained_traceback() {
        let records = classify_stderr(&["Traceback (most recent call last):\n\
             \x20 File \"/app/main.py\", line 3, in <module>\n\
             \x20   1 / 0\n\
             ZeroDivisionError: division by zero\n\
             \n\
             During handling of the above exception, another exception occurred:\n\
             \n\
             Traceback (most recent call last):\n\
             \x20 File \"/app/main.py\", line 5, in <module>\n\
             \x20   raise RuntimeError(\"PR2530_TRACEBACK_SENTINEL\")\n\
             RuntimeError: PR2530_TRACEBACK_SENTINEL\n"]);
        assert_eq!(
            exception_lines(&records),
            vec![
                "ZeroDivisionError: division by zero",
                "RuntimeError: PR2530_TRACEBACK_SENTINEL",
            ]
        );
        assert_eq!(
            records
                .iter()
                .filter(|record| record.kind == GuestStderrKind::TracebackChainSeparator)
                .map(|record| record.line.as_str())
                .collect::<Vec<_>>(),
            vec!["During handling of the above exception, another exception occurred:"]
        );
    }

    /// `raise ... from ...` uses the other separator sentence.
    #[test]
    fn guest_stderr_marks_both_halves_of_a_direct_cause_chain() {
        let records = classify_stderr(&["Traceback (most recent call last):\n\
             \x20 File \"/app/main.py\", line 3, in <module>\n\
             KeyError: 'token'\n\
             \n\
             The above exception was the direct cause of the following exception:\n\
             \n\
             Traceback (most recent call last):\n\
             \x20 File \"/app/main.py\", line 5, in <module>\n\
             RuntimeError: config load failed\n"]);
        assert_eq!(
            exception_lines(&records),
            vec!["KeyError: 'token'", "RuntimeError: config load failed"]
        );
    }

    /// A multi-line exception message is one exception: the first line is the
    /// exception, the rest are its detail, and no detail line steals the mark.
    #[test]
    fn guest_stderr_keeps_a_multi_line_exception_message_as_one_exception() {
        let records = classify_stderr(&["Traceback (most recent call last):\n\
             \x20 File \"/app/main.py\", line 9, in <module>\n\
             ValueError: manifest is invalid:\n\
             missing key: name\n\
             missing key: version\n"]);
        assert_eq!(
            exception_lines(&records),
            vec!["ValueError: manifest is invalid:"]
        );
        assert_eq!(
            records
                .iter()
                .filter(|record| record.kind == GuestStderrKind::TracebackExceptionDetail)
                .map(|record| record.line.as_str())
                .collect::<Vec<_>>(),
            vec!["missing key: name", "missing key: version"]
        );
    }

    /// A compile-time `SyntaxError` prints no `Traceback` header at all, and
    /// its caret line is part of the frame, not the exception.
    #[test]
    fn guest_stderr_marks_a_headerless_syntax_error_block() {
        let records = classify_stderr(&["  File \"/app/main.py\", line 2\n\
             \x20   def broken(:\n\
             \x20              ^\n\
             SyntaxError: invalid syntax\n"]);
        assert_eq!(
            exception_lines(&records),
            vec!["SyntaxError: invalid syntax"]
        );
        assert_eq!(
            records
                .iter()
                .filter(|record| record.kind == GuestStderrKind::TracebackFrame)
                .count(),
            3
        );
    }

    /// A bare `KeyboardInterrupt` has no `: message` and is still the
    /// exception — the frame block ended at it.
    #[test]
    fn guest_stderr_marks_a_bare_keyboard_interrupt() {
        let records = classify_stderr(&["Traceback (most recent call last):\n\
             \x20 File \"/app/main.py\", line 12, in <module>\n\
             \x20   sleep(60)\n\
             KeyboardInterrupt\n"]);
        assert_eq!(exception_lines(&records), vec!["KeyboardInterrupt"]);
    }

    /// `drain_stderr` splits on byte boundaries, not line boundaries: a line
    /// cut in half must be rejoined before it is classified, never logged as
    /// two fragments.
    #[test]
    fn guest_stderr_rejoins_a_line_split_mid_drain() {
        let records = classify_stderr(&[
            "Traceback (most recent call last):\n  File \"/app/main.py\", line 7, in <mod",
            "ule>\nRuntimeError: PR2530_TRAC",
            "EBACK_SENTINEL",
        ]);
        assert_eq!(
            exception_lines(&records),
            vec!["RuntimeError: PR2530_TRACEBACK_SENTINEL"]
        );
        assert_eq!(records.len(), 3);
    }

    /// A guest that never writes a newline must still reach the log.
    #[test]
    fn guest_stderr_flushes_an_unterminated_line_past_the_buffer_cap() {
        let mut classifier = GuestStderrClassifier::new();
        assert!(classifier
            .push(&"x".repeat(MAX_BUFFERED_STDERR_LINE))
            .is_empty());
        let records = classifier.push("y");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].kind, GuestStderrKind::Payload);
        assert_eq!(records[0].line.len(), MAX_BUFFERED_STDERR_LINE + 1);
    }

    /// Ordinary guest stderr with no traceback in it stays ordinary.
    #[test]
    fn guest_stderr_leaves_plain_output_unmarked() {
        let records = classify_stderr(&["warning one\nwarning two\n"]);
        assert_eq!(records.len(), 2);
        assert!(records
            .iter()
            .all(|record| record.kind == GuestStderrKind::Payload));
    }

    #[test]
    fn traceback_exception_line_uses_the_final_exception() {
        assert_eq!(
            traceback_exception_line(
                "Traceback (most recent call last):\n  File \"main.py\", line 1, in <module>\nImportError: fixture import failure\n"
            ),
            Some("ImportError: fixture import failure")
        );
    }

    #[test]
    fn traceback_exception_line_waits_for_an_unindented_exception() {
        assert_eq!(
            traceback_exception_line(
                "Traceback (most recent call last):\n  File \"main.py\", line 1, in <module>\n"
            ),
            None
        );
    }

    #[test]
    fn traceback_exception_line_prefers_the_last_chained_exception() {
        assert_eq!(
            traceback_exception_line(
                "Traceback (most recent call last):\n  File \"main.py\", line 1, in <module>\nValueError: original\n\nDuring handling of the above exception, another exception occurred:\n\nTraceback (most recent call last):\n  File \"main.py\", line 3, in <module>\nTypeError: chained failure\n"
            ),
            Some("TypeError: chained failure")
        );
    }

    #[test]
    fn traceback_exception_line_accepts_a_bare_exception_name() {
        assert_eq!(
            traceback_exception_line(
                "Traceback (most recent call last):\n  File \"main.py\", line 1, in <module>\nKeyboardInterrupt\n"
            ),
            Some("KeyboardInterrupt")
        );
    }

    #[test]
    fn pending_traceback_exception_ignores_a_partial_line_while_the_guest_lives() {
        let buffer = "Traceback (most recent call last):\n  File \"main.py\", line 1, in <module>\nImportErr";
        assert_eq!(pending_traceback_exception(buffer, false), None);
        assert_eq!(
            pending_traceback_exception(buffer, true),
            Some("ImportErr")
        );
        let completed = format!("{buffer}or: fixture import failure\n");
        assert_eq!(
            pending_traceback_exception(&completed, false),
            Some("ImportError: fixture import failure")
        );
    }

    #[test]
    fn python_lifecycle_reads_liveness_from_the_runtime_not_the_tree() {
        // The bug: a guest that dies after its first frame must not keep
        // reporting `running` off its stale tree.
        assert_eq!(python_lifecycle(None, true, true), ("exited", None));
        assert_eq!(python_lifecycle(None, true, false), ("exited", None));
        assert_eq!(python_lifecycle(None, false, true), ("running", None));
        assert_eq!(python_lifecycle(None, false, false), ("starting", None));
        assert_eq!(
            python_lifecycle(Some("ImportError: nope"), true, true),
            ("failed", Some("ImportError: nope"))
        );
    }

    #[test]
    fn python_keys_use_sdk_lowercase_names() {
        assert_eq!(python_key_name(egui::Key::ArrowDown), "down");
        assert_eq!(python_key_name(egui::Key::Escape), "escape");
        assert_eq!(python_key_name(egui::Key::Enter), "enter");
    }

    #[test]
    fn python_key_name_maps_digit_keys_to_plain_digits() {
        assert_eq!(python_key_name(egui::Key::Num0), "0");
        assert_eq!(python_key_name(egui::Key::Num1), "1");
        assert_eq!(python_key_name(egui::Key::Num2), "2");
        assert_eq!(python_key_name(egui::Key::Num3), "3");
        assert_eq!(python_key_name(egui::Key::Num4), "4");
        assert_eq!(python_key_name(egui::Key::Num5), "5");
        assert_eq!(python_key_name(egui::Key::Num6), "6");
        assert_eq!(python_key_name(egui::Key::Num7), "7");
        assert_eq!(python_key_name(egui::Key::Num8), "8");
        assert_eq!(python_key_name(egui::Key::Num9), "9");
    }

    #[test]
    fn python_key_name_fallback_matches_documented_punctuation_names() {
        assert_eq!(python_key_name(egui::Key::Plus), "plus");
    }

    /// Stint 0462: punctuation keys must map to their literal characters, matching
    /// `canonical_key_name()` in wasm_pane.rs, so PGAP apps comparing `key == "/"`
    /// (e.g. logs.py's search shortcut) actually match.
    #[test]
    fn python_key_name_maps_punctuation_keys_to_literal_characters() {
        assert_eq!(python_key_name(egui::Key::Slash), "/");
        assert_eq!(python_key_name(egui::Key::Minus), "-");
        assert_eq!(python_key_name(egui::Key::Equals), "=");
        assert_eq!(python_key_name(egui::Key::Backslash), "\\");
        assert_eq!(python_key_name(egui::Key::Semicolon), ";");
        assert_eq!(python_key_name(egui::Key::Quote), "'");
        assert_eq!(python_key_name(egui::Key::Backtick), "`");
        assert_eq!(python_key_name(egui::Key::Comma), ",");
        assert_eq!(python_key_name(egui::Key::Period), ".");
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

    /// Bare Escape drives CloseApp, and every command chord belongs to the host.
    /// Python and component-WASM apps must enforce the same boundary.
    #[test]
    fn python_key_events_reserves_all_command_chords_for_the_host() {
        let bare_escape = [egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }];
        assert!(
            python_key_events(&bare_escape).is_empty(),
            "bare Escape must not reach the guest — it drives host CloseApp"
        );

        let cmd_escape = [egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        }];
        assert!(
            python_key_events(&cmd_escape).is_empty(),
            "command chords belong exclusively to the host"
        );

        let cmd_d = [egui::Event::Key {
            key: egui::Key::D,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        }];
        assert!(
            python_key_events(&cmd_d).is_empty(),
            "Cmd+D must never arrive at a Python app as bare delete input"
        );
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

    // Stint 0445: good-by-default inter-child spacing. When an app declares no
    // `gap`, the decoder fills in the design-token default; an explicit value
    // (including `0.0`, meaning "pack flush") always wins over the default.
    #[test]
    fn container_gap_defaults_when_unset_and_explicit_value_wins() {
        let col_default =
            decode_node_data(&json!({"type": "column", "children": []})).expect("decode column");
        let UiNodeData::Column(c) = col_default else {
            panic!("expected column");
        };
        assert_eq!(
            c.gap,
            crate::ui::style::SPACE_MD,
            "an unset column gap must fall back to the SPACE_MD token"
        );

        let row_default =
            decode_node_data(&json!({"type": "row", "children": []})).expect("decode row");
        let UiNodeData::Row(r) = row_default else {
            panic!("expected row");
        };
        assert_eq!(
            r.gap,
            crate::ui::style::SPACE_SM,
            "an unset row gap must fall back to the SPACE_SM token"
        );

        let col_zero = decode_node_data(&json!({"type": "column", "children": [], "gap": 0.0}))
            .expect("decode column");
        let UiNodeData::Column(c) = col_zero else {
            panic!("expected column");
        };
        assert_eq!(c.gap, 0.0, "an explicit gap=0 must pack children flush");

        let row_explicit = decode_node_data(&json!({"type": "row", "children": [], "gap": 20.0}))
            .expect("decode row");
        let UiNodeData::Row(r) = row_explicit else {
            panic!("expected row");
        };
        assert_eq!(r.gap, 20.0, "an explicit row gap must be honored verbatim");
    }

    #[test]
    fn manifest_http_hosts_allow_exact_and_subdomains_only() {
        use crate::host::services::http_host_allowed;
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
        let mut pending = HashMap::from([(frame_id, Arc::new(pending_tree))]);
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
    }

    #[test]
    fn continuous_scheduler_coalesces_guest_wake_into_next_host_deadline() {
        let now = std::time::Instant::now();
        let mut scheduler = PythonFrameScheduler::new(now);
        scheduler.set_mode(Some("continuous"), Some(60), now);
        scheduler.poll_render(now).expect("first frame");

        assert!(scheduler.next_repaint_deadline(now).is_some());
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
        }
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
    fn drainable_output_blocks_until_guest_writes_then_wakes_decoder() {
        let output = DrainableOutput::default();
        let mut writer = output.clone();
        let reader = output.clone();
        let handle = std::thread::spawn(move || reader.wait_and_drain());
        // Give the reader time to park on the condvar, then write.
        std::thread::sleep(std::time::Duration::from_millis(20));
        let waker = Waker::from(Arc::new(NoopWake));
        let mut context = Context::from_waker(&waker);
        assert!(Pin::new(&mut writer)
            .poll_write(&mut context, b"frame_done\n")
            .is_ready());
        assert_eq!(
            handle.join().expect("reader thread"),
            Some(b"frame_done\n".to_vec())
        );
    }

    #[test]
    fn drainable_output_close_unblocks_decoder_with_none() {
        let output = DrainableOutput::default();
        let reader = output.clone();
        let handle = std::thread::spawn(move || reader.wait_and_drain());
        std::thread::sleep(std::time::Duration::from_millis(20));
        output.close();
        assert_eq!(handle.join().expect("reader thread"), None);
    }

    #[test]
    fn state_persist_resolves_against_live_context_root_and_rejects_undeclared_scope() {
        let app = tempdir().expect("app dir");
        std::fs::write(
            app.path().join("main.py"),
            "def init(size, args): return []\ndef update(event): return []\ndef view():\n    from plexi_sdk.ui import Text\n    return Text('x')\n",
        )
        .expect("write app");
        let root_a = tempdir().expect("root a");
        let root_b = tempdir().expect("root b");
        let config = PythonLaunchConfig {
            app_id: "test.state-scope".to_string(),
            app_dir: app.path().to_path_buf(),
            entry: app.path().join("main.py"),
            module_name: "main".to_string(),
            launch_args: Vec::new(),
            workspace_root: app.path().to_path_buf(),
            capabilities: Vec::new(),
            allowed_hosts: Vec::new(),
            theme: crate::ui::theme::colors_from_config(
                &crate::config::PlexiConfig::default(),
            )
            .to_theme_map(),
            state_scopes: vec![
                crate::host::state_scope::StateScope::Global,
                crate::host::state_scope::StateScope::Context,
            ],
            state_format: crate::host::state_scope::StateFormat::Json,
            context_root: root_a.path().to_path_buf(),
        };
        let mut pane = LivePythonPane::launch(config).expect("launch pane");

        // Context-scoped persist lands inside the launching context root and
        // ensures the app_states gitignore so the state cannot be committed.
        pane.save_state(Some(&json!("context")), Some(&json!({"k": 1})));
        let file_a = root_a
            .path()
            .join(".plexi/app_states/test.state-scope.json");
        let persisted: Value =
            serde_json::from_slice(&std::fs::read(&file_a).expect("state file under root A"))
                .expect("valid JSON");
        assert_eq!(persisted, json!({"k": 1}));
        let ignore_a =
            std::fs::read_to_string(root_a.path().join(".plexi/.gitignore")).expect("gitignore");
        assert!(
            ignore_a.lines().any(|line| line.trim() == "app_states/"),
            "context-scope persist must ensure the app_states ignore: {ignore_a:?}"
        );

        // Resolution follows a context root change at runtime — the next
        // persist lands under the NEW root, not the launch-captured one.
        pane.set_context_root(root_b.path());
        pane.save_state(Some(&json!("context")), Some(&json!({"k": 2})));
        let file_b = root_b
            .path()
            .join(".plexi/app_states/test.state-scope.json");
        let persisted_b: Value =
            serde_json::from_slice(&std::fs::read(&file_b).expect("state file under root B"))
                .expect("valid JSON");
        assert_eq!(persisted_b, json!({"k": 2}));
        let stale_a: Value =
            serde_json::from_slice(&std::fs::read(&file_a).expect("root A file untouched"))
                .expect("valid JSON");
        assert_eq!(stale_a, json!({"k": 1}), "old root's file must not be rewritten");

        // A scope the app did not declare is an error at persist time, never
        // a silent fallback to another scope's file.
        let undeclared_root = tempdir().expect("undeclared root");
        let global_only = PythonLaunchConfig {
            app_id: "test.global-only".to_string(),
            app_dir: app.path().to_path_buf(),
            entry: app.path().join("main.py"),
            module_name: "main".to_string(),
            launch_args: Vec::new(),
            workspace_root: app.path().to_path_buf(),
            capabilities: Vec::new(),
            allowed_hosts: Vec::new(),
            theme: crate::ui::theme::colors_from_config(
                &crate::config::PlexiConfig::default(),
            )
            .to_theme_map(),
            state_scopes: crate::host::state_scope::default_scopes(),
            state_format: crate::host::state_scope::StateFormat::Json,
            context_root: undeclared_root.path().to_path_buf(),
        };
        let mut global_pane = LivePythonPane::launch(global_only).expect("launch global-only");
        global_pane.save_state(Some(&json!("context")), Some(&json!({"k": 3})));
        assert!(
            !undeclared_root
                .path()
                .join(".plexi/app_states/test.global-only.json")
                .exists(),
            "an undeclared scope must not persist anything"
        );
    }

    /// Shared scaffolding for the file-backed state tests (stint 0644):
    /// a trivial app + a context-scoped launch config against a fresh root.
    fn file_state_config(
        app_dir: &Path,
        app_id: &str,
        context_root: &Path,
        format: crate::host::state_scope::StateFormat,
    ) -> PythonLaunchConfig {
        std::fs::write(
            app_dir.join("main.py"),
            "def init(size, args): return []\ndef update(event): return []\ndef view():\n    from plexi_sdk.ui import Text\n    return Text('x')\n",
        )
        .expect("write app");
        PythonLaunchConfig {
            app_id: app_id.to_string(),
            app_dir: app_dir.to_path_buf(),
            entry: app_dir.join("main.py"),
            module_name: "main".to_string(),
            launch_args: Vec::new(),
            workspace_root: app_dir.to_path_buf(),
            capabilities: Vec::new(),
            allowed_hosts: Vec::new(),
            state_scopes: vec![crate::host::state_scope::StateScope::Context],
            state_format: format,
            theme: crate::ui::theme::colors_from_config(&crate::config::PlexiConfig::default())
                .to_theme_map(),
            context_root: context_root.to_path_buf(),
        }
    }

    fn context_state_file(root: &Path, app_id: &str, ext: &str) -> PathBuf {
        root.join(".plexi/app_states")
            .join(format!("{app_id}.{ext}"))
    }

    #[test]
    fn persist_does_not_clobber_a_file_changed_since_load() {
        let app = tempdir().expect("app dir");
        let root = tempdir().expect("context root");
        let file = context_state_file(root.path(), "test.no-clobber", "json");
        std::fs::create_dir_all(file.parent().unwrap()).expect("state dir");
        std::fs::write(&file, serde_json::to_vec_pretty(&json!({"a": 1})).unwrap())
            .expect("seed state");

        let config = file_state_config(
            app.path(),
            "test.no-clobber",
            root.path(),
            crate::host::state_scope::StateFormat::Json,
        );
        let mut pane = LivePythonPane::launch(config).expect("launch pane");

        // External writer lands after load. Different byte length guarantees
        // the (mtime, len) pair differs even at 1s mtime granularity.
        std::fs::write(
            &file,
            serde_json::to_vec_pretty(&json!({"external": "winner", "padding": 12345})).unwrap(),
        )
        .expect("external write");

        pane.save_state(
            Some(&json!("context")),
            Some(&json!({"app": "would-clobber"})),
        );

        let on_disk: Value =
            serde_json::from_slice(&std::fs::read(&file).expect("read state")).expect("json");
        assert_eq!(
            on_disk,
            json!({"external": "winner", "padding": 12345}),
            "disk wins: the app's persist must be dropped, not overwrite the external write"
        );
        let scope_state = pane
            .persisted_states
            .get(&crate::host::state_scope::StateScope::Context)
            .expect("scope state");
        assert_eq!(
            Value::Object(scope_state.values.clone()),
            json!({"external": "winner", "padding": 12345}),
            "the on-disk values must replace (not merge into) the in-memory scope"
        );

        // A follow-up persist now syncs cleanly (identity was re-cached).
        pane.save_state(Some(&json!("context")), Some(&json!({"app": "retry"})));
        let retried: Value =
            serde_json::from_slice(&std::fs::read(&file).expect("read state")).expect("json");
        assert_eq!(retried, json!({"app": "retry"}));
    }

    #[test]
    fn persist_is_atomic_and_leaves_no_partial_file() {
        let app = tempdir().expect("app dir");
        let root = tempdir().expect("context root");
        let config = file_state_config(
            app.path(),
            "test.atomic",
            root.path(),
            crate::host::state_scope::StateFormat::Json,
        );
        let mut pane = LivePythonPane::launch(config).expect("launch pane");
        pane.save_state(Some(&json!("context")), Some(&json!({"k": 1})));

        let file = context_state_file(root.path(), "test.atomic", "json");
        let on_disk: Value =
            serde_json::from_slice(&std::fs::read(&file).expect("read state")).expect("json");
        assert_eq!(on_disk, json!({"k": 1}));
        let residue: Vec<String> = std::fs::read_dir(file.parent().unwrap())
            .expect("read state dir")
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains(".tmp-"))
            .collect();
        assert!(residue.is_empty(), "temp residue left behind: {residue:?}");
    }

    #[test]
    fn malformed_state_file_surfaces_error_and_does_not_reset() {
        let app = tempdir().expect("app dir");
        let root = tempdir().expect("context root");
        let file = context_state_file(root.path(), "test.malformed", "json");
        std::fs::create_dir_all(file.parent().unwrap()).expect("state dir");
        std::fs::write(&file, b"{not json at all").expect("seed corrupt state");

        let config = file_state_config(
            app.path(),
            "test.malformed",
            root.path(),
            crate::host::state_scope::StateFormat::Json,
        );
        let mut pane = LivePythonPane::launch(config).expect("launch pane");
        let scope = crate::host::state_scope::StateScope::Context;
        assert!(
            pane.persisted_states
                .get(&scope)
                .and_then(|s| s.error.as_ref())
                .is_some(),
            "a corrupt state file must surface as a scope error"
        );

        // Persists are refused while the error stands — the corrupt file is
        // never silently replaced with `{}` or the app's payload.
        pane.save_state(Some(&json!("context")), Some(&json!({"k": 1})));
        assert_eq!(
            std::fs::read(&file).expect("read state"),
            b"{not json at all",
            "a corrupt state file must not be overwritten"
        );

        // Fixing the file externally and re-reading clears the error.
        std::fs::write(
            &file,
            serde_json::to_vec_pretty(&json!({"fixed": true})).unwrap(),
        )
        .expect("fix state file");
        pane.apply_external_state(scope);
        let scope_state = pane.persisted_states.get(&scope).expect("scope state");
        assert!(
            scope_state.error.is_none(),
            "successful re-read clears the error"
        );
        assert_eq!(
            Value::Object(scope_state.values.clone()),
            json!({"fixed": true})
        );
        pane.save_state(Some(&json!("context")), Some(&json!({"k": 2})));
        let on_disk: Value =
            serde_json::from_slice(&std::fs::read(&file).expect("read state")).expect("json");
        assert_eq!(
            on_disk,
            json!({"k": 2}),
            "persists resume once the error clears"
        );
    }

    #[test]
    fn apply_external_state_replaces_rather_than_merges() {
        let app = tempdir().expect("app dir");
        let root = tempdir().expect("context root");
        let file = context_state_file(root.path(), "test.replace", "json");
        std::fs::create_dir_all(file.parent().unwrap()).expect("state dir");
        std::fs::write(
            &file,
            serde_json::to_vec_pretty(&json!({"a": 1, "b": 2})).unwrap(),
        )
        .expect("seed state");

        let config = file_state_config(
            app.path(),
            "test.replace",
            root.path(),
            crate::host::state_scope::StateFormat::Json,
        );
        let mut pane = LivePythonPane::launch(config).expect("launch pane");
        let scope = crate::host::state_scope::StateScope::Context;

        // External write drops key "b"; longer content keeps (mtime, len)
        // distinct without relying on mtime granularity.
        std::fs::write(
            &file,
            serde_json::to_vec_pretty(&json!({"a": 99, "padding": "xxxxxxxxxxxx"})).unwrap(),
        )
        .expect("external write");
        pane.apply_external_state(scope);

        let scope_state = pane.persisted_states.get(&scope).expect("scope state");
        assert_eq!(
            Value::Object(scope_state.values.clone()),
            json!({"a": 99, "padding": "xxxxxxxxxxxx"}),
            "external state must replace wholesale — deleted keys must vanish, never merge"
        );
    }

    #[test]
    fn markdown_format_writes_document_verbatim() {
        let app = tempdir().expect("app dir");
        let root = tempdir().expect("context root");
        let config = file_state_config(
            app.path(),
            "test.markdown",
            root.path(),
            crate::host::state_scope::StateFormat::Markdown,
        );
        let mut pane = LivePythonPane::launch(config).expect("launch pane");

        let document = "# Checklist\n\n- [ ] first\n- [x] second\n";
        pane.save_state(
            Some(&json!("context")),
            Some(&json!({"document": document})),
        );

        let file = context_state_file(root.path(), "test.markdown", "md");
        assert!(file.exists(), "markdown state must use the .md extension");
        assert_eq!(
            std::fs::read(&file).expect("read markdown state"),
            document.as_bytes(),
            "markdown bytes must round-trip verbatim — no JSON envelope, no escaping"
        );

        // Read-back is the inverse: {"document": "<file text>"}.
        let external = "# Edited outside\n";
        std::fs::write(&file, external).expect("external markdown write");
        pane.apply_external_state(crate::host::state_scope::StateScope::Context);
        let scope_state = pane
            .persisted_states
            .get(&crate::host::state_scope::StateScope::Context)
            .expect("scope state");
        assert_eq!(
            Value::Object(scope_state.values.clone()),
            json!({"document": external})
        );
    }

    #[test]
    fn markdown_format_rejects_non_string_document() {
        let app = tempdir().expect("app dir");
        let root = tempdir().expect("context root");
        let config = file_state_config(
            app.path(),
            "test.markdown-bad",
            root.path(),
            crate::host::state_scope::StateFormat::Markdown,
        );
        let mut pane = LivePythonPane::launch(config).expect("launch pane");

        pane.save_state(Some(&json!("context")), Some(&json!({"document": 42})));
        pane.save_state(Some(&json!("context")), Some(&json!({"not_document": "x"})));

        let file = context_state_file(root.path(), "test.markdown-bad", "md");
        assert!(
            !file.exists(),
            "a non-string (or missing) document must be a loud error with NO write"
        );
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
            theme: crate::ui::theme::colors_from_config(
                &crate::config::PlexiConfig::default(),
            )
            .to_theme_map(),
            state_scopes: crate::host::state_scope::default_scopes(),
            state_format: crate::host::state_scope::StateFormat::Json,
            context_root: app.path().to_path_buf(),
        };
        let mut runtime = WasmPythonRuntime::launch(&config).expect("launch CPython WASM");
        runtime
            .send(&python_init_payload(&config, json!({}), (480.0, 320.0)))
            .expect("send init");
        runtime
            .send(&json!({"type": "render", "frame_id": 1}))
            .expect("send render");

        let deadline = std::time::Instant::now()
            + crate::testing::load_aware_timeout(std::time::Duration::from_secs(30));
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

    /// Run one visible egui pass over the pane, the way `tiling.rs` does for a
    /// pane the user can see. Used only to get the app past `init` — after
    /// this the test never renders again, standing in for a pane that scrolled
    /// off screen, moved to an inactive context, or sits under an occluded
    /// window.
    fn visible_pass(runtime: &mut crate::host::pane::AppRuntime, colors: &crate::ui::theme::Colors) {
        let ctx = egui::Context::default();
        let _ = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 600.0),
                )),
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show_inside(ui, |ui| {
                    runtime.ui(
                        ui,
                        &crate::app::app_trait::AppRenderContext {
                            colors,
                            pane_id: 1,
                        },
                        None,
                    );
                });
            },
        );
    }

    /// Stint 0684: an assistant tool call to an app pane that is *not*
    /// rendering must still be answered. `tool_dispatch` writes the `ToolCall`
    /// straight into the guest's stdin and blocks on the reply, so the guest
    /// runs and answers regardless of what the host is painting — but the host
    /// only ever read that reply out of the decoder channel inside
    /// `LivePythonPane::ui`. A pane in an inactive context, behind an occluded
    /// window, or otherwise off screen therefore never surfaced its
    /// `ToolResult` and every call to it timed out.
    ///
    /// The contract this pins: after one visible pass to get the app running,
    /// `background_tick` alone must carry a tool call to its result.
    #[test]
    fn python_tool_result_reaches_the_host_without_a_ui_pass() {
        let app = tempdir().expect("app dir");
        std::fs::write(
            app.path().join("main.py"),
            "from plexi_sdk.effects import AiTool, ExposeTools, ToolResult\n\
             from plexi_sdk.events import ToolCall\n\
             from plexi_sdk.ui import Column, Text\n\
             \n\
             def init(size, args):\n\
             \x20   return [ExposeTools([AiTool(name='probe.ping', description='ping',\n\
             \x20       input_schema={'type': 'object', 'properties': {}},\n\
             \x20       output_schema={'type': 'object'}, read_only=True)])]\n\
             \n\
             def update(event):\n\
             \x20   if isinstance(event, ToolCall):\n\
             \x20       return [ToolResult(event.call_id, output_json='{\"pong\": true}')]\n\
             \x20   return []\n\
             \n\
             def view():\n\
             \x20   return Column([Text('probe')])\n",
        )
        .expect("write app");
        let config = PythonLaunchConfig {
            app_id: "test.tool-offscreen".to_string(),
            app_dir: app.path().to_path_buf(),
            entry: app.path().join("main.py"),
            module_name: "main".to_string(),
            launch_args: Vec::new(),
            workspace_root: app.path().to_path_buf(),
            capabilities: Vec::new(),
            allowed_hosts: Vec::new(),
            theme: crate::ui::theme::colors_from_config(
                &crate::config::PlexiConfig::default(),
            )
            .to_theme_map(),
            state_scopes: crate::host::state_scope::default_scopes(),
            state_format: crate::host::state_scope::StateFormat::Json,
            context_root: app.path().to_path_buf(),
        };
        let colors = crate::ui::theme::colors_from_config(&crate::config::PlexiConfig::default());
        let mut runtime = crate::host::pane::AppRuntime::Python(Box::new(
            LivePythonPane::launch(config).expect("launch CPython WASM"),
        ));

        // The pane was on screen once: render until the app has declared its
        // tools, which is how `tool_dispatch` learns to reach it at all.
        let deadline = std::time::Instant::now()
            + crate::testing::load_aware_timeout(std::time::Duration::from_secs(60));
        let mut exposed = false;
        while !exposed && std::time::Instant::now() < deadline {
            visible_pass(&mut runtime, &colors);
            exposed = runtime
                .take_pending_commands()
                .iter()
                .any(|cmd| matches!(cmd, crate::app::app_trait::AppCommand::ExposeTools { .. }));
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(exposed, "app never exposed its tools on a visible pass");

        // Now the pane goes off screen. `tool_dispatch` still reaches the
        // guest — it writes to stdin, which is independent of rendering.
        let crate::host::pane::AppRuntime::Python(pane) = &runtime else {
            panic!("python runtime");
        };
        pane.tool_event_sender()
            .push_json_line(&json!({
                "type": "tool_call",
                "call_id": "call-offscreen",
                "name": "probe.ping",
                "input_json": "{}",
                "caller_id": "test",
            }))
            .expect("deliver tool call");

        // Only background servicing runs from here — no `ui` pass ever again.
        let deadline = std::time::Instant::now()
            + crate::testing::load_aware_timeout(std::time::Duration::from_secs(30));
        let mut result = None;
        while result.is_none() && std::time::Instant::now() < deadline {
            if runtime.needs_background_tick() {
                runtime.background_tick();
            }
            result = runtime.take_pending_commands().into_iter().find_map(|cmd| {
                match cmd {
                    crate::app::app_trait::AppCommand::ToolResult {
                        call_id,
                        output_json,
                        ..
                    } if call_id == "call-offscreen" => Some(output_json),
                    _ => None,
                }
            });
            std::thread::sleep(std::time::Duration::from_millis(20));
        }

        assert_eq!(
            result.expect(
                "an off-screen pane must still answer tool calls — no ui pass runs for a pane in \
                 an inactive context or under an occluded window"
            ),
            Some("{\"pong\": true}".to_string())
        );
    }

    #[test]
    fn headless_frame_fails_fast_when_the_guest_dies_at_import() {
        let app = tempdir().expect("app dir");
        std::fs::write(
            app.path().join("main.py"),
            "raise RuntimeError('broken on purpose')\n",
        )
        .expect("write app");
        let config = PythonLaunchConfig {
            app_id: "test.broken-guest".to_string(),
            app_dir: app.path().to_path_buf(),
            entry: app.path().join("main.py"),
            module_name: "main".to_string(),
            launch_args: Vec::new(),
            workspace_root: app.path().to_path_buf(),
            capabilities: Vec::new(),
            allowed_hosts: Vec::new(),
            theme: crate::ui::theme::colors_from_config(
                &crate::config::PlexiConfig::default(),
            )
            .to_theme_map(),
            state_scopes: crate::host::state_scope::default_scopes(),
            state_format: crate::host::state_scope::StateFormat::Json,
            context_root: app.path().to_path_buf(),
        };
        let err = run_headless_frame(&config, (480.0, 320.0), None)
            .expect_err("an entry that raises at import must fail the headless probe");
        // The invariant is the *cause*, not the wall clock: a guest that raises
        // at import is detected via its process-exit signal and reported as a
        // death, never mistaken for a live app or a poll timeout. Asserting on
        // elapsed time flakes under parallel-test CPU contention (the crash can
        // legitimately take longer than the poll budget to surface), so this
        // checks the error path itself.
        let WasmPythonError::BridgeJson(message) = &err else {
            panic!("guest death must surface as a BridgeJson error, got {err:?}");
        };
        assert!(
            message.contains("app exited before responding"),
            "broken guest must take the process-exit death path, not the timeout backstop: {message}"
        );
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
    fn manifest_cloud_execution_is_rejected() {
        // Cloud/remote execution (stints 0286/0287) is future work with no
        // launch path yet — a manifest declaring it must fail loudly rather
        // than silently launching locally. Mirrors the WASM launch-path guard
        // in `open_installed_wasm_app_pane`.
        let dir = tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("manifest.toml"),
            r#"
schema_version = 1

[app]
id = "cloud-py"
type = "app"
name = "Cloud Python"
entry = "main.py"
version = "0.1.0"

[runtime]
python_compat = true
execution = "cloud"
"#,
        )
        .expect("manifest");
        std::fs::write(dir.path().join("main.py"), "def view(): pass\n").expect("entry");

        let err = PythonLaunchConfig::from_manifest_file(dir.path()).unwrap_err();
        assert!(matches!(
            err,
            WasmPythonError::UnsupportedExecution { execution: "cloud" }
        ));
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
    fn read_host_log_tail_returns_whole_small_log() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("plexi.log");
        std::fs::write(
            &path,
            "[2026-07-18 01:00:00] [INFO] [plexi::boot] up\n\
             [2026-07-18 01:00:01] [ERROR] [app::todo] boom\n",
        )
        .expect("write log");

        let text = read_host_log_tail(&path, DEFAULT_HOST_LOG_TAIL_BYTES).expect("read tail");

        assert!(text.contains("[plexi::boot] up"));
        assert!(text.contains("[app::todo] boom"));
    }

    #[test]
    fn read_host_log_tail_drops_partial_leading_line_when_truncated() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("plexi.log");
        // Three whole lines; a tiny window forces a mid-line seek so the first
        // returned line must be dropped rather than handed back half-formed.
        std::fs::write(
            &path,
            "AAAAAAAAAAAAAAAAAAAA\nBBBBBBBBBBBBBBBBBBBB\nCCCCCCCCCCCCCCCCCCCC\n",
        )
        .expect("write log");

        let text = read_host_log_tail(&path, 25).expect("read tail");

        assert!(
            !text.contains("AAAA"),
            "partial leading line must be dropped, got {text:?}"
        );
        assert!(text.contains("CCCCCCCCCCCCCCCCCCCC"));
        assert!(
            text.lines().all(|line| line.len() == 20),
            "every returned line must be whole, got {text:?}"
        );
    }

    #[test]
    fn read_host_log_tail_reports_missing_log() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("does-not-exist.log");

        let err = read_host_log_tail(&path, DEFAULT_HOST_LOG_TAIL_BYTES).unwrap_err();

        assert!(err.contains("open"), "error must name the failed op: {err}");
        assert!(err.contains("does-not-exist.log"));
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

    /// Stint 0674: a field declares itself the pane's default text surface with
    /// `autofocus`. Absent, it must decode false — an app that never asked for
    /// focus must never steal it.
    #[test]
    fn ui_tree_decodes_text_input_autofocus() {
        let tree = decode_ui_tree(
            r#"{
                "root":0,
                "nodes":[
                    {"id":0,"key":"0","data":{"type":"Column","children":[1,2],"gap":4.0}},
                    {"id":1,"key":"0/a","data":{"type":"TextInput","value":"","placeholder":"p","autofocus":true}},
                    {"id":2,"key":"0/b","data":{"type":"TextInput","value":"","placeholder":"p"}}
                ]
            }"#,
        )
        .expect("tree");

        let autofocus = |node: &IndexedNode| match &node.data {
            UiNodeData::TextInput(ti) => ti.autofocus,
            other => panic!("expected a TextInput, got {other:?}"),
        };
        assert!(autofocus(&tree.nodes[1]));
        assert!(!autofocus(&tree.nodes[2]));
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

    fn encoded_value(json_text: &str) -> Value {
        serde_json::from_str(json_text).expect("valid arena JSON")
    }

    // Full frame with a text node and a canvas node (background + one moving
    // rect), matching the shape breakout emits.
    const FULL_TREE_JSON: &str = r##"{
        "root": 0,
        "nodes": [
            {"id":0,"key":"0","data":{"type":"Column","children":[1,2],"align":"start","grow":false}},
            {"id":1,"key":"0/0","data":{"type":"Text","text":"score=0"}},
            {"id":2,"key":"0/1","data":{"type":"canvas","width":640.0,"height":360.0,"grow":true,"fit":"fill","commands":[
                {"type":"rect","x":0.0,"y":0.0,"w":640.0,"h":360.0,"fill":"#000000","radius":0.0},
                {"type":"rect","x":0.0,"y":100.0,"w":20.0,"h":20.0,"fill":"#ff0000","radius":0.0}
            ]}}
        ]
    }"##;

    #[test]
    fn full_tree_frame_decodes_with_canvas_fit() {
        let tree = decode_python_ui_tree_value(&encoded_value(FULL_TREE_JSON)).expect("full tree");
        assert_eq!(tree.tree.nodes.len(), 3);
        assert_eq!(
            tree.canvas_fits.get(&2),
            Some(&super::super::wasm_render::CanvasFit::Fill)
        );
    }

    #[test]
    fn delta_replaces_full_node_matching_equivalent_full_decode() {
        let base = decode_python_ui_tree_value(&encoded_value(FULL_TREE_JSON)).expect("base");
        // The text node (arena slot 1) changed to score=7: a full-node patch.
        let changed = [encoded_value(
            r#"{"id":1,"key":"0/0","data":{"type":"Text","text":"score=7"}}"#,
        )];
        let patched = apply_tree_delta(&base, &changed).expect("apply delta");

        let UiNodeData::Text(text) = &patched.tree.nodes[1].data else {
            panic!("expected text node");
        };
        assert_eq!(text.text, "score=7");
        // Untouched slots are identical to the base.
        assert_eq!(
            format!("{:?}", patched.tree.nodes[2]),
            format!("{:?}", base.tree.nodes[2])
        );
    }

    #[test]
    fn delta_patches_only_named_canvas_commands() {
        let base = decode_python_ui_tree_value(&encoded_value(FULL_TREE_JSON)).expect("base");
        // Move only the second rect (command index 1); background stays put.
        let changed = [encoded_value(
            r##"{"id":2,"key":"0/1","commands_changed":[[1,{"type":"rect","x":120.0,"y":100.0,"w":20.0,"h":20.0,"fill":"#ff0000","radius":0.0}]]}"##,
        )];
        let patched = apply_tree_delta(&base, &changed).expect("apply canvas delta");

        let UiNodeData::Canvas(canvas) = &patched.tree.nodes[2].data else {
            panic!("expected canvas node");
        };
        let CanvasCommand::Rect(background) = &canvas.commands[0] else {
            panic!("expected background rect");
        };
        assert_eq!(background.x, 0.0); // unchanged
        let CanvasCommand::Rect(moved) = &canvas.commands[1] else {
            panic!("expected moved rect");
        };
        assert_eq!(moved.x, 120.0); // patched in place
    }

    #[test]
    fn delta_out_of_range_id_is_a_desync_error() {
        let base = decode_python_ui_tree_value(&encoded_value(FULL_TREE_JSON)).expect("base");
        let changed = [encoded_value(
            r#"{"id":99,"key":"x","data":{"type":"Empty"}}"#,
        )];
        assert!(apply_tree_delta(&base, &changed).is_err());
    }

    #[test]
    fn delta_commands_changed_on_non_canvas_is_a_desync_error() {
        let base = decode_python_ui_tree_value(&encoded_value(FULL_TREE_JSON)).expect("base");
        // Slot 1 is a Text node, not a canvas.
        let changed = [encoded_value(
            r#"{"id":1,"key":"0/0","commands_changed":[[0,{"type":"rect"}]]}"#,
        )];
        assert!(apply_tree_delta(&base, &changed).is_err());
    }

    #[test]
    fn delta_out_of_range_command_index_is_a_desync_error() {
        let base = decode_python_ui_tree_value(&encoded_value(FULL_TREE_JSON)).expect("base");
        let changed = [encoded_value(
            r#"{"id":2,"key":"0/1","commands_changed":[[9,{"type":"rect"}]]}"#,
        )];
        assert!(apply_tree_delta(&base, &changed).is_err());
    }

    #[test]
    fn decoder_thread_emits_full_then_delta_frames() {
        let stdout = DrainableOutput::default();
        let stdin = AppendableStdin::default();
        let (tx, rx) = std::sync::mpsc::channel();
        let repaint: RepaintHook = Arc::new(Mutex::new(None));
        let reader = stdout.clone();
        let queued = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let thread_queued = queued.clone();
        let handle = std::thread::spawn(move || {
            decode_loop(reader, stdin, "test".to_string(), tx, repaint, thread_queued);
        });

        // Serialize to one line each — the decoder splits on '\n', so the
        // pretty-printed FULL_TREE_JSON must be collapsed first.
        let full = serde_json::to_string(&json!({
            "type": "component_tree",
            "frame_id": 1,
            "tree": encoded_value(FULL_TREE_JSON),
        }))
        .expect("serialize full frame");
        stdout.push_bytes(full.as_bytes());
        stdout.push_bytes(b"\n");
        stdout.push_bytes(
            br#"{"type":"tree_delta","frame_id":2,"changed":[{"id":1,"key":"0/0","data":{"type":"Text","text":"score=7"}}]}"#,
        );
        stdout.push_bytes(b"\n");

        let first = rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("first frame");
        let DecodedOutput::Tree { frame_id, tree, .. } = first else {
            panic!("expected full tree frame");
        };
        assert_eq!(frame_id, Some(1));
        assert_eq!(tree.tree.nodes.len(), 3);

        let second = rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("second frame");
        let DecodedOutput::Tree { frame_id, tree, .. } = second else {
            panic!("expected delta-reconstructed frame");
        };
        assert_eq!(frame_id, Some(2));
        let UiNodeData::Text(text) = &tree.tree.nodes[1].data else {
            panic!("expected text node");
        };
        assert_eq!(text.text, "score=7");

        // Stint 0688: the decoder counts every output it sends, so an off-screen
        // pane can be asked "do you have work?" without consuming anything. Both
        // frames are counted here — this test reads the raw channel, and only
        // `PythonOutputDecoder::try_recv` decrements.
        assert_eq!(queued.load(std::sync::atomic::Ordering::Acquire), 2);

        stdout.close();
        handle.join().expect("decoder thread");
    }

    #[test]
    fn decoder_thread_requests_full_resync_on_unapplyable_delta() {
        let stdout = DrainableOutput::default();
        let stdin = AppendableStdin::default();
        let stdin_probe = stdin.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let repaint: RepaintHook = Arc::new(Mutex::new(None));
        let reader = stdout.clone();
        let handle = std::thread::spawn(move || {
            decode_loop(
                reader,
                stdin,
                "test".to_string(),
                tx,
                repaint,
                Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            );
        });

        // A delta with no prior full tree cannot be applied: fail-loud resync.
        stdout.push_bytes(b"{\"type\":\"tree_delta\",\"frame_id\":1,\"changed\":[]}\n");

        // The decoder must have asked the guest for a full tree, and emitted no
        // frame for the dropped delta.
        let request = drain_appendable_stdin(&stdin_probe, std::time::Duration::from_secs(2));
        assert!(
            request.contains("request_full_tree"),
            "expected resync request, got {request:?}"
        );
        assert!(
            rx.try_recv().is_err(),
            "no frame should be emitted for an unapplyable delta"
        );

        stdout.close();
        handle.join().expect("decoder thread");
    }

    // Poll the guest stdin the decoder writes into until a full line arrives.
    fn drain_appendable_stdin(stdin: &AppendableStdin, timeout: std::time::Duration) -> String {
        let waker = Waker::from(Arc::new(NoopWake));
        let mut context = Context::from_waker(&waker);
        let mut collected = Vec::new();
        let deadline = std::time::Instant::now() + timeout;
        let mut input = stdin.clone();
        while std::time::Instant::now() < deadline {
            let mut bytes = [0_u8; 256];
            let mut read = ReadBuf::new(&mut bytes);
            if Pin::new(&mut input)
                .poll_read(&mut context, &mut read)
                .is_ready()
            {
                let filled = read.filled();
                if !filled.is_empty() {
                    collected.extend_from_slice(filled);
                    if collected.contains(&b'\n') {
                        break;
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        String::from_utf8_lossy(&collected).into_owned()
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

        // Calc declares its connector tools on init; the bridge must carry the
        // whole set across with schemas re-serialized into the WIT string fields.
        let declared = init
            .iter()
            .find_map(|effect| match effect {
                PythonBridgeEffect::Host(Effect::DeclareTools(req)) => Some(req),
                _ => None,
            })
            .expect("calc init must declare tools");
        let names: Vec<&str> = declared.tools.iter().map(|t| t.name.as_str()).collect();
        for expected in [
            "calc.add",
            "calc.subtract",
            "calc.multiply",
            "calc.divide",
            "calc.evaluate",
        ] {
            assert!(
                names.contains(&expected),
                "missing declared tool {expected} in {names:?}"
            );
        }
        for tool in &declared.tools {
            assert!(tool.read_only, "{} must be read-only", tool.name);
            assert!(
                serde_json::from_str::<Value>(&tool.input_schema_json)
                    .expect("input schema json")
                    .is_object(),
                "{} input schema must cross the bridge as a JSON object string",
                tool.name
            );
            assert!(serde_json::from_str::<Value>(&tool.output_schema_json)
                .expect("output schema json")
                .is_object());
        }
    }

    #[test]
    fn native_python_bridge_decodes_tool_result_effect() {
        let effects = decode_effects(
            r#"[{"type":"ToolResult","call_id":"call-1","output_json":"{\"result\":12}","error":null}]"#,
        )
        .expect("tool result effect");
        let PythonBridgeEffect::Host(Effect::ToolResult(result)) = &effects[0] else {
            panic!("expected ToolResult, got {:?}", effects[0]);
        };
        assert_eq!(result.call_id, "call-1");
        assert_eq!(result.output_json.as_deref(), Some(r#"{"result":12}"#));
        assert!(result.error.is_none());
    }

    #[test]
    fn native_python_bridge_rejects_non_object_tool_schema() {
        let err = decode_effects(
            r#"[{"type":"ExposeTools","tools":[{"name":"t","description":"d","input_schema":"nope","output_schema":{},"timeout_ms":null,"read_only":true}]}]"#,
        )
        .expect_err("a string schema must not silently decode");
        assert!(
            format!("{err}").contains("input_schema"),
            "error must name the offending field: {err}"
        );
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
        // Stint 0389: `AppBar` decodes to its own live `UiNodeData::AppBar`
        // node now, not a downgraded `Text` node containing the title.
        let view = native_python_app_view("kraken", "main");

        assert!(view.nodes.iter().any(
            |node| matches!(&node.data, UiNodeData::AppBar(bar) if bar.title.contains("Kraken"))
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

    #[test]
    fn decode_badge_color_accepts_canonical_roles() {
        assert!(matches!(
            decode_badge_color("accent"),
            Ok(BadgeColor::Accent)
        ));
        assert!(matches!(
            decode_badge_color("success"),
            Ok(BadgeColor::Success)
        ));
        assert!(matches!(
            decode_badge_color("warning"),
            Ok(BadgeColor::Warning)
        ));
        assert!(matches!(
            decode_badge_color("danger"),
            Ok(BadgeColor::Danger)
        ));
        assert!(matches!(
            decode_badge_color("neutral"),
            Ok(BadgeColor::Neutral)
        ));
    }

    #[test]
    fn decode_badge_color_accepts_theme_status_aliases() {
        // sdk/python/plexi_sdk/_theme.py defines red==danger, green==success,
        // yellow==warning as theme role aliases; the host decoder must accept
        // them alongside the canonical badge color names.
        assert!(matches!(decode_badge_color("red"), Ok(BadgeColor::Danger)));
        assert!(matches!(
            decode_badge_color("green"),
            Ok(BadgeColor::Success)
        ));
        assert!(matches!(
            decode_badge_color("yellow"),
            Ok(BadgeColor::Warning)
        ));
    }

    #[test]
    fn decode_badge_color_rejects_unknown_value() {
        // There is intentionally no "blue" role; "accent" is the accent/
        // blue-ish role. Unknown values must fail loudly, naming the value.
        let err = decode_badge_color("blue").expect_err("blue is not a badge color");
        match err {
            WasmPythonError::BridgeJson(msg) => {
                assert!(
                    msg.contains("blue"),
                    "error should name the bad value: {msg}"
                );
            }
            other => panic!("expected BridgeJson error, got {other:?}"),
        }
    }

    // ─── Binary-safe file I/O (stint 0509) ───────────────────────────────────

    #[test]
    fn file_write_content_b64_round_trips_binary_exact() {
        // Every byte value, including invalid UTF-8 sequences and NULs.
        let payload: Vec<u8> = (0..=255u8).cycle().take(1024).collect();
        let message = json!({
            "type": "file_write",
            "path": "out.bin",
            "content_b64": BASE64.encode(&payload),
        });
        let decoded = decode_file_write_content(&message).expect("b64 payload decodes");
        assert_eq!(decoded, payload);
    }

    #[test]
    fn file_write_text_content_still_writes_utf8_bytes() {
        let message = json!({"type": "file_write", "path": "out.txt", "content": "héllo"});
        let decoded = decode_file_write_content(&message).expect("text payload decodes");
        assert_eq!(decoded, "héllo".as_bytes());
    }

    #[test]
    fn file_write_rejects_missing_both_and_ambiguous_content() {
        let neither = json!({"type": "file_write", "path": "out.bin"});
        let err = decode_file_write_content(&neither).expect_err("no payload must fail");
        assert!(err.contains("content"), "error names the missing field: {err}");

        let both = json!({
            "type": "file_write",
            "path": "out.bin",
            "content": "a",
            "content_b64": "YQ==",
        });
        let err = decode_file_write_content(&both).expect_err("ambiguous payload must fail");
        assert!(err.contains("exactly one"), "error explains the fix: {err}");
    }

    #[test]
    fn file_write_rejects_invalid_base64_loudly() {
        let message = json!({"type": "file_write", "path": "out.bin", "content_b64": "!!not-b64"});
        let err = decode_file_write_content(&message).expect_err("bad base64 must fail");
        assert!(err.contains("base64"), "error names the encoding: {err}");
    }

    #[test]
    fn file_write_rejects_oversize_payload_with_named_limit() {
        // An empty-ish base64 string can't be oversize, so build one just over
        // the cap. b64 of N bytes is 4*ceil(N/3) chars; keep it cheap with a
        // repeated 'A' payload (decodes to zero bytes).
        let over = crate::host::MAX_FILE_IO_BYTES + 3;
        let b64_len = over.div_ceil(3) * 4;
        let message = json!({
            "type": "file_write",
            "path": "out.bin",
            "content_b64": "A".repeat(b64_len),
        });
        let err = decode_file_write_content(&message).expect_err("oversize must fail");
        assert!(
            err.contains(&crate::host::MAX_FILE_IO_BYTES.to_string()),
            "error names the limit: {err}"
        );
    }

    #[test]
    fn jailed_path_rejects_absolute_parent_and_symlink_escape() {
        let root = tempdir().expect("tempdir");
        let outside = tempdir().expect("outside tempdir");
        std::fs::write(outside.path().join("secret.txt"), b"x").expect("seed outside file");

        let abs = outside.path().join("secret.txt");
        let err = resolve_jailed_path(root.path(), abs.to_str().expect("utf8"), false)
            .expect_err("absolute path must be jailed");
        assert!(err.contains("escapes workspace"), "{err}");

        let err = resolve_jailed_path(root.path(), "../secret.txt", false)
            .expect_err("parent traversal must be jailed");
        assert!(err.contains("escapes workspace"), "{err}");

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(outside.path(), root.path().join("link"))
                .expect("create escape symlink");
            let err = resolve_jailed_path(root.path(), "link/secret.txt", false)
                .expect_err("symlink escape must be jailed");
            assert!(err.contains("escapes workspace"), "{err}");
        }
    }

    #[test]
    fn jailed_path_allows_new_file_in_existing_subdir_for_write() {
        let root = tempdir().expect("tempdir");
        std::fs::create_dir(root.path().join("media")).expect("create subdir");
        let resolved = resolve_jailed_path(root.path(), "media/out.wav", true)
            .expect("new file under existing subdir resolves");
        assert!(resolved.ends_with("media/out.wav"));
        assert!(resolved.starts_with(root.path().canonicalize().expect("canonical root")));
    }
}

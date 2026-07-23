//! HostServices — host side-effect plumbing.
//!
//! `EventSink` is the only production field right now; fs/secrets/net/spawn
//! trait seams live in the app runtimes (`host::wasm_python`, `host::wasm_app`)
//! and `app_registry` until HostModel owns their routing.

use crate::host::effect::HostEffect;
use std::collections::HashMap;

// ── EventSink ──────────────────────────────────────────────────────────────

pub trait EventSink: Send {
    fn emit(&mut self, effect: &HostEffect);
}

/// Append-only JSONL sink. One line per `HostEffect` written to the configured
/// path. Production wires this to `~/.plexi-v3/events.jsonl` so every command
/// path leaves a durable audit trail and the Runs palette + future agent
/// consumers have a single source of truth.
pub struct FileEventSink {
    path: std::path::PathBuf,
    writer: Option<std::io::BufWriter<std::fs::File>>,
}

impl FileEventSink {
    pub fn new(path: std::path::PathBuf) -> Self {
        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                log::warn!(
                    "FileEventSink: create_dir_all({}) failed: {e}",
                    parent.display()
                );
            }
        }
        let writer = match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            Ok(f) => Some(std::io::BufWriter::new(f)),
            Err(e) => {
                log::error!("FileEventSink: open({}) failed: {e}", path.display());
                None
            }
        };
        let mut sink = Self { path, writer };
        // Startup heartbeat so the post-install smoke test can assert the
        // sink opened the file for write (non-empty file means the wiring
        // is live, even before the first AppRequest fires).
        if let Some(writer) = sink.writer.as_mut() {
            use std::io::Write;
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let line = format!("{{\"kind\":\"sink_opened\",\"timestamp\":{now}}}\n");
            if let Err(e) = writer.write_all(line.as_bytes()).and_then(|()| writer.flush()) {
                log::debug!(
                    "FileEventSink: startup heartbeat write({}) failed: {e}",
                    sink.path.display()
                );
            }
        }
        sink
    }
}

impl EventSink for FileEventSink {
    fn emit(&mut self, effect: &HostEffect) {
        use std::io::Write;
        let Some(writer) = self.writer.as_mut() else {
            return;
        };
        match serde_json::to_string(effect) {
            Ok(mut line) => {
                line.push('\n');
                if let Err(e) = writer.write_all(line.as_bytes()) {
                    log::warn!("FileEventSink write({}) failed: {e}", self.path.display());
                }
                if let Err(e) = writer.flush() {
                    log::debug!("FileEventSink flush({}) failed: {e}", self.path.display());
                }
            }
            Err(e) => log::warn!("FileEventSink serialize failed: {e}"),
        }
    }
}

// ── NetService ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub body: String,
    pub error: Option<String>,
    pub response_headers: std::collections::HashMap<String, Vec<String>>,
}

/// Host-side HTTP broker. `Send + Sync` so a single handle can be shared across
/// per-pane WASM runtimes that all call out concurrently.
pub trait NetService: Send + Sync {
    /// Issue a synchronous HTTP request. Implementations must never panic;
    /// transport errors are returned as `HttpResponse { status: 0, error: Some(..) }`.
    fn http(
        &self,
        method: &str,
        url: &str,
        headers: &HashMap<String, String>,
        body: Option<&str>,
    ) -> HttpResponse;
}

/// Production impl — blocking HTTP via `ureq`. Pure-Rust, no tokio.
/// 10s connect timeout, 30s total timeout so apps cannot wedge the host
/// forever on a hung peer.
pub struct UreqNetService {
    agent: ureq::Agent,
}

impl UreqNetService {
    pub fn new() -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(30))
            .redirects(0)
            .build();
        Self { agent }
    }
}

impl Default for UreqNetService {
    fn default() -> Self {
        Self::new()
    }
}

impl UreqNetService {
    fn collect_headers(resp: &ureq::Response) -> HashMap<String, Vec<String>> {
        let mut map: HashMap<String, Vec<String>> = HashMap::new();
        for name in resp.headers_names() {
            if let Some(value) = resp.header(&name) {
                map.entry(name.to_lowercase())
                    .or_default()
                    .push(value.to_string());
            }
        }
        map
    }
}

impl NetService for UreqNetService {
    fn http(
        &self,
        method: &str,
        url: &str,
        headers: &HashMap<String, String>,
        body: Option<&str>,
    ) -> HttpResponse {
        let mut req = self.agent.request(method, url);
        for (k, v) in headers {
            req = req.set(k, v);
        }
        let response = match body {
            Some(b) => req.send_string(b),
            None => req.call(),
        };
        match response {
            Ok(resp) => {
                let status = resp.status();
                let response_headers = Self::collect_headers(&resp);
                let body_text = resp.into_string().unwrap_or_default();
                HttpResponse {
                    status,
                    body: body_text,
                    error: None,
                    response_headers,
                }
            }
            Err(ureq::Error::Status(status, resp)) => {
                // Non-2xx — ureq returns this as Err, but the caller still
                // wants the body + status for real diagnostics (e.g. 429
                // Retry-After bodies or GitHub's JSON error payloads).
                let response_headers = Self::collect_headers(&resp);
                let body_text = resp.into_string().unwrap_or_default();
                HttpResponse {
                    status,
                    body: body_text,
                    error: None,
                    response_headers,
                }
            }
            Err(ureq::Error::Transport(t)) => {
                log::warn!("UreqNetService: transport error for {method} {url}: {t}");
                HttpResponse {
                    status: 0,
                    body: String::new(),
                    error: Some(format!("transport: {t}")),
                    response_headers: HashMap::new(),
                }
            }
        }
    }
}

// ── PickerService ──────────────────────────────────────────────────────────

/// One `DrawCommand::OpenFilePicker` request, decoded from the wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilePickRequest {
    /// File extensions without leading dots; empty = accept all files.
    pub filter: Vec<String>,
    /// Allow selecting more than one file (`Open` mode only).
    pub multiple: bool,
    pub mode: crate::app_protocol::FilePickerMode,
}

/// What the picker resolved to. `Picked` paths are exactly what the dialog
/// (or script) returned — canonicalization and grant registration happen at
/// the pane, not here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilePickOutcome {
    Picked(Vec<std::path::PathBuf>),
    Cancelled,
}

/// Host-side file-picker seam (stint 0508). Production shows a native dialog
/// via `rfd`; tests and headless tester agents inject `ScriptedPickerService`
/// so the full pick → grant → read/write flow runs without a human clicking
/// a dialog — a native dialog is the one thing no agent can click.
///
/// `pick` blocks the calling thread until the dialog resolves; call it from a
/// background thread, never the main/paint thread.
pub trait PickerService: Send + Sync {
    fn pick(&self, request: &FilePickRequest) -> FilePickOutcome;
}

/// Production impl — native dialog via `rfd`'s async API, driven to
/// completion on the calling (background) thread.
pub struct RfdPickerService;

impl PickerService for RfdPickerService {
    fn pick(&self, request: &FilePickRequest) -> FilePickOutcome {
        use crate::app_protocol::FilePickerMode;
        let mut dialog = rfd::AsyncFileDialog::new();
        if !request.filter.is_empty() && request.mode != FilePickerMode::Folder {
            let extensions: Vec<&str> = request.filter.iter().map(String::as_str).collect();
            dialog = dialog.add_filter("files", &extensions);
        }
        let paths: Vec<std::path::PathBuf> = match request.mode {
            FilePickerMode::Open if request.multiple => {
                block_on_dialog(dialog.pick_files()).map_or_else(Vec::new, |handles| {
                    handles
                        .iter()
                        .map(|handle| handle.path().to_path_buf())
                        .collect()
                })
            }
            FilePickerMode::Open => block_on_dialog(dialog.pick_file())
                .map_or_else(Vec::new, |handle| vec![handle.path().to_path_buf()]),
            FilePickerMode::Folder => block_on_dialog(dialog.pick_folder())
                .map_or_else(Vec::new, |handle| vec![handle.path().to_path_buf()]),
            FilePickerMode::Save => block_on_dialog(dialog.save_file())
                .map_or_else(Vec::new, |handle| vec![handle.path().to_path_buf()]),
        };
        if paths.is_empty() {
            FilePickOutcome::Cancelled
        } else {
            FilePickOutcome::Picked(paths)
        }
    }
}

/// Drive one rfd dialog future to completion on the current thread.
/// Blocks; must not run on the main thread.
pub(crate) fn block_on_dialog<F: std::future::Future>(f: F) -> F::Output {
    use std::pin::pin;
    use std::sync::{Arc, Condvar, Mutex};
    use std::task::{Context, Poll, Wake, Waker};

    struct Signal(Arc<(Mutex<bool>, Condvar)>);
    impl Signal {
        fn signal(&self) {
            let (lock, cvar) = &*self.0;
            *lock.lock().unwrap() = true;
            cvar.notify_one();
        }
    }
    impl Wake for Signal {
        fn wake(self: Arc<Self>) {
            self.signal();
        }
        fn wake_by_ref(self: &Arc<Self>) {
            self.signal();
        }
    }

    let signal = Arc::new((Mutex::new(false), Condvar::new()));
    let waker: Waker = Arc::new(Signal(Arc::clone(&signal))).into();
    let mut cx = Context::from_waker(&waker);
    let mut f = pin!(f);
    loop {
        match f.as_mut().poll(&mut cx) {
            Poll::Ready(val) => return val,
            Poll::Pending => {
                let (lock, cvar) = &*signal;
                let mut ready = lock.lock().unwrap();
                while !*ready {
                    ready = cvar.wait(ready).unwrap();
                }
                *ready = false;
            }
        }
    }
}

/// Scripted impl — resolves each request from a queued list of outcomes, in
/// order, without showing any dialog. Selected in a live host via
/// `PLEXI_PICKER_SCRIPT` (a JSON file of outcomes, read per pane at launch)
/// and injected directly in harness tests and scenes.
pub struct ScriptedPickerService {
    queue: std::sync::Mutex<std::collections::VecDeque<FilePickOutcome>>,
}

impl ScriptedPickerService {
    pub fn from_outcomes(outcomes: Vec<FilePickOutcome>) -> Self {
        Self {
            queue: std::sync::Mutex::new(outcomes.into()),
        }
    }

    /// Parse a script file: a JSON array where each entry is either
    /// `{"paths": ["/abs/one", ...]}` or `{"cancel": true}`. Entries are
    /// consumed in order, one per `OpenFilePicker` request. Each pane reads
    /// the file at launch, so every pane starts with the full list.
    pub fn from_script_file(path: &std::path::Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|error| format!("read picker script {}: {error}", path.display()))?;
        let entries: Vec<serde_json::Value> = serde_json::from_str(&text)
            .map_err(|error| format!("parse picker script {}: {error}", path.display()))?;
        let mut outcomes = Vec::new();
        for entry in &entries {
            if entry.get("cancel").and_then(serde_json::Value::as_bool) == Some(true) {
                outcomes.push(FilePickOutcome::Cancelled);
                continue;
            }
            let Some(paths) = entry.get("paths").and_then(serde_json::Value::as_array) else {
                return Err(format!(
                    "picker script {}: entry must be {{\"paths\": [..]}} or {{\"cancel\": true}}, got {entry}",
                    path.display()
                ));
            };
            let paths: Vec<std::path::PathBuf> = paths
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(std::path::PathBuf::from)
                .collect();
            outcomes.push(FilePickOutcome::Picked(paths));
        }
        Ok(Self::from_outcomes(outcomes))
    }
}

impl PickerService for ScriptedPickerService {
    fn pick(&self, request: &FilePickRequest) -> FilePickOutcome {
        let next = self.queue.lock().unwrap().pop_front();
        match next {
            Some(outcome) => outcome,
            None => {
                log::error!(
                    "ScriptedPickerService: script exhausted, cancelling pick request {request:?}"
                );
                FilePickOutcome::Cancelled
            }
        }
    }
}

/// The picker backend a newly launched app pane should use. Priority:
/// in-process override (scenes, tests) → `PLEXI_PICKER_SCRIPT` env (live
/// hosts driven by tester agents) → native `rfd` dialog.
pub fn default_picker_service() -> std::sync::Arc<dyn PickerService> {
    #[cfg(test)]
    if let Some(service) = picker_override() {
        return service;
    }
    match std::env::var_os("PLEXI_PICKER_SCRIPT") {
        Some(script_path) => {
            let script_path = std::path::PathBuf::from(script_path);
            match ScriptedPickerService::from_script_file(&script_path) {
                Ok(service) => {
                    log::info!(
                        "picker: using scripted backend from PLEXI_PICKER_SCRIPT={}",
                        script_path.display()
                    );
                    std::sync::Arc::new(service)
                }
                Err(error) => {
                    // Fail loud but safe: a broken script must never fall back
                    // to a real dialog under a tester agent — every pick
                    // cancels and the error names the file to fix.
                    log::error!("picker: {error}; all picks will cancel");
                    std::sync::Arc::new(ScriptedPickerService::from_outcomes(Vec::new()))
                }
            }
        }
        None => std::sync::Arc::new(RfdPickerService),
    }
}

/// In-process picker override, installed by the scene runner (and tests) so
/// headless suites never mutate process env. `None` clears it. The scene
/// runner is `#[cfg(test)]`, so the whole override plumbing is too — a
/// production host scripts its picker only via `PLEXI_PICKER_SCRIPT`.
#[cfg(test)]
pub fn set_picker_override(service: Option<std::sync::Arc<dyn PickerService>>) {
    *picker_override_slot().lock().unwrap() = service;
}

#[cfg(test)]
fn picker_override() -> Option<std::sync::Arc<dyn PickerService>> {
    picker_override_slot().lock().unwrap().clone()
}

#[cfg(test)]
fn picker_override_slot()
-> &'static std::sync::Mutex<Option<std::sync::Arc<dyn PickerService>>> {
    static SLOT: std::sync::OnceLock<std::sync::Mutex<Option<std::sync::Arc<dyn PickerService>>>> =
        std::sync::OnceLock::new();
    SLOT.get_or_init(|| std::sync::Mutex::new(None))
}

// ── HostServices aggregate ─────────────────────────────────────────────────

pub struct HostServices {
    pub event_sink: Box<dyn EventSink>,
}

impl HostServices {
    /// Production wiring — real event bus.
    /// `event_sink` appends to `<config_dir>/effects.jsonl` so every HostEffect
    /// leaves a durable audit trail.
    pub fn new() -> Self {
        let effects_path = crate::config::config_dir().join("effects.jsonl");
        Self {
            event_sink: Box::new(FileEventSink::new(effects_path)),
        }
    }
}

impl Default for HostServices {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_protocol::FilePickerMode;

    fn any_request() -> FilePickRequest {
        FilePickRequest {
            filter: Vec::new(),
            multiple: false,
            mode: FilePickerMode::Open,
        }
    }

    /// Scripted outcomes resolve in order; an exhausted script cancels
    /// instead of ever reaching a real dialog.
    #[test]
    fn scripted_picker_consumes_outcomes_in_order_then_cancels() {
        let picker = ScriptedPickerService::from_outcomes(vec![
            FilePickOutcome::Picked(vec!["/tmp/a.txt".into()]),
            FilePickOutcome::Cancelled,
        ]);
        assert_eq!(
            picker.pick(&any_request()),
            FilePickOutcome::Picked(vec!["/tmp/a.txt".into()])
        );
        assert_eq!(picker.pick(&any_request()), FilePickOutcome::Cancelled);
        assert_eq!(
            picker.pick(&any_request()),
            FilePickOutcome::Cancelled,
            "exhausted script must cancel, never dialog"
        );
    }

    /// The `PLEXI_PICKER_SCRIPT` file format round-trips: `paths` entries
    /// pick, `cancel` entries cancel, anything else is a named error.
    #[test]
    fn scripted_picker_parses_script_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let script = dir.path().join("picks.json");
        std::fs::write(
            &script,
            r#"[{"paths": ["/tmp/one.txt", "/tmp/two.txt"]}, {"cancel": true}]"#,
        )
        .expect("write script");

        let picker = ScriptedPickerService::from_script_file(&script).expect("parse");
        assert_eq!(
            picker.pick(&any_request()),
            FilePickOutcome::Picked(vec!["/tmp/one.txt".into(), "/tmp/two.txt".into()])
        );
        assert_eq!(picker.pick(&any_request()), FilePickOutcome::Cancelled);

        std::fs::write(&script, r#"[{"bogus": 1}]"#).expect("write bad script");
        let error = ScriptedPickerService::from_script_file(&script)
            .err()
            .expect("malformed script entry must fail");
        assert!(error.contains("picks.json"), "error names the file: {error}");
    }
}

//! HostServices — trait-object seam for every host side effect.
//!
//! Split out so tests can inject mocks: a Layer-2 harness instantiates
//! `HostModel` with `HostServices::mock()` and the state machine never
//! touches the real keychain, filesystem, or network. Production uses
//! `HostServices::new()` which wires the concrete implementations in
//! `secrets.rs`, `std::fs`, `ureq`, and `AppRegistry`.

use crate::host::effect::HostEffect;
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

// ── EventSink ──────────────────────────────────────────────────────────────

pub trait EventSink: Send {
    fn emit(&mut self, effect: &HostEffect);
}

/// No-op sink — used in Layer-2 tests that don't care about event observation.
pub struct NoopEventSink;

impl EventSink for NoopEventSink {
    fn emit(&mut self, _effect: &HostEffect) {}
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
                log::warn!("FileEventSink: create_dir_all({}) failed: {e}", parent.display());
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
        // is live, even before the first HostCommand fires).
        if let Some(writer) = sink.writer.as_mut() {
            use std::io::Write;
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let line = format!("{{\"kind\":\"sink_opened\",\"timestamp\":{now}}}\n");
            let _ = writer.write_all(line.as_bytes());
            let _ = writer.flush();
        }
        sink
    }
}

impl EventSink for FileEventSink {
    fn emit(&mut self, effect: &HostEffect) {
        use std::io::Write;
        let Some(writer) = self.writer.as_mut() else { return };
        match serde_json::to_string(effect) {
            Ok(mut line) => {
                line.push('\n');
                if let Err(e) = writer.write_all(line.as_bytes()) {
                    log::warn!("FileEventSink write({}) failed: {e}", self.path.display());
                }
                let _ = writer.flush();
            }
            Err(e) => log::warn!("FileEventSink serialize failed: {e}"),
        }
    }
}

/// Accumulates all emitted effects into a vec. Tests only.
pub struct VecEventSink {
    pub events: Vec<HostEffect>,
}

impl VecEventSink {
    pub fn new() -> Self {
        Self { events: Vec::new() }
    }
}

impl Default for VecEventSink {
    fn default() -> Self {
        Self::new()
    }
}

impl EventSink for VecEventSink {
    fn emit(&mut self, effect: &HostEffect) {
        self.events.push(effect.clone());
    }
}

// ── FsService ──────────────────────────────────────────────────────────────

pub trait FsService: Send {
    fn read(&self, path: &Path) -> io::Result<Vec<u8>>;
    fn write(&mut self, path: &Path, bytes: &[u8]) -> io::Result<()>;
    fn exists(&self, path: &Path) -> bool;
}

pub struct RealFsService;

impl FsService for RealFsService {
    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        std::fs::read(path)
    }

    fn write(&mut self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        std::fs::write(path, bytes)
    }

    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }
}

/// Tests only — in-memory fake filesystem.
pub struct MockFsService {
    pub files: HashMap<PathBuf, Vec<u8>>,
}

impl MockFsService {
    pub fn new() -> Self {
        Self { files: HashMap::new() }
    }
}

impl Default for MockFsService {
    fn default() -> Self {
        Self::new()
    }
}

impl FsService for MockFsService {
    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        self.files
            .get(path)
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("mock fs miss: {}", path.display())))
    }

    fn write(&mut self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        self.files.insert(path.to_path_buf(), bytes.to_vec());
        Ok(())
    }

    fn exists(&self, path: &Path) -> bool {
        self.files.contains_key(path)
    }
}

// ── SecretsService ─────────────────────────────────────────────────────────

pub trait SecretsService: Send {
    /// Look up a workspace-scoped secret. Returns None when not found or
    /// when the scope is invalid (implementations log + refuse).
    fn get(&self, key: &str, app_id: &str, workspace_root: &Path) -> Option<String>;
}

/// Production impl — thin wrapper over `crate::secrets::get_secret_scoped`.
/// Keeps the production behavior (macOS Keychain, v3 scoped account key)
/// while routing through the trait so tests can substitute mocks.
pub struct KeychainSecretsService;

impl SecretsService for KeychainSecretsService {
    fn get(&self, key: &str, app_id: &str, workspace_root: &Path) -> Option<String> {
        #[cfg(target_os = "macos")]
        {
            crate::secrets::get_secret_scoped(key, app_id, workspace_root)
                .map(|z| z.to_string())
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (key, app_id, workspace_root);
            None
        }
    }
}

/// Tests only — key → value lookup, ignores scope.
pub struct MockSecretsService {
    pub values: HashMap<String, String>,
}

impl MockSecretsService {
    pub fn new() -> Self {
        Self { values: HashMap::new() }
    }

    pub fn with(mut self, key: &str, value: &str) -> Self {
        self.values.insert(key.to_string(), value.to_string());
        self
    }
}

impl Default for MockSecretsService {
    fn default() -> Self {
        Self::new()
    }
}

impl SecretsService for MockSecretsService {
    fn get(&self, key: &str, _app_id: &str, _workspace_root: &Path) -> Option<String> {
        self.values.get(key).cloned()
    }
}

// ── NetService ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub body: String,
    pub error: Option<String>,
}

/// Host-side HTTP broker. `Send + Sync` so a single handle can be shared across
/// per-pane `ProcessApp` instances that all call out concurrently.
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

    /// Convenience wrapper — GET with no custom headers.
    fn http_get(&self, url: &str) -> HttpResponse {
        self.http("GET", url, &HashMap::new(), None)
    }
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
            .build();
        Self { agent }
    }
}

impl Default for UreqNetService {
    fn default() -> Self {
        Self::new()
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
                let body_text = resp.into_string().unwrap_or_default();
                HttpResponse {
                    status,
                    body: body_text,
                    error: None,
                }
            }
            Err(ureq::Error::Status(status, resp)) => {
                // Non-2xx — ureq returns this as Err, but the caller still
                // wants the body + status for real diagnostics (e.g. 429
                // Retry-After bodies or GitHub's JSON error payloads).
                let body_text = resp.into_string().unwrap_or_default();
                HttpResponse {
                    status,
                    body: body_text,
                    error: None,
                }
            }
            Err(ureq::Error::Transport(t)) => {
                log::warn!(
                    "UreqNetService: transport error for {method} {url}: {t}"
                );
                HttpResponse {
                    status: 0,
                    body: String::new(),
                    error: Some(format!("transport: {t}")),
                }
            }
        }
    }
}

/// Tests only — URL → body map. Unknown URLs return 404.
pub struct MockNetService {
    pub responses: HashMap<String, String>,
}

impl MockNetService {
    pub fn new() -> Self {
        Self { responses: HashMap::new() }
    }

    pub fn with(mut self, url: &str, body: &str) -> Self {
        self.responses.insert(url.to_string(), body.to_string());
        self
    }
}

impl Default for MockNetService {
    fn default() -> Self {
        Self::new()
    }
}

impl NetService for MockNetService {
    fn http(
        &self,
        _method: &str,
        url: &str,
        _headers: &HashMap<String, String>,
        _body: Option<&str>,
    ) -> HttpResponse {
        match self.responses.get(url) {
            Some(body) => HttpResponse {
                status: 200,
                body: body.clone(),
                error: None,
            },
            None => HttpResponse {
                status: 404,
                body: String::new(),
                error: Some(format!("no mock for {url}")),
            },
        }
    }
}

// ── SpawnService ───────────────────────────────────────────────────────────

/// Test seam for app spawn. Production implementation lives in
/// `process_app` / `app_registry`; kept here as a trait so Layer-2 tests
/// can observe spawn requests without touching subprocesses.
pub trait SpawnService: Send {
    /// Record that the host would spawn `app_id` with the given args.
    /// Returns `Ok(pane_id)` when the spawn is accepted.
    fn spawn(
        &mut self,
        app_id: &str,
        workspace_root: &Path,
        args: &[String],
    ) -> io::Result<u64>;
}

/// Default production stub — just logs. The real process_app::ProcessApp
/// spawning is driven from PlexiApp, not from HostModel, so this trait is
/// currently only exercised by tests.
pub struct LoggingSpawnService;

impl SpawnService for LoggingSpawnService {
    fn spawn(
        &mut self,
        app_id: &str,
        workspace_root: &Path,
        args: &[String],
    ) -> io::Result<u64> {
        log::debug!(
            "LoggingSpawnService::spawn app_id={app_id} root={} args={args:?}",
            workspace_root.display()
        );
        Ok(0)
    }
}

/// Tests only — records every call in-memory.
pub struct MockSpawnService {
    pub calls: Vec<(String, PathBuf, Vec<String>)>,
    pub next_pane_id: u64,
}

impl MockSpawnService {
    pub fn new() -> Self {
        Self { calls: Vec::new(), next_pane_id: 1 }
    }
}

impl Default for MockSpawnService {
    fn default() -> Self {
        Self::new()
    }
}

impl SpawnService for MockSpawnService {
    fn spawn(
        &mut self,
        app_id: &str,
        workspace_root: &Path,
        args: &[String],
    ) -> io::Result<u64> {
        self.calls
            .push((app_id.to_string(), workspace_root.to_path_buf(), args.to_vec()));
        let id = self.next_pane_id;
        self.next_pane_id += 1;
        Ok(id)
    }
}

// ── HostServices aggregate ─────────────────────────────────────────────────

pub struct HostServices {
    pub event_sink: Box<dyn EventSink>,
    pub fs: Box<dyn FsService>,
    pub secrets: Box<dyn SecretsService>,
    /// Arc rather than Box so every `ProcessApp` can hold a cheap clone and
    /// issue HTTP calls without going back through `HostServices`.
    pub net: std::sync::Arc<dyn NetService>,
    pub spawn: Box<dyn SpawnService>,
}

impl HostServices {
    /// Production wiring — real keychain, real filesystem, real event bus.
    /// `event_sink` appends to `<config_dir>/effects.jsonl` so every HostEffect
    /// leaves a durable audit trail. Uses a separate file from
    /// `events.jsonl` (written by `crate::event_log`) to avoid format drift
    /// between the host-level effect stream and the app-level event stream.
    pub fn new() -> Self {
        let effects_path = crate::config::config_dir().join("effects.jsonl");
        Self {
            event_sink: Box::new(FileEventSink::new(effects_path)),
            fs: Box::new(RealFsService),
            secrets: Box::new(KeychainSecretsService),
            net: std::sync::Arc::new(UreqNetService::new()),
            spawn: Box::new(LoggingSpawnService),
        }
    }

    /// Test wiring — every side effect is a mock the caller controls.
    pub fn mock() -> Self {
        Self {
            event_sink: Box::new(VecEventSink::new()),
            fs: Box::new(MockFsService::new()),
            secrets: Box::new(MockSecretsService::new()),
            net: std::sync::Arc::new(MockNetService::new()),
            spawn: Box::new(MockSpawnService::new()),
        }
    }
}

impl Default for HostServices {
    fn default() -> Self {
        Self::new()
    }
}

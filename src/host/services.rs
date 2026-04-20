//! HostServices — host side-effect plumbing.
//!
//! `EventSink` is the only production field right now; fs/secrets/net/spawn
//! trait seams live in `process_app` and `app_registry` until HostModel
//! owns their routing.

use crate::host::effect::HostEffect;
use std::collections::HashMap;

// ── EventSink ──────────────────────────────────────────────────────────────

pub trait EventSink: Send {
    fn emit(&mut self, effect: &HostEffect);
}

/// No-op sink — used in Layer-2 tests that don't care about event observation.
#[cfg(test)]
pub struct NoopEventSink;

#[cfg(test)]
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
#[cfg(test)]
pub struct VecEventSink {
    pub events: Vec<HostEffect>,
}

#[cfg(test)]
impl VecEventSink {
    pub fn new() -> Self {
        Self { events: Vec::new() }
    }
}

#[cfg(test)]
impl Default for VecEventSink {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
impl EventSink for VecEventSink {
    fn emit(&mut self, effect: &HostEffect) {
        self.events.push(effect.clone());
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
#[cfg(test)]
pub struct MockNetService {
    pub responses: HashMap<String, String>,
}

#[cfg(test)]
impl MockNetService {
    pub fn new() -> Self {
        Self { responses: HashMap::new() }
    }

    pub fn with(mut self, url: &str, body: &str) -> Self {
        self.responses.insert(url.to_string(), body.to_string());
        self
    }
}

#[cfg(test)]
impl Default for MockNetService {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
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

    /// Test wiring — accumulates effects for assertion.
    #[cfg(test)]
    pub fn mock() -> Self {
        Self {
            event_sink: Box::new(VecEventSink::new()),
        }
    }
}

impl Default for HostServices {
    fn default() -> Self {
        Self::new()
    }
}

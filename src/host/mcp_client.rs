//! Outbound MCP stdio transport (stint 0382).
//!
//! Plexi consumes external MCP servers through the app-connector plane: a
//! bridge app hosts the MCP client and re-exposes the server's tools via
//! `ExposeTools`. This module owns the half of that split the app runtime
//! cannot own — the **server process** — and nothing else.
//!
//! The split is deliberate:
//!
//! - **The host owns the process.** SDK apps run as CPython-in-Wasmtime with no
//!   ambient process access, so an app physically cannot spawn a server. More
//!   importantly it must not be able to: an app-supplied argv would be
//!   arbitrary native code execution wearing a capability's clothes.
//! - **The app owns the protocol.** This module never parses a JSON-RPC body,
//!   never knows what `tools/list` means, and never decides what a tool is. It
//!   moves newline-delimited JSON between a configured child process and the
//!   owning pane. MCP semantics — initialize, discovery, schema translation,
//!   call proxying — live in the bridge app.
//!
//! The containment property that makes the capability safe to grant:
//! **an app names a server by id; the host resolves the id to an argv from
//! user-owned config.** `mcp.client` therefore authorizes "talk to a server
//! the user configured", never "run this command". A compromised or malicious
//! bridge app cannot reach a binary the user did not already list.
//!
//! **The server is external and untrusted, so its transport cannot exist
//! without bounds.** Every `McpConnection` is constructed under [`McpLimits`].
//! The host bounds the child's lifetime and every transport queue or record it
//! retains; it does not sandbox the child or cap the child's own CPU/memory:
//!
//! - *Lifetime*: the child is spawned as its own process-group leader and
//!   teardown SIGKILLs the whole group, so `npx`/`uvx`-style launchers cannot
//!   leave descendant servers running. (A descendant that calls `setsid`
//!   escapes the group; closing that requires the 0554 seatbelt worker, which
//!   does not exist in-tree yet.)
//! - *Output volume*: stdout lines are capped at [`MAX_LINE_BYTES`] (overflow
//!   closes the connection), stderr lines at [`MAX_STDERR_LINE_BYTES`]
//!   (overflow truncates), and the pane-facing queue is created by
//!   [`inbound_channel`] with a fixed depth. The bound holds only because the
//!   consumer is bounded too: the pane's pump (`pump_mcp_inbound` in
//!   `wasm_python`) stops pulling while the guest's stdin queue is above its
//!   watermark, and the guest stdin itself enforces a hard cap — so a flooding
//!   server stalls in its own pipe instead of growing the host's heap at any
//!   hop. (The first shipped version bounded only the channel and drained it
//!   into an unbounded guest-stdin queue; a stdout-flooding server grew host
//!   RSS to 3 GiB and killed the host.)
//! - *Log volume*: stderr line **rate** is budgeted, not just line length —
//!   [`STDERR_LOG_LINES_PER_WINDOW`] logged lines per [`STDERR_LOG_WINDOW`],
//!   with one summary naming the suppressed count. Without it a
//!   stderr-flooding server converts its output 1:1 into `plexi.log` growth
//!   (measured ~11 MB/min) that date-based rotation never truncates mid-day.
//! - *Input volume*: writes go through a dedicated writer thread behind a
//!   fixed-depth queue. [`McpConnection::send`] never blocks the host thread;
//!   a server that stops reading stdin surfaces as a send error, which the
//!   pane converts into a close.
//! - *Responsiveness*: a server that produces no stdout at all within
//!   [`McpLimits::first_output_deadline`] is killed and reported closed. The
//!   protocol-level handshake deadline (initialize → tools/list) lives in the
//!   bridge app, which owns the protocol.
//! - *Delivery*: inbound lines are serviced from the host's **logic** pass
//!   (`service_python_pane_runtimes`), never only from paint, and every
//!   enqueued line fires the connection's [`McpWake`] so an idle or occluded
//!   host wakes to deliver it. Both halves are load-bearing: with paint-only
//!   draining, a healthy server's handshake reply sat undelivered until the
//!   bridge's own deadline killed the connection. When delivery stalls at the
//!   guest-stdin watermark, the pump parks the line with the explicit token
//!   returned by `AppendableStdin::arm_low_water_wake`. Repeated host passes
//!   carry that token instead of creating new logical armings. Guest
//!   consumption or close extracts the token and invokes its callback exactly
//!   once; supersession, if another producer deliberately installs fresh work,
//!   resolves the replaced token before returning the replacement. These are
//!   state-transition wakes, never timer polling, and with no line pending
//!   nothing is armed.
//!
//! Config lives at `<config_dir>/mcp_servers.toml`:
//!
//! ```toml
//! [servers.filesystem]
//! command = "npx"
//! args = ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]
//!
//! [servers.git]
//! command = "uvx"
//! args = ["mcp-server-git"]
//! env = { GIT_AUTHOR_NAME = "plexi" }
//! ```
//!
//! Note: this is the *outbound* direction. `PlexiEvent::McpToolCall` and
//! `AppRequest::McpToolResult` are the inbound direction — an external MCP
//! client calling into a Plexi app's `[app.mcp]` surface. The two never meet.
//!
//! **Runtime scope: CPython bridge apps only.** The app-facing messages
//! (`mcp_connect` / `mcp_send` / `mcp_disconnect` and the `mcp_connected` /
//! `mcp_message` / `mcp_closed` replies) are handled in `LivePythonPane`,
//! alongside `http_request` and `file_read`, and never reach the WIT `effect`
//! variant. Rust WASM component apps therefore cannot open an MCP connection.
//! Extending `wit/plexi.wit` changes the component type — the host rejects
//! every prebuilt component compiled against the old world ("expected variant
//! of N cases, found M") — so adding the WASM path means rebuilding the
//! CPython shim bundle and every `apps/wasm-poc/` fixture in the same change.
//! That is a separate unit of work, not a line in this one.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Deserialize;

/// Config file name under `config_dir()`. User-owned: Plexi never writes it.
pub const MCP_SERVERS_FILE: &str = "mcp_servers.toml";

/// Environment variables inherited by a server child. Everything else is
/// dropped — a server sees only these plus its own declared `env` table.
/// `PATH` and `HOME` are here because `npx`/`uvx`-style launchers cannot
/// resolve an interpreter without them, not because inheritance is the default.
const INHERITED_ENV: &[&str] = &["PATH", "HOME"];

/// Extra PATH entries prepended for GUI-launched hosts. A macOS app bundle
/// launched from Finder inherits a minimal `PATH` that omits the Homebrew and
/// `/usr/local` prefixes where `npx`/`uvx` actually live.
const PATH_FALLBACK: &str = "/usr/local/bin:/opt/homebrew/bin:/usr/bin:/bin";

/// Max bytes accepted on one stdout line from a server before the connection
/// is torn down. A single JSON-RPC message larger than this is a runaway
/// server, not a large legitimate response; truncating it would hand the app
/// a corrupt body, so the connection closes instead.
const MAX_LINE_BYTES: usize = 8 * 1024 * 1024;

/// Max bytes kept from one stderr line. stderr is diagnostics, not protocol,
/// so an oversized line is truncated (with a marker) and the rest discarded —
/// unlike stdout, nothing downstream parses it.
const MAX_STDERR_LINE_BYTES: usize = 8 * 1024;

/// Window for the stderr *logging* budget. Line length alone does not bound
/// what a server can write into `plexi.log` — a flooding server emitting
/// normal-length lines converts its output 1:1 into host log growth (measured
/// ~11 MB/min), and log rotation is date-based, so nothing truncates it
/// mid-day. Reading and `last_stderr` retention stay at full rate; only the
/// per-line log writes are budgeted.
const STDERR_LOG_WINDOW: Duration = Duration::from_secs(10);

/// Max stderr lines logged per [`STDERR_LOG_WINDOW`]. Beyond it, lines are
/// counted, and the next window opens with one summary line naming how many
/// were suppressed — loud about the flood without reproducing it.
const STDERR_LOG_LINES_PER_WINDOW: u32 = 20;

/// Depth of the pane-facing message queue created by [`inbound_channel`].
/// The stdout reader blocks when it is full, which in turn lets the pipe fill
/// and blocks the server itself. This bound is one link in a chain — server
/// pipe → this queue → the guest-stdin watermark and hard cap in
/// `wasm_python` — and only the whole chain makes host memory bounded:
/// worst case per connection is `INBOUND_QUEUE_LINES * MAX_LINE_BYTES` here
/// plus the guest-stdin cap, never unbounded.
const INBOUND_QUEUE_LINES: usize = 16;

/// Depth of the stdin writer queue. Outbound messages come from the app (tool
/// calls), so the queue protects against a *server* that stops reading, not
/// against the app: when it fills, `send` fails fast instead of blocking the
/// host thread on a pipe write.
const OUTBOUND_QUEUE_MESSAGES: usize = 64;

/// Bounds every connection is spawned under. [`McpConnection::spawn`] always
/// applies the defaults — there is no unbounded constructor.
#[derive(Debug, Clone)]
pub struct McpLimits {
    /// A server that writes nothing to stdout within this window after spawn
    /// is killed and reported closed. Generous because a cold `npx -y` may
    /// download the package before the server ever speaks; the bridge app's
    /// own handshake timer fires earlier for the interactive stuck-pane case.
    pub first_output_deadline: Duration,
}

impl Default for McpLimits {
    fn default() -> Self {
        Self {
            first_output_deadline: Duration::from_secs(120),
        }
    }
}

/// One inbound line, tagged with the server id it arrived from.
pub type InboundLine = (String, McpLine);

/// Sending half of the pane-facing feed. Cloned into every [`McpConnection`].
pub type InboundSender = SyncSender<InboundLine>;

/// Receiving half of the pane-facing feed. Drained from the host's logic pass.
pub type InboundReceiver = Receiver<InboundLine>;

/// The one sanctioned way to build the pane-facing feed for MCP connections.
/// Bounded by construction: hand the `InboundSender` to every [`McpConnection`]
/// a pane opens, drain the `InboundReceiver` from the host's *logic* pass.
pub fn inbound_channel() -> (InboundSender, InboundReceiver) {
    std::sync::mpsc::sync_channel(INBOUND_QUEUE_LINES)
}

/// Called by the stdout reader after each line lands in the inbound channel,
/// so the owning pane can wake the host's event loop. MCP data arrives on the
/// server's schedule, not the pane's paint schedule — without a wake, a line
/// enqueued while the host idles (or while the window is occluded) sits until
/// something unrelated triggers a pass. The hook must be cheap and reentrant;
/// it runs on the reader thread.
pub type McpWake = Arc<dyn Fn() + Send + Sync>;

/// Decision from [`StderrLogBudget::admit`] for one stderr line.
#[derive(Debug, PartialEq, Eq)]
enum StderrLogDecision {
    /// Within budget: log the line.
    Log,
    /// Budget exhausted for the current window: count it, write nothing.
    Suppress,
    /// A new window opened after suppression: log one summary naming the
    /// suppressed count, then log the line.
    SummaryThenLog { suppressed: u64 },
}

/// Windowed budget for stderr log writes — see [`STDERR_LOG_LINES_PER_WINDOW`].
struct StderrLogBudget {
    window_started: std::time::Instant,
    logged: u32,
    suppressed: u64,
}

impl StderrLogBudget {
    fn new(now: std::time::Instant) -> Self {
        Self {
            window_started: now,
            logged: 0,
            suppressed: 0,
        }
    }

    fn admit(&mut self, now: std::time::Instant) -> StderrLogDecision {
        if now.duration_since(self.window_started) >= STDERR_LOG_WINDOW {
            let suppressed = std::mem::take(&mut self.suppressed);
            self.window_started = now;
            self.logged = 1;
            return if suppressed > 0 {
                StderrLogDecision::SummaryThenLog { suppressed }
            } else {
                StderrLogDecision::Log
            };
        }
        if self.logged < STDERR_LOG_LINES_PER_WINDOW {
            self.logged += 1;
            StderrLogDecision::Log
        } else {
            self.suppressed += 1;
            StderrLogDecision::Suppress
        }
    }

    /// Suppressed lines never yet summarized — reported once at stream end so
    /// a flood that stops (or a server that exits mid-flood) still leaves an
    /// accurate final count.
    fn pending_suppressed(&self) -> u64 {
        self.suppressed
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum McpConfigError {
    #[error("mcp_servers.toml: {0}")]
    Parse(String),
    #[error("mcp_servers.toml: server '{0}' has an empty command")]
    EmptyCommand(String),
    #[error("no MCP server named '{0}' in mcp_servers.toml (configured: {1})")]
    UnknownServer(String, String),
}

/// One `[servers.<id>]` entry. `command` is required — a server with no
/// command is a config error, never a silently skipped entry.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct McpServerConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    /// Extra environment for the child, on top of `INHERITED_ENV`.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Working directory for the child. Defaults to the pane's workspace root.
    #[serde(default)]
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct McpServersFile {
    #[serde(default)]
    servers: BTreeMap<String, McpServerConfig>,
}

/// The user's configured MCP servers. The only source of argv for a spawn.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct McpServerRegistry {
    servers: BTreeMap<String, McpServerConfig>,
}

impl McpServerRegistry {
    /// Parse a `mcp_servers.toml` body. Every entry is validated eagerly so a
    /// typo surfaces at load time rather than on first connect.
    pub fn parse(body: &str) -> Result<Self, McpConfigError> {
        let file: McpServersFile =
            toml::from_str(body).map_err(|e| McpConfigError::Parse(e.to_string()))?;
        for (id, server) in &file.servers {
            if server.command.trim().is_empty() {
                return Err(McpConfigError::EmptyCommand(id.clone()));
            }
        }
        Ok(Self {
            servers: file.servers,
        })
    }

    /// Load from `<config_dir>/mcp_servers.toml`. An absent file means the user
    /// has configured no servers — that is a valid empty registry, and every
    /// `resolve` against it fails loudly by name. A file that exists but does
    /// not parse is an error, never an empty registry.
    pub fn load(config_dir: &Path) -> Result<Self, McpConfigError> {
        let path = config_dir.join(MCP_SERVERS_FILE);
        match std::fs::read_to_string(&path) {
            Ok(body) => {
                let registry = Self::parse(&body)?;
                log::info!(
                    "mcp_client: loaded {} server(s) from {}: {}",
                    registry.servers.len(),
                    path.display(),
                    registry.ids().join(", ")
                );
                Ok(registry)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                log::info!(
                    "mcp_client: no {} at {}; no MCP servers configured",
                    MCP_SERVERS_FILE,
                    path.display()
                );
                Ok(Self::default())
            }
            Err(e) => Err(McpConfigError::Parse(format!(
                "read {}: {e}",
                path.display()
            ))),
        }
    }

    /// Resolve a server id to its configured spawn. The error names every
    /// configured id so a misspelled server is diagnosable from the message.
    pub fn resolve(&self, server_id: &str) -> Result<&McpServerConfig, McpConfigError> {
        self.servers.get(server_id).ok_or_else(|| {
            let known: Vec<&str> = self.servers.keys().map(String::as_str).collect();
            McpConfigError::UnknownServer(
                server_id.to_string(),
                if known.is_empty() {
                    "none".to_string()
                } else {
                    known.join(", ")
                },
            )
        })
    }

    pub fn ids(&self) -> Vec<&str> {
        self.servers.keys().map(String::as_str).collect()
    }
}

/// What a reader thread reports back about one connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpLine {
    /// One newline-delimited JSON-RPC message from the server's stdout.
    Message(String),
    /// The connection ended. `reason` names why — clean EOF, oversized line,
    /// a missed deadline, or an IO failure. Emitted at most once per
    /// connection; delivery is best-effort when the pane receiver still
    /// exists.
    Closed(String),
}

/// Shared owner of the right to tear the child down. Whichever side fires
/// first — [`McpConnection`]'s `Drop` or the first-output watchdog — takes the
/// process-group id out under the lock, so the group is signalled at most once
/// and never after `Drop` could have reaped the leader (a reaped pid can be
/// recycled by the OS, and signalling a recycled group would kill a stranger).
struct Teardown {
    pgid: Mutex<Option<i32>>,
    /// A reason recorded by the watchdog before it kills, so the stdout
    /// reader's close notice names the deadline instead of a bare EOF.
    forced_reason: Mutex<Option<String>>,
}

impl Teardown {
    /// SIGKILL the child's whole process group, exactly once. The child was
    /// spawned as its own group leader, so `npx`/`uvx` descendants die with it.
    fn kill_group(&self) {
        self.with_pgid_once(|pgid| {
            #[cfg(unix)]
            unsafe {
                // Negative pid targets the process group. ESRCH (already gone)
                // is fine — the goal is "not running", not "we did the killing".
                libc::kill(-pgid, libc::SIGKILL);
            }
            #[cfg(not(unix))]
            let _ = pgid;
        });
    }

    fn with_pgid_once(&self, signal: impl FnOnce(i32)) {
        let Ok(mut slot) = self.pgid.lock() else {
            return;
        };
        let Some(pgid) = slot.take() else {
            return;
        };
        // Hold the mutex through kill(2), not merely through `take()`.
        // Otherwise Drop could observe None, reap the leader, and allow pgid
        // reuse before the watchdog issued its delayed signal.
        signal(pgid);
    }

    fn take_forced_reason(&self) -> Option<String> {
        self.forced_reason
            .lock()
            .ok()
            .and_then(|mut slot| slot.take())
    }
}

/// A live server process. Dropping it kills the child's whole process group:
/// an MCP server — and anything it spawned — must not outlive the pane that
/// opened it.
pub struct McpConnection {
    server_id: String,
    child: Child,
    /// Feed to the dedicated stdin writer thread. Taken on drop so the writer
    /// exits and the child sees EOF.
    outbound_tx: Option<SyncSender<String>>,
    teardown: Arc<Teardown>,
    /// Dropped with the connection so the first-output watchdog wakes and
    /// exits early instead of sleeping out its full deadline.
    _watchdog_cancel: Sender<()>,
    /// Last stderr line, kept for diagnostics — an MCP server that fails to
    /// start usually explains itself there and nowhere else.
    last_stderr: Arc<Mutex<String>>,
}

impl McpConnection {
    /// Spawn a configured server under [`McpLimits::default`] and start its
    /// I/O threads.
    ///
    /// `on_line` receives every stdout message and exactly one terminal
    /// `Closed`. Build it with [`inbound_channel`] so the feed is bounded.
    /// Sends are best-effort: a receiver dropped because the pane closed is
    /// normal, not an error.
    pub fn spawn(
        server_id: &str,
        config: &McpServerConfig,
        default_cwd: &Path,
        on_line: SyncSender<(String, McpLine)>,
        wake: McpWake,
    ) -> std::io::Result<Self> {
        Self::spawn_with_limits(
            server_id,
            config,
            default_cwd,
            on_line,
            wake,
            McpLimits::default(),
        )
    }

    fn spawn_with_limits(
        server_id: &str,
        config: &McpServerConfig,
        default_cwd: &Path,
        on_line: SyncSender<(String, McpLine)>,
        wake: McpWake,
        limits: McpLimits,
    ) -> std::io::Result<Self> {
        let cwd: PathBuf = config
            .cwd
            .as_ref()
            .map(PathBuf::from)
            .unwrap_or_else(|| default_cwd.to_path_buf());

        let mut command = Command::new(&config.command);
        command
            .args(&config.args)
            .current_dir(&cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env_clear();
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            // Own process group, so teardown's group SIGKILL reaches the
            // real server behind an `npx`/`uvx` launcher, not just the
            // launcher itself.
            command.process_group(0);
        }
        for key in INHERITED_ENV {
            if let Ok(value) = std::env::var(key) {
                let value = if *key == "PATH" {
                    format!("{PATH_FALLBACK}:{value}")
                } else {
                    value
                };
                command.env(key, value);
            } else if *key == "PATH" {
                command.env(key, PATH_FALLBACK);
            }
        }
        for (key, value) in &config.env {
            command.env(key, value);
        }

        log::info!(
            "mcp_client: spawning server_id={server_id} command={} args={:?} cwd={}",
            config.command,
            config.args,
            cwd.display()
        );
        let mut child = command.spawn().inspect_err(|e| {
            log::warn!(
                "mcp_client: spawn failed server_id={server_id} command={}: {e}",
                config.command
            );
        })?;

        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let teardown = Arc::new(Teardown {
            pgid: Mutex::new(if cfg!(unix) { Some(child.id() as i32) } else { None }),
            forced_reason: Mutex::new(None),
        });
        let last_stderr = Arc::new(Mutex::new(String::new()));
        let (outbound_tx, outbound_rx) =
            std::sync::mpsc::sync_channel::<String>(OUTBOUND_QUEUE_MESSAGES);
        let (watchdog_cancel_tx, watchdog_cancel_rx) = std::sync::mpsc::channel::<()>();

        // From here on, any early return drops `connection`, whose Drop kills
        // the process group — a half-wired child never outlives this function.
        let connection = Self {
            server_id: server_id.to_string(),
            child,
            outbound_tx: Some(outbound_tx),
            teardown: Arc::clone(&teardown),
            _watchdog_cancel: watchdog_cancel_tx.clone(),
            last_stderr: Arc::clone(&last_stderr),
        };

        if let Some(stderr) = stderr {
            let sink = Arc::clone(&last_stderr);
            let id = server_id.to_string();
            std::thread::Builder::new()
                .name(format!("mcp-stderr-{server_id}"))
                .spawn(move || {
                    let mut reader = BufReader::new(stderr);
                    let mut line = String::new();
                    let mut budget = StderrLogBudget::new(std::time::Instant::now());
                    // Every line still updates `last_stderr` (bounded by
                    // MAX_STDERR_LINE_BYTES); only log writes are budgeted.
                    let admit = |budget: &mut StderrLogBudget| match budget
                        .admit(std::time::Instant::now())
                    {
                        StderrLogDecision::Log => true,
                        StderrLogDecision::Suppress => false,
                        StderrLogDecision::SummaryThenLog { suppressed } => {
                            log::warn!(
                                "mcp_client: [{id}] stderr flooding: suppressed {suppressed} line(s) in the last {}s (budget {STDERR_LOG_LINES_PER_WINDOW} logged lines per window)",
                                STDERR_LOG_WINDOW.as_secs()
                            );
                            true
                        }
                    };
                    loop {
                        line.clear();
                        match read_bounded_line(&mut reader, &mut line, MAX_STDERR_LINE_BYTES) {
                            Ok(LineRead::Eof) => break,
                            Ok(LineRead::Line) => {
                                let trimmed = line.trim_end();
                                if trimmed.is_empty() {
                                    continue;
                                }
                                if admit(&mut budget) {
                                    log::info!("mcp_client: [{id}] stderr: {trimmed}");
                                }
                                if let Ok(mut slot) = sink.lock() {
                                    *slot = trimmed.to_string();
                                }
                            }
                            Ok(LineRead::Overflow) => {
                                if admit(&mut budget) {
                                    log::warn!(
                                        "mcp_client: [{id}] stderr line exceeded {MAX_STDERR_LINE_BYTES} bytes; truncated"
                                    );
                                }
                                if let Ok(mut slot) = sink.lock() {
                                    *slot = format!("{line}… [truncated]");
                                }
                                if !matches!(discard_line(&mut reader), Ok(false)) {
                                    break;
                                }
                            }
                            Err(_) => break,
                        }
                    }
                    let pending = budget.pending_suppressed();
                    if pending > 0 {
                        log::warn!(
                            "mcp_client: [{id}] stderr closed with {pending} unlogged line(s) suppressed by the rate limit"
                        );
                    }
                })
                .map_err(|e| std::io::Error::other(format!("mcp stderr thread: {e}")))?;
        }

        if let Some(stdin) = stdin {
            let sink = Arc::clone(&last_stderr);
            let id = server_id.to_string();
            std::thread::Builder::new()
                .name(format!("mcp-stdin-{server_id}"))
                .spawn(move || {
                    let mut stdin = stdin;
                    while let Ok(message) = outbound_rx.recv() {
                        let outcome = stdin
                            .write_all(message.as_bytes())
                            .and_then(|()| stdin.write_all(b"\n"))
                            .and_then(|()| stdin.flush());
                        if let Err(e) = outcome {
                            let detail = sink.lock().map(|s| s.clone()).unwrap_or_default();
                            if detail.is_empty() {
                                log::warn!("mcp_client: [{id}] write to server failed: {e}");
                            } else {
                                log::warn!(
                                    "mcp_client: [{id}] write to server failed: {e} (server stderr: {detail})"
                                );
                            }
                            return;
                        }
                    }
                    // Connection dropped its sender: exiting drops stdin,
                    // which sends the child EOF.
                })
                .map_err(|e| std::io::Error::other(format!("mcp stdin thread: {e}")))?;
        }

        if let Some(stdout) = stdout {
            let id = server_id.to_string();
            let teardown = Arc::clone(&teardown);
            std::thread::Builder::new()
                .name(format!("mcp-stdout-{server_id}"))
                .spawn(move || {
                    let mut reader = BufReader::new(stdout);
                    let mut line = String::new();
                    let mut watchdog_cancel = Some(watchdog_cancel_tx);
                    let reason = loop {
                        line.clear();
                        match read_bounded_line(&mut reader, &mut line, MAX_LINE_BYTES) {
                            Ok(LineRead::Eof) => break "server closed stdout".to_string(),
                            Ok(LineRead::Line) => {
                                if let Some(cancel) = watchdog_cancel.take() {
                                    let _ = cancel.send(());
                                }
                                let trimmed = line.trim();
                                if trimmed.is_empty() {
                                    continue;
                                }
                                // Blocking send on a bounded channel: a flood
                                // stalls here, then in the pipe, then in the
                                // server — never in host memory.
                                if on_line
                                    .send((id.clone(), McpLine::Message(trimmed.to_string())))
                                    .is_err()
                                {
                                    return;
                                }
                                wake();
                            }
                            Ok(LineRead::Overflow) => {
                                break format!("MCP message exceeds {MAX_LINE_BYTES} bytes");
                            }
                            Err(e) => break format!("read from server: {e}"),
                        }
                    };
                    let reason = teardown.take_forced_reason().unwrap_or(reason);
                    log::info!("mcp_client: [{id}] closed: {reason}");
                    let _ = on_line.send((id, McpLine::Closed(reason)));
                    wake();
                })
                .map_err(|e| std::io::Error::other(format!("mcp stdout thread: {e}")))?;
        }

        {
            let id = server_id.to_string();
            let teardown = Arc::clone(&teardown);
            let deadline = limits.first_output_deadline;
            std::thread::Builder::new()
                .name(format!("mcp-watchdog-{server_id}"))
                .spawn(move || {
                    // Woken early by the stdout reader on first output, or by
                    // the connection dropping. Only a genuine timeout kills.
                    if let Err(RecvTimeoutError::Timeout) =
                        watchdog_cancel_rx.recv_timeout(deadline)
                    {
                        log::warn!(
                            "mcp_client: [{id}] no output within {}s; killing server",
                            deadline.as_secs()
                        );
                        if let Ok(mut slot) = teardown.forced_reason.lock() {
                            slot.get_or_insert(format!(
                                "server produced no output within {}s",
                                deadline.as_secs()
                            ));
                        }
                        teardown.kill_group();
                    }
                })
                .map_err(|e| std::io::Error::other(format!("mcp watchdog thread: {e}")))?;
        }

        Ok(connection)
    }

    /// Queue one JSON-RPC message for the writer thread, as a single
    /// newline-delimited line. Never blocks the calling thread.
    ///
    /// The caller supplies already-serialized JSON. This asserts only that the
    /// framing holds — an embedded newline would split one message into two
    /// and silently corrupt the stream, so it is rejected rather than escaped.
    /// A full queue means the server has stopped reading stdin; that is a
    /// terminal fault for the caller to close on, not a condition to wait out.
    pub fn send(&self, message_json: &str) -> Result<(), String> {
        if message_json.contains('\n') {
            return Err("MCP message contains a newline; JSON-RPC framing is line-delimited and the message would split".to_string());
        }
        let Some(outbound) = &self.outbound_tx else {
            return Err("MCP server stdin is closed".to_string());
        };
        match outbound.try_send(message_json.to_string()) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err(format!(
                "MCP server '{}' is not reading stdin ({OUTBOUND_QUEUE_MESSAGES} messages already queued)",
                self.server_id
            )),
            Err(TrySendError::Disconnected(_)) => {
                let detail = self.last_stderr();
                if detail.is_empty() {
                    Err("write to MCP server failed; stdin writer exited".to_string())
                } else {
                    Err(format!(
                        "write to MCP server failed; stdin writer exited (server stderr: {detail})"
                    ))
                }
            }
        }
    }

    /// Last line the server wrote to stderr, or empty. The only diagnostic a
    /// server that dies during startup usually leaves behind.
    pub fn last_stderr(&self) -> String {
        self.last_stderr
            .lock()
            .map(|slot| slot.clone())
            .unwrap_or_default()
    }
}

impl Drop for McpConnection {
    fn drop(&mut self) {
        // Drop the writer feed first: the writer thread exits and the child
        // sees stdin EOF — the polite signal. The kill is not waited on it.
        self.outbound_tx.take();
        // Kill the whole group so launcher descendants die too, then the
        // direct kill as the non-unix / group-failure backstop, then reap.
        self.teardown.kill_group();
        if let Err(e) = self.child.kill() {
            if e.kind() != std::io::ErrorKind::InvalidInput {
                log::warn!("mcp_client: kill server_id={} failed: {e}", self.server_id);
            }
        }
        let _ = self.child.wait();
        log::info!("mcp_client: closed server_id={}", self.server_id);
    }
}

/// Outcome of one bounded line read.
#[derive(Debug, PartialEq, Eq)]
enum LineRead {
    /// A complete line (or the final unterminated line before EOF) is in `out`.
    Line,
    /// End of stream with nothing read.
    Eof,
    /// The line exceeded `max_bytes`. `out` holds the prefix read so far; the
    /// remainder of the line is still in the stream (see [`discard_line`]).
    Overflow,
}

/// `BufRead::read_line` with a hard ceiling, so a server that never emits a
/// newline cannot grow the host's heap without bound.
fn read_bounded_line<R: BufRead>(
    reader: &mut R,
    out: &mut String,
    max_bytes: usize,
) -> std::io::Result<LineRead> {
    let mut total = 0usize;
    loop {
        let available = match reader.fill_buf() {
            Ok(buf) => buf,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        if available.is_empty() {
            return Ok(if total == 0 { LineRead::Eof } else { LineRead::Line });
        }
        let (chunk, consumed, done) = match available.iter().position(|b| *b == b'\n') {
            Some(idx) => (&available[..idx], idx + 1, true),
            None => (available, available.len(), false),
        };
        if total + chunk.len() > max_bytes {
            return Ok(LineRead::Overflow);
        }
        out.push_str(&String::from_utf8_lossy(chunk));
        total += chunk.len();
        reader.consume(consumed);
        if done {
            return Ok(LineRead::Line);
        }
    }
}

/// Consume the stream up to and including the next newline. Returns `true` at
/// EOF. Companion to [`LineRead::Overflow`] for streams where truncation is
/// acceptable (stderr) rather than terminal (stdout).
fn discard_line<R: BufRead>(reader: &mut R) -> std::io::Result<bool> {
    loop {
        let available = match reader.fill_buf() {
            Ok(buf) => buf,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        if available.is_empty() {
            return Ok(true);
        }
        match available.iter().position(|b| *b == b'\n') {
            Some(idx) => {
                reader.consume(idx + 1);
                return Ok(false);
            }
            None => {
                let len = available.len();
                reader.consume(len);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_server_table() {
        let registry = McpServerRegistry::parse(
            r#"
            [servers.filesystem]
            command = "npx"
            args = ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]

            [servers.git]
            command = "uvx"
            args = ["mcp-server-git"]
            env = { GIT_AUTHOR_NAME = "plexi" }
            "#,
        )
        .expect("parse");
        assert_eq!(registry.ids(), vec!["filesystem", "git"]);
        let fs = registry.resolve("filesystem").expect("resolve");
        assert_eq!(fs.command, "npx");
        assert_eq!(fs.args.len(), 3);
        assert!(fs.env.is_empty());
        assert_eq!(
            registry.resolve("git").expect("resolve").env.get("GIT_AUTHOR_NAME"),
            Some(&"plexi".to_string())
        );
    }

    #[test]
    fn empty_registry_when_file_absent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let registry = McpServerRegistry::load(dir.path()).expect("load");
        assert!(registry.ids().is_empty());
    }

    #[test]
    fn unknown_server_names_the_configured_ids() {
        let registry = McpServerRegistry::parse(
            r#"
            [servers.filesystem]
            command = "npx"
            "#,
        )
        .expect("parse");
        let err = registry.resolve("fliesystem").expect_err("must reject");
        let message = err.to_string();
        assert!(message.contains("fliesystem"), "{message}");
        assert!(message.contains("filesystem"), "{message}");
    }

    #[test]
    fn unknown_server_against_empty_registry_says_none() {
        let registry = McpServerRegistry::default();
        let message = registry.resolve("anything").expect_err("must reject").to_string();
        assert!(message.contains("configured: none"), "{message}");
    }

    #[test]
    fn empty_command_is_a_config_error() {
        let err = McpServerRegistry::parse(
            r#"
            [servers.broken]
            command = "  "
            "#,
        )
        .expect_err("must reject");
        assert_eq!(err, McpConfigError::EmptyCommand("broken".to_string()));
    }

    #[test]
    fn unknown_key_is_a_config_error() {
        // A typo'd key must fail loudly. Silently ignoring `commnad` would
        // spawn nothing and report "empty command" from the wrong place.
        let err = McpServerRegistry::parse(
            r#"
            [servers.typo]
            command = "npx"
            arguments = ["oops"]
            "#,
        )
        .expect_err("must reject");
        assert!(
            matches!(err, McpConfigError::Parse(ref m) if m.contains("arguments")),
            "{err}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn group_signal_holds_take_once_lock_until_kill_returns() {
        let teardown = Arc::new(Teardown {
            pgid: Mutex::new(Some(123)),
            forced_reason: Mutex::new(None),
        });
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let worker_teardown = Arc::clone(&teardown);
        let worker = std::thread::spawn(move || {
            worker_teardown.with_pgid_once(|pgid| {
                entered_tx.send(pgid).expect("report critical section");
                release_rx.recv().expect("release simulated kill");
            });
        });

        assert_eq!(entered_rx.recv().expect("worker entered"), 123);
        assert!(
            matches!(
                teardown.pgid.try_lock(),
                Err(std::sync::TryLockError::WouldBlock)
            ),
            "competing teardown must not pass the lock before kill returns"
        );
        release_tx.send(()).expect("release worker");
        worker.join().expect("worker");
        assert!(teardown.pgid.lock().expect("pgid mutex").is_none());
    }

    /// A replay server that speaks real captured MCP traffic. The response
    /// bodies in `testdata/mcp/` were captured from real servers
    /// (`@modelcontextprotocol/server-filesystem`), never hand-written, so the
    /// transport is exercised against byte shapes a real server produces.
    fn replay_script(dir: &Path, responses: &[&str]) -> PathBuf {
        let path = dir.join("replay.sh");
        let mut script = String::from("#!/bin/sh\ni=0\nwhile IFS= read -r line; do\n  i=$((i+1))\n");
        for (idx, response) in responses.iter().enumerate() {
            script.push_str(&format!(
                "  if [ \"$i\" = \"{}\" ]; then printf '%s\\n' '{}'; fi\n",
                idx + 1,
                response.replace('\'', "'\\''")
            ));
        }
        script.push_str("done\n");
        write_script(&path, &script);
        path
    }

    fn write_script(path: &Path, body: &str) {
        std::fs::write(path, body).expect("write script");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
                .expect("chmod script");
        }
    }

    fn noop_wake() -> McpWake {
        Arc::new(|| {})
    }

    fn script_config(path: &Path) -> McpServerConfig {
        McpServerConfig {
            command: path.display().to_string(),
            args: Vec::new(),
            env: BTreeMap::new(),
            cwd: None,
        }
    }

    fn captured(name: &str) -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/host/testdata/mcp")
            .join(name);
        std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read fixture {}: {e}", path.display()))
            .trim()
            .to_string()
    }

    #[test]
    fn round_trips_real_captured_server_traffic() {
        let dir = tempfile::tempdir().expect("tempdir");
        let initialize = captured("filesystem_initialize.json");
        let tools_list = captured("filesystem_tools_list.json");
        let script = replay_script(dir.path(), &[&initialize, &tools_list]);

        let (tx, rx) = inbound_channel();
        let conn = McpConnection::spawn("filesystem", &script_config(&script), dir.path(), tx, noop_wake())
            .expect("spawn replay");

        conn.send(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#)
            .expect("send initialize");
        let (id, line) = rx
            .recv_timeout(Duration::from_secs(10))
            .expect("initialize reply");
        assert_eq!(id, "filesystem");
        let McpLine::Message(body) = line else {
            panic!("expected a message, got {line:?}");
        };
        let parsed: serde_json::Value = serde_json::from_str(&body).expect("valid JSON");
        assert_eq!(parsed["result"]["serverInfo"]["name"], "secure-filesystem-server");

        conn.send(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#)
            .expect("send tools/list");
        let (_, line) = rx
            .recv_timeout(Duration::from_secs(10))
            .expect("tools/list reply");
        let McpLine::Message(body) = line else {
            panic!("expected a message, got {line:?}");
        };
        let parsed: serde_json::Value = serde_json::from_str(&body).expect("valid JSON");
        let tools = parsed["result"]["tools"].as_array().expect("tools array");
        assert!(
            tools.iter().any(|t| t["name"] == "read_text_file"),
            "captured fixture must carry real tools"
        );
    }

    #[test]
    fn reports_close_when_the_server_exits() {
        let dir = tempfile::tempdir().expect("tempdir");
        let script = replay_script(dir.path(), &[]);
        let (tx, rx) = inbound_channel();
        let conn = McpConnection::spawn("dead", &script_config(&script), dir.path(), tx, noop_wake())
            .expect("spawn");
        drop(conn);
        let (id, line) = rx
            .recv_timeout(Duration::from_secs(10))
            .expect("close notice");
        assert_eq!(id, "dead");
        assert!(matches!(line, McpLine::Closed(_)), "{line:?}");
    }

    #[test]
    fn rejects_a_message_carrying_a_newline() {
        let dir = tempfile::tempdir().expect("tempdir");
        let script = replay_script(dir.path(), &[]);
        let (tx, _rx) = inbound_channel();
        let conn = McpConnection::spawn("framing", &script_config(&script), dir.path(), tx, noop_wake())
            .expect("spawn");
        let err = conn
            .send("{\"jsonrpc\":\"2.0\",\n\"id\":1}")
            .expect_err("must reject");
        assert!(err.contains("newline"), "{err}");
    }

    #[test]
    fn bounded_line_read_reports_an_unterminated_flood_as_overflow() {
        let flood = vec![b'x'; MAX_LINE_BYTES + 1];
        let mut reader = BufReader::new(&flood[..]);
        let mut out = String::new();
        let read = read_bounded_line(&mut reader, &mut out, MAX_LINE_BYTES).expect("read");
        assert_eq!(read, LineRead::Overflow);
    }

    #[test]
    fn discard_line_skips_to_the_next_line_after_an_overflow() {
        let data = b"aaaaaaaaaa\nsecond\n";
        let mut reader = BufReader::new(&data[..]);
        let mut out = String::new();
        assert_eq!(
            read_bounded_line(&mut reader, &mut out, 4).expect("read"),
            LineRead::Overflow
        );
        assert!(!discard_line(&mut reader).expect("discard"), "not EOF yet");
        out.clear();
        assert_eq!(
            read_bounded_line(&mut reader, &mut out, 64).expect("read"),
            LineRead::Line
        );
        assert_eq!(out, "second");
    }

    #[test]
    fn spawn_failure_surfaces_the_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = McpServerConfig {
            command: "/nonexistent/plexi-mcp-does-not-exist".to_string(),
            args: Vec::new(),
            env: BTreeMap::new(),
            cwd: None,
        };
        let (tx, _rx) = inbound_channel();
        let Err(err) = McpConnection::spawn("missing", &config, dir.path(), tx, noop_wake()) else {
            panic!("spawning a nonexistent command must fail");
        };
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    }

    /// The finding this guards: `npx`-style launchers spawn the real server as
    /// a descendant, and killing only the launcher orphaned it. The launcher
    /// script here backgrounds a `sleep` (the stand-in server), reports its
    /// pid over stdout, and keeps running; dropping the connection must take
    /// the descendant down with the launcher.
    #[cfg(unix)]
    #[test]
    fn drop_kills_the_whole_process_group() {
        let dir = tempfile::tempdir().expect("tempdir");
        let script = dir.path().join("launcher.sh");
        write_script(
            &script,
            "#!/bin/sh\nsleep 300 &\nprintf '{\"descendant_pid\":%s}\\n' \"$!\"\nexec sleep 300\n",
        );
        let (tx, rx) = inbound_channel();
        let conn = McpConnection::spawn("launcher", &script_config(&script), dir.path(), tx, noop_wake())
            .expect("spawn");
        let (_, line) = rx
            .recv_timeout(Duration::from_secs(10))
            .expect("descendant pid");
        let McpLine::Message(body) = line else {
            panic!("expected a message, got {line:?}");
        };
        let parsed: serde_json::Value = serde_json::from_str(&body).expect("valid JSON");
        let pid = parsed["descendant_pid"].as_str().map(str::to_string).unwrap_or_else(|| parsed["descendant_pid"].to_string());
        let pid: libc::pid_t = pid.trim().parse().expect("numeric pid");
        assert_eq!(unsafe { libc::kill(pid, 0) }, 0, "descendant must be alive");

        drop(conn);

        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            // ESRCH once the group SIGKILL lands. A zombie would still be
            // signal-able, but the descendant's parent (the launcher) is
            // killed and reaped, so init reaps the descendant promptly.
            if unsafe { libc::kill(pid, 0) } == -1 {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "descendant pid {pid} survived the process-group kill"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// A server that never reads stdin must surface as a fast send error, not
    /// a blocked host thread: the writer queue fills, and `send` fails.
    #[test]
    fn send_fails_fast_when_the_server_stops_reading_stdin() {
        let dir = tempfile::tempdir().expect("tempdir");
        let script = dir.path().join("deaf.sh");
        write_script(&script, "#!/bin/sh\nexec sleep 300\n");
        let (tx, _rx) = inbound_channel();
        let conn = McpConnection::spawn("deaf", &script_config(&script), dir.path(), tx, noop_wake())
            .expect("spawn");

        // 8 KiB per message: the pipe (~64 KiB) plus the writer queue absorb
        // a bounded amount, then try_send must report Full. Far below the
        // cap this loop would block forever if send ever blocked.
        let message = format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"pad","params":{{"pad":"{}"}}}}"#,
            "x".repeat(8 * 1024)
        );
        let started = std::time::Instant::now();
        let mut refused = None;
        for _ in 0..OUTBOUND_QUEUE_MESSAGES + 64 {
            if let Err(err) = conn.send(&message) {
                refused = Some(err);
                break;
            }
        }
        let err = refused.expect("a non-reading server must eventually refuse sends");
        assert!(err.contains("not reading stdin"), "{err}");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "send path must never block the caller"
        );
    }

    /// A server that produces no stdout at all is killed at the first-output
    /// deadline and reported closed with a reason naming the deadline.
    #[test]
    fn silent_server_is_closed_at_the_first_output_deadline() {
        let dir = tempfile::tempdir().expect("tempdir");
        let script = dir.path().join("silent.sh");
        write_script(&script, "#!/bin/sh\nexec sleep 300\n");
        let (tx, rx) = inbound_channel();
        let conn = McpConnection::spawn_with_limits(
            "silent",
            &script_config(&script),
            dir.path(),
            tx,
            noop_wake(),
            McpLimits {
                first_output_deadline: Duration::from_millis(200),
            },
        )
        .expect("spawn");

        let (id, line) = rx
            .recv_timeout(Duration::from_secs(10))
            .expect("deadline close");
        assert_eq!(id, "silent");
        let McpLine::Closed(reason) = line else {
            panic!("expected Closed, got {line:?}");
        };
        assert!(reason.contains("no output within"), "{reason}");
        drop(conn);
    }

    /// A server that speaks promptly must NOT be killed by the watchdog even
    /// long after the deadline has elapsed.
    #[test]
    fn responsive_server_outlives_the_first_output_deadline() {
        let dir = tempfile::tempdir().expect("tempdir");
        let initialize = captured("filesystem_initialize.json");
        let script = replay_script(dir.path(), &[&initialize]);
        let (tx, rx) = inbound_channel();
        let conn = McpConnection::spawn_with_limits(
            "prompt",
            &script_config(&script),
            dir.path(),
            tx,
            noop_wake(),
            McpLimits {
                first_output_deadline: Duration::from_millis(200),
            },
        )
        .expect("spawn");
        conn.send(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#)
            .expect("send");
        let (_, line) = rx.recv_timeout(Duration::from_secs(10)).expect("reply");
        assert!(matches!(line, McpLine::Message(_)), "{line:?}");

        // Well past the deadline: the connection must still be alive — no
        // Closed may arrive while the server sits idle awaiting input.
        std::thread::sleep(Duration::from_millis(600));
        assert!(
            rx.try_recv().is_err(),
            "watchdog must stand down once the server has spoken"
        );
        drop(conn);
    }

    /// The stderr log budget: within-budget lines log, over-budget lines are
    /// counted silently, and the next window opens with exactly one summary.
    /// This is the disk-exhaustion bound — a flooding server must not convert
    /// its stderr 1:1 into plexi.log growth.
    #[test]
    fn stderr_log_budget_suppresses_and_summarizes() {
        let start = std::time::Instant::now();
        let mut budget = StderrLogBudget::new(start);
        for _ in 0..STDERR_LOG_LINES_PER_WINDOW {
            assert_eq!(budget.admit(start), StderrLogDecision::Log);
        }
        for _ in 0..1000 {
            assert_eq!(budget.admit(start), StderrLogDecision::Suppress);
        }
        assert_eq!(budget.pending_suppressed(), 1000);

        // New window: one summary carrying the suppressed count, then the
        // budget refills.
        let later = start + STDERR_LOG_WINDOW;
        assert_eq!(
            budget.admit(later),
            StderrLogDecision::SummaryThenLog { suppressed: 1000 }
        );
        assert_eq!(budget.pending_suppressed(), 0);
        for _ in 1..STDERR_LOG_LINES_PER_WINDOW {
            assert_eq!(budget.admit(later), StderrLogDecision::Log);
        }
        assert_eq!(budget.admit(later), StderrLogDecision::Suppress);
    }

    /// A quiet stream never pays for windows it did not flood: rollover with
    /// nothing suppressed is a plain `Log`, not a summary.
    #[test]
    fn stderr_log_budget_quiet_rollover_has_no_summary() {
        let start = std::time::Instant::now();
        let mut budget = StderrLogBudget::new(start);
        assert_eq!(budget.admit(start), StderrLogDecision::Log);
        let later = start + STDERR_LOG_WINDOW * 3;
        assert_eq!(budget.admit(later), StderrLogDecision::Log);
        assert_eq!(budget.pending_suppressed(), 0);
    }

    /// A stderr flood is truncated and the connection keeps working — stderr
    /// is diagnostics, never a lever to exhaust host memory or kill the link.
    #[test]
    fn stderr_flood_is_truncated_and_the_connection_survives() {
        let dir = tempfile::tempdir().expect("tempdir");
        let initialize = captured("filesystem_initialize.json");
        let script = dir.path().join("noisy.sh");
        // One giant unterminated-until-late stderr line, then a normal reply
        // path on stdin/stdout.
        write_script(
            &script,
            &format!(
                "#!/bin/sh\nhead -c 100000 /dev/zero | tr '\\0' 'e' 1>&2\necho 1>&2\nwhile IFS= read -r line; do\n  printf '%s\\n' '{}'\ndone\n",
                initialize.replace('\'', "'\\''")
            ),
        );
        let (tx, rx) = inbound_channel();
        let conn = McpConnection::spawn("noisy", &script_config(&script), dir.path(), tx, noop_wake())
            .expect("spawn");
        conn.send(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#)
            .expect("send");
        let (_, line) = rx.recv_timeout(Duration::from_secs(10)).expect("reply");
        assert!(matches!(line, McpLine::Message(_)), "{line:?}");
        let kept = conn.last_stderr();
        assert!(
            kept.len() <= MAX_STDERR_LINE_BYTES + "… [truncated]".len(),
            "stderr retention must be bounded, kept {} bytes",
            kept.len()
        );
    }
}

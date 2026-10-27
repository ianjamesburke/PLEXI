//! `plexi host start/stop/status` — CLI-driven host launch with declarative
//! boot state (stint 0342).
//!
//! Unlike every other command in `src/cli/`, `host start/stop/status` run
//! *outside* a Plexi pane by definition — there is no running instance yet
//! to own `PLEXI_SOCKET`. All three talk to the channel's `notify.sock`
//! directly (same socket `src/app/mod.rs` binds and `nudge_running_instance`
//! connects to), never through the `PLEXI_SOCKET` env-var path used by every
//! in-pane CLI command.
//!
//! `host start` seeds panes via the same spawn-queue directory `plexi open`
//! and `plexi pane new` use outside a pane (`host_config_dir(channel).join("spawn-queue")`),
//! so no new host-side boot protocol is introduced — `drain_spawn_queue()`
//! picks the seeded panes up on the launched app's first frame.
//!
//! All channel-scoped paths here go through `host_config_dir(channel)`, not
//! `crate::config::config_dir()` — the latter lets the bare `plexi` binary
//! adopt an inherited `PLEXI_CHANNEL`, which would let `host start/stop/status`
//! silently target the wrong channel's profile when invoked from inside a pane.

#[cfg(windows)]
mod windows_launch;

use serde::Deserialize;
use std::io::Write;
use crate::platform::ipc::{self, IpcStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// One declarative pane to seed on `host start`, either parsed from a
/// `--pane` flag (`parse_pane_flag`) or a `[[pane]]` table in a `--layout`
/// TOML file (`LayoutFile`).
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct PaneSpec {
    pub cwd: Option<String>,
    pub cmd: Option<String>,
    pub placement: Option<String>,
}

/// `--layout <file.toml>` wrapper: one or more `[[pane]]` tables.
///
/// Not to be confused with the `[[steps]]` scene format under `src/testing/`
/// — that is an unrelated format for the in-process headless test harness.
#[derive(Debug, Deserialize)]
pub struct LayoutFile {
    #[serde(default)]
    pub pane: Vec<PaneSpec>,
}

/// Parse a `--pane 'cwd=<dir>[,cmd=<command>][,tab|window]'` flag.
///
/// Comma-separated `key=value` pairs (`cwd`, `cmd`) plus bare `tab`/`window`
/// placement tokens. `cwd` is required. Omitting a placement token maps to a
/// plain split (`"split_h"`) at seed time — see `placement_to_layout`.
///
/// Known limitation: this format has no escaping, so a `cmd` value that
/// itself contains a comma will be split incorrectly. Accepted as a
/// consequence of the flag format specified for this task; commands needing
/// commas should go through `--layout` instead, where TOML string values are
/// unambiguous.
pub fn parse_pane_flag(s: &str) -> Result<PaneSpec, String> {
    let mut spec = PaneSpec::default();
    let mut have_cwd = false;
    for segment in s.split(',') {
        let segment = segment.trim();
        if segment.is_empty() {
            return Err(format!("malformed --pane spec {s:?}: empty segment"));
        }
        if let Some((key, value)) = segment.split_once('=') {
            match key.trim() {
                "cwd" => {
                    if have_cwd {
                        return Err(format!("malformed --pane spec {s:?}: duplicate cwd"));
                    }
                    spec.cwd = Some(value.to_string());
                    have_cwd = true;
                }
                "cmd" => {
                    if spec.cmd.is_some() {
                        return Err(format!("malformed --pane spec {s:?}: duplicate cmd"));
                    }
                    spec.cmd = Some(value.to_string());
                }
                other => {
                    return Err(format!(
                        "malformed --pane spec {s:?}: unknown key {other:?} (expected cwd or cmd)"
                    ));
                }
            }
        } else {
            match segment {
                "tab" | "window" => {
                    if spec.placement.is_some() {
                        return Err(format!(
                            "malformed --pane spec {s:?}: duplicate placement token"
                        ));
                    }
                    spec.placement = Some(segment.to_string());
                }
                other => {
                    return Err(format!(
                        "malformed --pane spec {s:?}: unrecognized token {other:?} \
                         (expected key=value or tab/window)"
                    ));
                }
            }
        }
    }
    if !have_cwd {
        return Err(format!(
            "malformed --pane spec {s:?}: cwd=<dir> is required"
        ));
    }
    Ok(spec)
}

/// Maps `PaneSpec.placement` to the `"layout"` field values already
/// understood by `drain_spawn_queue`/`PaneLaunchSpec::from_spawn_pane`.
fn placement_to_layout(spec: &PaneSpec) -> Result<&'static str, String> {
    match spec.placement.as_deref() {
        None => Ok("split_h"),
        Some("tab") => Ok("tab"),
        Some("window") => Ok("new_window"),
        Some(other) => Err(format!(
            "invalid pane placement {other:?}: expected \"tab\" or \"window\" (omit for default split)"
        )),
    }
}

/// The pane/context-scoped `PLEXI_*` vars that must never leak into a
/// `host start`-launched process — reading them to decide what to launch
/// would reintroduce the exact channel-leak trap this command exists to
/// eliminate (see root CLAUDE.md Traps: "`PLEXI_CHANNEL` leaks into app
/// tooling"). `PLEXI_CHANNEL` itself is stripped unconditionally too, then
/// re-added explicitly from the resolved channel — never inherited.
const STRIP_VARS: &[&str] = &[
    "PLEXI_SOCKET",
    "PLEXI_PANE_ID",
    crate::host::process_cleanup::SESSION_ENV,
    "PLEXI_CONTEXT_ID",
    "PLEXI_CONTEXT_NAME",
    "PLEXI_CONTEXT_DESCRIPTION",
    "PLEXI_CONTEXT_ROOT",
    "PLEXI_CONTEXT_DEPTH",
    "PLEXI_RUNNING",
    "PLEXI_CALL_CREDENTIAL",
    "PLEXI_AGENT_SPAWNED",
    "PLEXI_AGENT_HARNESS",
    "PLEXI_AGENT_ROLE",
    "PLEXI_AGENT_ID",
    "PLEXI_PARENT_PANE",
    "PLEXI_EPHEMERAL",
    "PLEXI_RESUME_COMMAND",
    "PLEXI_HOST_MCP_PORT",
    "PLEXI_HOST_MCP_TOKEN",
    "PLEXI_CHANNEL",
    // Never inherited: a pane inside an ephemeral host must not silently
    // launch further ephemeral hosts. Re-added only for an explicit
    // `host start --ephemeral`.
    crate::workspace::EPHEMERAL_SESSION_ENV,
    // Background mode is also an explicit launch choice, never ambient state.
    crate::workspace::BACKGROUND_SESSION_ENV,
];

/// Pure env-filter used to build the launched child's environment. Takes the
/// parent env as a plain `Vec` (rather than reading `std::env::vars()`
/// directly) so it is deterministically testable without mutating real
/// process env.
pub(crate) fn filter_child_env(
    parent_env: Vec<(String, String)>,
    channel: Option<&str>,
) -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> = parent_env
        .into_iter()
        .filter(|(k, _)| !STRIP_VARS.contains(&k.as_str()))
        .collect();
    if let Some(c) = channel {
        env.push(("PLEXI_CHANNEL".to_string(), c.to_string()));
    }
    env
}

/// Constructs launch env from explicit boot flags after stripping all
/// inherited Plexi routing and boot-mode state.
pub(crate) fn host_child_env(
    parent_env: Vec<(String, String)>,
    channel: Option<&str>,
    ephemeral: bool,
    background: bool,
) -> Vec<(String, String)> {
    let mut env = filter_child_env(parent_env, channel);
    if ephemeral {
        env.push((
            crate::workspace::EPHEMERAL_SESSION_ENV.to_string(),
            "1".to_string(),
        ));
    }
    if background {
        env.push((
            crate::workspace::BACKGROUND_SESSION_ENV.to_string(),
            "1".to_string(),
        ));
    }
    env
}

/// The channel-scoped profile dir (`~/.plexi<-suffix>`) for `channel`, e.g.
/// `None` → `~/.plexi`, `Some("alpha")` → `~/.plexi-alpha`.
///
/// Deliberately does **not** call `crate::config::config_dir()`, which
/// lets the bare `plexi` binary adopt `PLEXI_CHANNEL` from the environment
/// when its own name carries no channel. `host start/stop/status` can be
/// invoked from inside a pane
/// (e.g. an agent driving a nested PR-channel test), where the calling
/// pane's `PLEXI_CHANNEL` would silently redirect every socket/spawn-queue
/// path to the wrong channel's profile — the exact leak trap this command
/// exists to eliminate (see root CLAUDE.md Traps: "`PLEXI_CHANNEL` leaks
/// into app tooling"). This function always derives the profile dir from
/// `channel`, which callers resolve once via `resolve_channel_paths()`
/// (binary-basename-only, ignoring env).
fn host_config_dir(channel: Option<&str>) -> PathBuf {
    let suffix = channel.map(|c| format!("-{c}")).unwrap_or_default();
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(format!(".plexi{suffix}"))
}

/// Resolve the canonical installed target, falling back only to this executable
/// for an unpacked package or development build.
fn resolve_channel_paths() -> (Option<String>, PathBuf, PathBuf) {
    let channel = crate::config::build_channel();
    let binary = match crate::distribution::installed() {
        Ok(Some(receipt)) => receipt.active.executable(),
        Ok(None) => std::env::current_exe().unwrap_or_default(),
        Err(error) => {
            log::error!("host: installation record failed: {error}");
            PathBuf::new()
        }
    };
    let bundle = binary.ancestors().find(|p| p.extension().is_some_and(|e| e == "app"))
        .map(Path::to_path_buf).unwrap_or_else(|| binary.clone());
    log::info!("host: resolved channel={channel:?} bundle={bundle:?} binary={binary:?}");
    (channel, bundle, binary)
}

/// Best-effort pid lookup for the process holding `notify.sock` open. No
/// pid-file mechanism exists anywhere in this codebase (confirmed: `src/app/`
/// and `src/config/` have no precedent), so this shells out to `lsof -t` as a
/// diagnostic only — failure here never blocks the caller, it just means pid
/// is reported as unknown.
#[cfg(windows)]
fn detect_pid(socket_path: &Path) -> Option<u32> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::Pipes::GetNamedPipeServerProcessId;
    let stream = ipc::connect_timeout(socket_path, Duration::from_millis(100)).ok()?;
    let mut pid = 0;
    // SAFETY: a live client pipe handle and a writable PID out-parameter.
    if unsafe { GetNamedPipeServerProcessId(stream.as_raw_handle().cast(), &mut pid) } == 0 {
        log::warn!("host: pipe owner lookup failed for {socket_path:?}: {}", std::io::Error::last_os_error());
        None
    } else {
        Some(pid)
    }
}

#[cfg(unix)]
fn detect_pid(socket_path: &Path) -> Option<u32> {
    // `lsof` is not installed by default on a minimal Linux, and without a pid
    // `host stop`'s SIGTERM fallback has nothing to signal. /proc answers the
    // same question from the kernel with no external dependency, so try it
    // first and keep `lsof` as the fallback.
    #[cfg(target_os = "linux")]
    if let Some(pid) = detect_pid_via_proc(socket_path) {
        return Some(pid);
    }
    match std::process::Command::new("lsof")
        .arg("-t")
        .arg(socket_path)
        .output()
    {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout)
            .lines()
            .next()
            .and_then(|l| l.trim().parse::<u32>().ok()),
        Ok(out) => {
            log::debug!(
                "host: lsof found no owner for {socket_path:?} (status {:?})",
                out.status
            );
            None
        }
        Err(e) => {
            log::warn!("host: lsof unavailable ({e}) — pid unknown for {socket_path:?}");
            None
        }
    }
}

enum RunningState {
    Running,
    NotRunning,
}

/// Probe whether a host for this channel is already up. Reuses the same
/// stale-socket cleanup idiom as `src/cli/mod.rs`'s `send_to_socket`:
/// `ConnectionRefused` means the socket file is stale (owning process died
/// without cleanup) — remove it and treat as not-running.
fn probe_notify_socket(socket_path: &Path, timeout: Duration) -> RunningState {
    // An empty transport frame only probes the connection. It uses the same
    // bounded connect path as commands, including a saturated Unix backlog.
    match super::send_line_to_socket(socket_path, b"", timeout) {
        Ok(_) | Err(super::SocketTransportError::ConnectTimeout) => RunningState::Running,
        Err(super::SocketTransportError::Connect(e))
            if e.kind() == std::io::ErrorKind::ConnectionRefused => {
                log::info!("host: removing stale endpoint {socket_path:?}");
                ipc::remove_stale_endpoint(socket_path);
                RunningState::NotRunning
            }
        Err(e) => {
            log::info!("host: endpoint {socket_path:?} not reachable: {e:?}");
            RunningState::NotRunning
        }
    }
}

/// Readiness is an application-dispatched ListPanes round trip, not just an
/// open pipe. Every phase spends the same deadline; late replies cannot turn
/// a timed-out startup into success.
fn query_ready_status(socket_path: &Path, profile_dir: &Path, deadline: Instant) -> Option<usize> {
    if Instant::now() >= deadline {
        return None;
    }
    let response_file = crate::rpc::response_file_in(profile_dir, "host-status", "json");
    let request = crate::protocol::AppRequest::ListPanes {
        response_file: response_file.clone(),
        context_id: None,
    };
    let line = format!("{}\n", serde_json::to_string(&request).ok()?);
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return None;
    }
    if let Err(e) = super::send_line_to_socket(socket_path, line.as_bytes(), remaining) {
        log::debug!("host: readiness transport failed for {socket_path:?}: {e:?}");
        return None;
    }
    let remaining = deadline.saturating_duration_since(Instant::now());
    let content = match crate::rpc::poll_string(&response_file, Some(remaining)) {
        Ok(content) => content,
        Err(e) => {
            log::debug!("host: readiness response failed for {socket_path:?}: {e}");
            return None;
        }
    };
    if Instant::now() >= deadline {
        return None;
    }
    serde_json::from_str::<Vec<serde_json::Value>>(&content).ok().map(|v| v.len())
}

fn wait_for_ready(socket_path: &Path, timeout: Duration, profile_dir: &Path) -> Result<usize, String> {
    let start = Instant::now();
    let deadline = start + timeout;
    log::info!("host_start: readiness poll start socket={socket_path:?} timeout={timeout:?}");
    while Instant::now() < deadline {
        let probe_deadline = deadline.min(Instant::now() + crate::rpc::DEFAULT_TIMEOUT);
        if let Some(count) = query_ready_status(socket_path, profile_dir, probe_deadline) {
            log::info!("host_start: ready in {:?}, pane_count={count}", start.elapsed());
            return Ok(count);
        }
        std::thread::sleep(Duration::from_millis(200).min(deadline.saturating_duration_since(Instant::now())));
    }
    log::error!("host_start: readiness timeout after {:?}", start.elapsed());
    Err(format!("timed out after {timeout:?} waiting for Plexi host to become ready"))
}

/// Write one spawn-queue JSON file for `spec`, matching the schema
/// `open.rs`'s fallback (no-`PLEXI_SOCKET`) spawn path writes and
/// `drain_spawn_queue` reads: `type_id`, `args`, `layout`, `cwd`.
fn seed_pane(queue_dir: &Path, index: usize, spec: &PaneSpec) -> Result<PathBuf, String> {
    let layout = placement_to_layout(spec)?;
    let args: Vec<String> = spec.cmd.clone().into_iter().collect();
    let payload = serde_json::json!({
        "type_id": "terminal",
        "args": args,
        "layout": layout,
        "cwd": spec.cwd,
        "origin": "host start",
        "queued_at_ms": crate::cli::spawn_queued_at_ms(),
    });
    let id = crate::platform::clock::now_nanos();
    let file = queue_dir.join(format!("{id}-{index}.json"));
    std::fs::write(&file, payload.to_string())
        .map_err(|e| format!("could not write spawn-queue file {file:?}: {e}"))?;
    log::info!(
        "host_start: seeded pane index={index} type_id=terminal cwd={:?} cmd_present={} placement={layout}",
        spec.cwd,
        spec.cmd.is_some()
    );
    Ok(file)
}

/// Collect the ordered pane list from `--layout <file>` (its `[[pane]]`
/// tables first) followed by repeated `--pane` flags in the order given on
/// the command line.
fn collect_pane_specs(
    layout_file: Option<&str>,
    panes: &[String],
) -> Result<Vec<PaneSpec>, String> {
    let mut specs = Vec::new();
    if let Some(path) = layout_file {
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("could not read layout file {path:?}: {e}"))?;
        let layout: LayoutFile = toml::from_str(&content)
            .map_err(|e| format!("could not parse layout file {path:?}: {e}"))?;
        specs.extend(layout.pane);
    }
    for raw in panes {
        specs.push(parse_pane_flag(raw)?);
    }
    Ok(specs)
}

/// `plexi host start [--layout <file>] [--pane <spec>]... [--timeout-secs <n>] [--background]`
///
/// Linux pid lookup for the process holding `socket_path` open, straight from
/// /proc — no `lsof`, no other binary.
///
/// Two steps, because the kernel exposes the mapping in two places:
/// `/proc/net/unix` maps a bound socket *path* to its inode, and a process's
/// `/proc/<pid>/fd/*` symlinks resolve to `socket:[<inode>]`. Matching the two
/// gives the listener.
///
/// Best-effort, exactly like the `lsof` path it fronts: any unreadable entry
/// (a process that exited mid-scan, another user's `/proc/<pid>/fd`) is
/// skipped rather than failing the lookup.
#[cfg(target_os = "linux")]
fn detect_pid_via_proc(socket_path: &Path) -> Option<u32> {
    let target = socket_path.to_str()?;
    let unix_table = match std::fs::read_to_string("/proc/net/unix") {
        Ok(table) => table,
        Err(e) => {
            log::warn!("host: could not read /proc/net/unix ({e}) — falling back to lsof");
            return None;
        }
    };
    // Columns: Num RefCount Protocol Flags Type St Inode Path
    let inodes: Vec<String> = unix_table
        .lines()
        .filter_map(|line| unix_socket_inode(line, target).map(str::to_owned))
        .collect();
    if inodes.is_empty() {
        return None;
    }

    let self_pid = std::process::id();
    let entries = match std::fs::read_dir("/proc") {
        Ok(entries) => entries,
        Err(e) => {
            log::warn!("host: could not read /proc ({e}) — pid unknown for {socket_path:?}");
            return None;
        }
    };
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue; // not a pid directory
        };
        if pid == self_pid {
            continue; // this CLI may have the socket open too
        }
        let Ok(fds) = std::fs::read_dir(entry.path().join("fd")) else {
            continue; // exited mid-scan, or another user's process
        };
        for fd in fds.flatten() {
            let Ok(link) = std::fs::read_link(fd.path()) else {
                continue;
            };
            let Some(link) = link.to_str() else { continue };
            let Some(inode) = link
                .strip_prefix("socket:[")
                .and_then(|rest| rest.strip_suffix(']'))
            else {
                continue;
            };
            if inodes.iter().any(|i| i == inode) {
                log::info!("host: /proc resolved pid {pid} as the owner of {socket_path:?}");
                return Some(pid);
            }
        }
    }
    None
}

#[cfg(any(target_os = "linux", test))]
fn unix_socket_inode<'a>(line: &'a str, target: &str) -> Option<&'a str> {
    let mut rest = line;
    let mut inode = "";
    for _ in 0..7 {
        rest = rest.trim_start();
        if rest.is_empty() {
            return None;
        }
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        inode = &rest[..end];
        rest = &rest[end..];
    }
    // Only the table columns are whitespace-delimited. The remaining path
    // is verbatim and may contain repeated spaces.
    (rest.trim_start() == target).then_some(inode)
}

/// Launches this channel's app bundle detached (survives the CLI process
/// exiting), seeds any declared panes via the spawn-queue before launch, and
/// blocks until a `notify.sock` round trip confirms the host is ready (or the
/// timeout elapses). Errors non-zero if a host for this channel is already
/// running.
pub fn host_start_cli(
    layout_file: Option<&str>,
    panes: &[String],
    timeout_secs: Option<u64>,
    ephemeral: bool,
    background: bool,
) -> i32 {
    let deadline = Instant::now() + Duration::from_secs(timeout_secs.unwrap_or(15));
    let specs = match collect_pane_specs(layout_file, panes) {
        Ok(s) => s,
        Err(e) => {
            log::error!("host_start: {e}");
            eprintln!("error: {e}");
            return 1;
        }
    };
    // Validate every spec's placement before writing anything to the
    // spawn-queue. Codex review (attempt 1 fix) flagged that writing specs
    // one at a time let an earlier valid pane's JSON file survive a later
    // spec's validation failure — the next unrelated `host start` (or any
    // `plexi` launch) would then drain and open that stale, half-seeded
    // layout. Fail the whole batch atomically instead.
    for spec in &specs {
        if let Err(e) = placement_to_layout(spec) {
            log::error!("host_start: {e}");
            eprintln!("error: {e}");
            return 1;
        }
    }

    let (channel, bundle_path, binary_path) = resolve_channel_paths();
    let socket_path = ipc::endpoint_in(&host_config_dir(channel.as_deref()));
    log::info!(
        "host_start: socket={socket_path:?} pane_count={}",
        specs.len()
    );

    if matches!(probe_notify_socket(&socket_path, deadline.saturating_duration_since(Instant::now())), RunningState::Running) {
        log::warn!("host_start: endpoint already held socket={socket_path:?}");
        eprintln!("error: a Plexi host for this channel is already running or busy (socket {})", socket_path.display());
        let running = running_build_info();
        let installed = installed_identity();
        for line in host_identity_lines(running.as_ref(), installed.as_ref().map(|(t, b)| (t.as_str(), b.as_str())), &restart_hint()) {
            eprintln!("{line}");
        }
        return 1;
    }

    if !bundle_path.exists() {
        log::error!("host_start: bundle not found at {bundle_path:?}");
        eprintln!(
            "error: Plexi is not installed for this channel (expected {})",
            bundle_path.display()
        );
        return 1;
    }
    if !binary_path.exists() {
        log::error!("host_start: binary not found at {binary_path:?}");
        eprintln!(
            "error: Plexi binary missing inside bundle (expected {})",
            binary_path.display()
        );
        return 1;
    }

    let queue_dir = host_config_dir(channel.as_deref()).join("spawn-queue");
    if let Err(e) = std::fs::create_dir_all(&queue_dir) {
        log::error!("host_start: could not create spawn-queue dir {queue_dir:?}: {e}");
        eprintln!("error: could not create spawn queue: {e}");
        return 1;
    }
    let mut seeded_files = Vec::with_capacity(specs.len());
    for (i, spec) in specs.iter().enumerate() {
        match seed_pane(&queue_dir, i, spec) {
            Ok(file) => seeded_files.push(file),
            Err(e) => {
                log::error!(
                    "host_start: {e} — removing {} already-seeded file(s)",
                    seeded_files.len()
                );
                for f in &seeded_files {
                    let _ = std::fs::remove_file(f);
                }
                eprintln!("error: {e}");
                return 1;
            }
        }
    }

    let child_env = host_child_env(
        std::env::vars().collect(),
        channel.as_deref(),
        ephemeral,
        background,
    );
    log::info!(
        "host_start: child env constructed — {} vars retained, PLEXI_CHANNEL={:?}, ephemeral={ephemeral}, background={background}",
        child_env.len(),
        channel
    );

    if Instant::now() >= deadline {
        eprintln!("error: host start deadline expired before launch");
        return 1;
    }
    let pid = match spawn_detached_host(&binary_path, &child_env) {
        Ok(pid) => pid,
        Err(e) => {
            log::error!("host_start: spawn failed for {binary_path:?}: {e}");
            eprintln!("error: could not launch Plexi: {e}");
            return 1;
        }
    };
    log::info!("host_start: process spawned pid={pid} detached=true");
    let timeout = deadline.saturating_duration_since(Instant::now());
    match wait_for_ready(&socket_path, timeout, &host_config_dir(channel.as_deref())) {
        Ok(count) => {
            println!(
                "Plexi host started (pid {}), {count} pane(s) ready.",
                pid
            );
            0
        }
        Err(e) => {
            log::error!("host_start: {e}");
            eprintln!("error: {e}");
            1
        }
    }
}

/// `plexi host stop` — clean shutdown request over `notify.sock` first
/// (`AppRequest::Shutdown`), falling back to `kill -TERM <pid>` (pid via
/// `lsof -t`) if the socket doesn't confirm exit within a short timeout.
/// Detach the spawned host from this terminal so closing the terminal — or
/// Ctrl-C in it — does not take the host down with it.
///
/// Unix: `setsid()` between fork and exec puts the child in its own session,
/// with no controlling terminal.
#[cfg(unix)]
fn detach_from_terminal(cmd: &mut std::process::Command) {
    use std::os::unix::process::CommandExt as _;
    // SAFETY: the pre_exec closure only calls libc::setsid(), an
    // async-signal-safe syscall, between fork and exec — no allocation, no
    // locking, no access to the parent's heap state.
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

/// Detach a restart coordinator twice and report the grandchild PID before it
/// execs. The first child creates a new session, then exits after forking the
/// process that will actually exec the coordinator.
#[cfg(unix)]
fn detach_restart_coordinator(cmd: &mut std::process::Command, report_fd: libc::c_int) {
    use std::os::unix::process::CommandExt as _;
    // SAFETY: this runs after fork and before exec. `setsid`, `fork`, `write`,
    // `close`, and `_exit` are async-signal-safe POSIX calls and work on macOS
    // as well as Linux. The grandchild retains the exec path supplied by Command.
    unsafe {
        cmd.pre_exec(move || {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            match libc::fork() {
                -1 => Err(std::io::Error::last_os_error()),
                0 => {
                    let pid = (libc::getpid() as u32).to_ne_bytes();
                    if libc::write(report_fd, pid.as_ptr().cast(), pid.len())
                        != pid.len() as libc::ssize_t
                    {
                        return Err(std::io::Error::last_os_error());
                    }
                    libc::close(report_fd);
                    Ok(())
                }
                _ => {
                    libc::close(report_fd);
                    libc::_exit(0)
                }
            }
        });
    }
}

/// Spawn the coordinator through the same setsid-plus-double-fork path used
/// by caller-host restarts. The pipe reports the grandchild PID and lets this
/// process reap the short-lived intermediate child.
#[cfg(unix)]
pub(crate) fn spawn_restart_coordinator(
    command: &mut std::process::Command,
) -> std::io::Result<u32> {
    let mut fds = [0; 2];
    // SAFETY: `fds` is valid writable storage for the two pipe descriptors.
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    let read_fd = fds[0];
    let write_fd = fds[1];
    detach_restart_coordinator(command, write_fd);
    let mut intermediate = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            // SAFETY: these descriptors were created by this function.
            unsafe {
                libc::close(read_fd);
                libc::close(write_fd);
            }
            return Err(error);
        }
    };
    // The child has its own descriptor after fork; close this copy before
    // waiting so a failure cannot make the read appear permanently open.
    // SAFETY: this descriptor belongs to this process.
    unsafe {
        libc::close(write_fd);
    }
    let mut pid = [0_u8; std::mem::size_of::<u32>()];
    // SAFETY: `pid` is writable for exactly the requested byte count.
    let read = unsafe { libc::read(read_fd, pid.as_mut_ptr().cast(), pid.len()) };
    // SAFETY: this descriptor belongs to this process.
    unsafe {
        libc::close(read_fd);
    }
    intermediate.wait()?;
    if read != pid.len() as libc::ssize_t {
        return Err(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "restart coordinator did not report its grandchild pid",
        ));
    }
    Ok(u32::from_ne_bytes(pid))
}

#[cfg(windows)]
pub(crate) fn spawn_restart_coordinator(
    command: &mut std::process::Command,
) -> std::io::Result<u32> {
    command.spawn().map(|child| child.id())
}

#[cfg(unix)]
fn spawn_detached_host(binary: &Path, child_env: &[(String, String)]) -> std::io::Result<u32> {
    let mut cmd = std::process::Command::new(binary);
    cmd.env_clear().envs(child_env.iter().cloned())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    detach_from_terminal(&mut cmd);
    cmd.spawn().map(|child| child.id())
}

#[cfg(windows)]
fn spawn_detached_host(binary: &Path, child_env: &[(String, String)]) -> std::io::Result<u32> {
    windows_launch::spawn(binary, child_env)
}

fn shutdown_complete(pid: Option<u32>, socket_path: &Path) -> bool {
    if let Some(pid) = pid {
        // A restart can bind the same endpoint before this caller polls again.
        // Confirm this process's exit without unlinking its replacement's socket.
        return !plexi_distribution::transaction::process_alive(pid);
    }
    #[cfg(unix)]
    if std::fs::symlink_metadata(socket_path)
        .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
    {
        // An absent socket is stopped even when its path exceeds sockaddr_un's
        // length limit (connect would report InvalidInput instead of NotFound).
        return true;
    }
    matches!(
        super::send_line_to_socket(socket_path, b"", Duration::from_millis(100)),
        Err(super::SocketTransportError::Connect(error))
            if matches!(error.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused)
    )
}

/// Uninstall must not delete a profile while a host can still write to it.
pub(crate) fn stop_for_uninstall() -> Result<(), String> {
    let channel = crate::config::build_channel();
    let socket = ipc::endpoint_in(&host_config_dir(channel.as_deref()));
    let pid = running_build_info().as_ref().and_then(reported_pid)
        .or_else(|| detect_pid(&socket));
    if shutdown_complete(pid, &socket) {
        return Ok(());
    }
    if host_stop_cli() != 0 {
        return Err("could not stop the channel host before uninstall".into());
    }
    let deadline = Instant::now() + Duration::from_secs(35);
    while !shutdown_complete(pid, &socket) {
        if Instant::now() >= deadline {
            return Err("channel host has not exited; no uninstall data was removed".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}

#[cfg(all(test, unix))]
#[test]
fn uninstall_accepts_absent_socket_beyond_unix_path_limit() {
    let root = tempfile::tempdir().unwrap();
    let socket = root.path().join("x".repeat(120)).join("notify.sock");
    assert!(shutdown_complete(None, &socket));
}

fn restart_closes_caller(
    pane: Option<&str>,
    source_socket: Option<&Path>,
    target_socket: &Path,
) -> bool {
    pane.is_some_and(|id| !id.is_empty()) && source_socket == Some(target_socket)
}

/// Build the independently reparented restart coordinator with precisely the
/// environment a normal `host start` would inherit. In particular, it cannot
/// carry the old host's cleanup marker or a pane identity into the quit sweep.
pub(crate) fn restart_coordinator_env(
    parent_env: Vec<(String, String)>,
    channel: Option<&str>,
) -> Vec<(String, String)> {
    filter_child_env(parent_env, channel)
}

fn restart_coordinator_command(
    exe: &Path,
    parent_env: Vec<(String, String)>,
    channel: Option<&str>,
) -> std::process::Command {
    let mut command = std::process::Command::new(exe);
    command
        .args(["host", "restart"])
        .env_clear()
        .envs(restart_coordinator_env(parent_env, channel))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    command
}

pub fn host_restart_cli(yes: bool, json: bool) -> i32 {
    let channel = crate::config::build_channel();
    let socket = ipc::endpoint_in(&host_config_dir(channel.as_deref()));
    let pane = std::env::var("PLEXI_PANE_ID").ok();
    let source = std::env::var_os("PLEXI_SOCKET").map(PathBuf::from);
    let own_host = restart_closes_caller(pane.as_deref(), source.as_deref(), &socket);
    if own_host && !yes {
        eprintln!("error: restarting this host will close this pane. Run `plexi host restart --yes` to confirm.");
        return 1;
    }
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(error) => {
            eprintln!("error: locate restart executable: {error}");
            return 1;
        }
    };
    if own_host {
        let mut child = restart_coordinator_command(
            &exe,
            std::env::vars().collect(),
            channel.as_deref(),
        );
        match spawn_restart_coordinator(&mut child) {
            Ok(coordinator_pid) => {
                log::info!(
                    "host_restart: detached coordinator pid={} caller pane will close",
                    coordinator_pid
                );
                if json {
                    println!(
                        "{}",
                        serde_json::json!({"restarting": true, "coordinator_pid": coordinator_pid})
                    );
                } else {
                    println!("Restarting; this pane will close. The saved workspace will restore.");
                }
                return 0;
            }
            Err(error) => {
                log::error!("host_restart: coordinator spawn failed: {error}");
                eprintln!("error: {error}");
                return 1;
            }
        }
    }
    let coordinator_started = Instant::now();
    log::info!(
        "host_restart: coordinator start pid={} channel={channel:?}",
        std::process::id()
    );
    let pid = running_build_info()
        .as_ref()
        .and_then(reported_pid)
        .or_else(|| detect_pid(&socket));
    if !shutdown_complete(pid, &socket) {
        log::info!("host_restart: waiting old_pid={pid:?}");
        let stopped = std::process::Command::new(&exe)
            .args(["host", "stop"])
            .output();
        match stopped {
            Ok(output) if output.status.success() => {}
            Ok(output) => {
                log::error!(
                    "host_restart: old host stop failed status={} stderr={}",
                    output.status,
                    String::from_utf8_lossy(&output.stderr).trim()
                );
                eprintln!("{}", String::from_utf8_lossy(&output.stderr));
                return 1;
            }
            Err(error) => {
                log::error!("host_restart: old host stop invocation failed: {error}");
                eprintln!("error: host stop: {error}");
                return 1;
            }
        }
        let deadline = Instant::now() + Duration::from_secs(35);
        while !shutdown_complete(pid, &socket) {
            if Instant::now() >= deadline {
                log::error!("host_restart: old host did not exit before deadline old_pid={pid:?}");
                eprintln!("error: previous host has not exited; restart aborted");
                return 1;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        log::info!(
            "host_restart: old host exited after {}ms old_pid={pid:?}",
            coordinator_started.elapsed().as_millis()
        );
    } else if !json {
        println!("not running; starting");
    }
    // Reuse the public start handler without seed panes; capture its human output for JSON callers.
    log::info!("host_restart: starting");
    match std::process::Command::new(&exe)
        .args(["host", "start"])
        .output()
    {
        Ok(output) if output.status.success() => {
            let new_pid = running_build_info().as_ref().and_then(reported_pid);
            log::info!("host_restart: started new_pid={new_pid:?}");
            host_status_cli(json)
        }
        Ok(output) => {
            log::error!(
                "host_restart: start failed status={} stderr={}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            );
            eprintln!("{}", String::from_utf8_lossy(&output.stderr));
            1
        }
        Err(error) => {
            log::error!("host_restart: start failed: {error}");
            eprintln!("error: host start: {error}");
            1
        }
    }
}

#[cfg(test)]
#[test]
fn restart_confirmation_only_for_the_callers_host() {
    let socket = Path::new("/scratch/alpha/notify.sock");
    assert!(restart_closes_caller(Some("1"), Some(socket), socket));
    assert!(!restart_closes_caller(
        Some("1"),
        Some(Path::new("/scratch/beta/notify.sock")),
        socket
    ));
    assert!(!restart_closes_caller(None, Some(socket), socket));
}

pub fn host_stop_cli() -> i32 {
    let channel = crate::config::build_channel();
    let profile = host_config_dir(channel.as_deref());
    host_stop_cli_in(&profile, channel)
}

fn host_stop_cli_in(profile: &Path, channel: Option<String>) -> i32 {
    let socket_path = ipc::endpoint_in(profile);
    let pid = running_build_info()
        .as_ref()
        .and_then(reported_pid)
        .or_else(|| detect_pid(&socket_path));
    log::info!("host_stop: channel={channel:?} socket={socket_path:?} pid={pid:?}");

    match IpcStream::connect(&socket_path) {
        Ok(mut stream) => {
            let payload = serde_json::to_string(&crate::protocol::AppRequest::Shutdown)
                .unwrap_or_else(|_| "{\"type\":\"shutdown\"}".to_string());
            match stream.write_all(format!("{payload}\n").as_bytes()) {
                Ok(()) => {
                    log::info!("host_stop: shutdown request sent, waiting for process exit");
                    let deadline = Instant::now() + Duration::from_secs(5);
                    loop {
                        if shutdown_complete(pid, &socket_path) {
                            log::info!("host_stop: original pid={pid:?} stopped");
                            host_stop_stdout("Plexi host stopped (clean shutdown).");
                            return 0;
                        }
                        if Instant::now() >= deadline {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(200));
                    }
                    log::warn!(
                        "host_stop: clean shutdown did not complete within timeout — requesting termination"
                    );
                }
                Err(e) => {
                    log::error!(
                        "host_stop: could not send shutdown request: {e} — requesting termination"
                    );
                }
            }
        }
        Err(e) => {
            log::warn!(
                "host_stop: could not connect to {socket_path:?}: {e} — requesting termination"
            );
        }
    }

    let Some(pid) = pid else {
        log::error!("host_stop: no host running for this channel (no pid, socket {socket_path:?})");
        eprintln!("error: no Plexi host is running for this channel");
        return 1;
    };
    if shutdown_complete(Some(pid), &socket_path) {
        host_stop_stdout("Plexi host stopped.");
        return 0;
    }
    clean_panes_after_socket_failure(profile, pid);
    match terminate_host_process(pid) {
        Ok(()) => {
            log::info!("host_stop: termination requested pid={pid}");
            host_stop_stdout(&format!("Termination requested for Plexi host (pid {pid})."));
            0
        }
        Err(e) => {
            log::error!("host_stop: could not terminate pid={pid}: {e}");
            eprintln!("error: could not terminate process {pid}: {e}");
            1
        }
    }
}

fn host_stop_stdout(message: &str) {
    if let Err(error) = writeln!(std::io::stdout(), "{message}") {
        log::debug!("host_stop: could not write status to stdout: {error}");
    }
}

/// Holds ignored HUP and TERM dispositions only while the socket-failure
/// cleanup can signal the caller's marked pane shell.  The dispositions are
/// restored before `host stop` returns, including on every early error path.
#[cfg(unix)]
struct FallbackSignalGuard {
    previous: Vec<(libc::c_int, libc::sighandler_t)>,
}

#[cfg(unix)]
impl FallbackSignalGuard {
    fn ignore_hup_and_term() -> Self {
        let mut previous = Vec::new();
        for signal in [libc::SIGHUP, libc::SIGTERM] {
            // SAFETY: changing this CLI process's temporary disposition keeps
            // a marked parent shell from terminating its foreground job.
            let old = unsafe { libc::signal(signal, libc::SIG_IGN) };
            if old == libc::SIG_ERR {
                log::warn!(
                    "host_stop: could not ignore signal={signal}: {}",
                    std::io::Error::last_os_error()
                );
            } else {
                previous.push((signal, old));
            }
        }
        Self { previous }
    }
}

#[cfg(unix)]
impl Drop for FallbackSignalGuard {
    fn drop(&mut self) {
        for (signal, previous) in self.previous.drain(..) {
            // SAFETY: restores the disposition this CLI had immediately
            // before the bounded socket-failure cleanup.
            if unsafe { libc::signal(signal, previous) } == libc::SIG_ERR {
                log::warn!(
                    "host_stop: could not restore signal={signal}: {}",
                    std::io::Error::last_os_error()
                );
            }
        }
    }
}

#[cfg(not(unix))]
struct FallbackSignalGuard;

#[cfg(not(unix))]
impl FallbackSignalGuard {
    fn ignore_hup_and_term() -> Self {
        Self
    }
}

fn clean_panes_after_socket_failure(profile: &Path, pid: u32) {
    // A caller in one of this host's panes is itself downstream of the marker
    // sweep. Its shell can forward HUP/TERM to this foreground CLI, so keep it
    // alive long enough for the caller to reach `terminate_host_process`.
    let _fallback_signals = FallbackSignalGuard::ignore_hup_and_term();
    if let Some(marker) = crate::host::process_cleanup::recorded_session_marker(profile, pid) {
        let restored_roots = crate::host::process_cleanup::restore_pane_roots(profile, &marker);
        let report = crate::host::process_cleanup::stop_pane_processes_for_marker(
            &marker,
            Duration::from_secs(2),
        );
        log::info!(
            "host_stop: socket fallback restored_roots={restored_roots} cleaned groups={} escalated={} tmux_ms={} total_ms={}",
            report.groups,
            report.escalated,
            report.tmux.as_millis(),
            (report.scan + report.signal + report.grace_waited + report.kill + report.tmux).as_millis()
        );
    } else {
        log::warn!("host_stop: no matching pane session marker for pid={pid}; skipping pane cleanup");
    }
}

#[cfg(unix)]
fn terminate_host_process(pid: u32) -> std::io::Result<()> {
    signal_host_process(pid, libc::SIGTERM)?;
    let deadline = Instant::now() + Duration::from_millis(250);
    while Instant::now() < deadline {
        if !host_pid_alive(pid) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    if host_pid_alive(pid) {
        signal_host_process(pid, libc::SIGKILL)?;
    }
    Ok(())
}

#[cfg(unix)]
fn host_pid_alive(pid: u32) -> bool {
    // SAFETY: signal 0 observes only the exact, previously identified host pid.
    if unsafe { libc::kill(pid as libc::pid_t, 0) } == 0 {
        return true;
    }
    let error = std::io::Error::last_os_error();
    match error.raw_os_error() {
        Some(libc::ESRCH) | Some(libc::EPERM) => {
            log::debug!("host_stop: probe pid={pid} unavailable: {error}");
        }
        _ => log::warn!("host_stop: probe pid={pid} failed: {error}"),
    }
    false
}

#[cfg(unix)]
fn signal_host_process(pid: u32, signal: i32) -> std::io::Result<()> {
    // SAFETY: pid came from this channel's socket owner and is never a group.
    if unsafe { libc::kill(pid as libc::pid_t, signal) } == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    match error.raw_os_error() {
        Some(libc::ESRCH) | Some(libc::EPERM) => {
            log::debug!("host_stop: signal={signal} pid={pid} unavailable: {error}");
            Ok(())
        }
        _ => Err(error),
    }
}

#[cfg(windows)]
fn terminate_host_process(pid: u32) -> std::io::Result<()> {
    use std::os::windows::io::{FromRawHandle, OwnedHandle};
    use windows_sys::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};
    // SAFETY: scalar inputs and checked handle; only the recorded host is targeted.
    let handle = unsafe { OpenProcess(PROCESS_TERMINATE, 0, pid) };
    if handle.is_null() {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: OpenProcess returned a distinct owned handle, closed on every path.
    let _owned = unsafe { OwnedHandle::from_raw_handle(handle.cast()) };
    // SAFETY: the owned handle remains valid for the operation.
    if unsafe { TerminateProcess(handle, 1) } == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn reported_pid(info: &serde_json::Value) -> Option<u32> {
    let pid = u32::try_from(info.get("pid")?.as_u64()?).ok()?;
    (pid != 0).then_some(pid)
}

/// Query the running host independently of its profile markers or installed package.
pub(crate) fn running_build_info() -> Option<serde_json::Value> {
    let channel = crate::config::build_channel();
    let profile = host_config_dir(channel.as_deref());
    let socket = ipc::endpoint_in(&profile);
    let response_file = crate::rpc::response_file_in(&profile, "host-build", "json");
    let request = crate::protocol::AppRequest::GetBuildInfo { response_file: response_file.clone() };
    let line = serde_json::to_string(&request).ok()?;
    super::send_line_to_socket(&socket, format!("{line}\n").as_bytes(), crate::rpc::DEFAULT_TIMEOUT).ok()?;
    let content = crate::rpc::poll_string(&response_file, Some(crate::rpc::DEFAULT_TIMEOUT)).ok()?;
    serde_json::from_str(&content).ok()
}

/// Human-readable running-vs-installed identity for `host status` and the
/// `host start` already-running path (stint 0596). Returns no lines when the
/// installation is unmanaged or the host did not report a build. When the
/// running build differs from the installed package, the last line is the
/// restart prompt. Never restarts anything on its own.
pub(crate) fn host_identity_lines(
    running: Option<&serde_json::Value>,
    installed: Option<(&str, &str)>,
    stop_start: &str,
) -> Vec<String> {
    let (Some(running), Some((installed_tag, installed_build))) = (running, installed) else {
        return Vec::new();
    };
    let Some(running_build) = running.get("build_id").and_then(|v| v.as_str()) else {
        return Vec::new();
    };
    let running_tag = running.get("tag").and_then(|v| v.as_str()).unwrap_or("unknown");
    let mut lines = vec![
        format!("Running host: {running_tag} ({running_build})"),
        format!("Installed:    {installed_tag} ({installed_build})"),
    ];
    if running_build != installed_build {
        lines.push(format!(
            "The running host is older than the installed build. Restart it to use the update: {stop_start}"
        ));
    }
    lines
}

fn installed_identity() -> Option<(String, String)> {
    let receipt = crate::distribution::installed().ok().flatten()?;
    Some((receipt.active.tag, receipt.active.build_id))
}

pub fn host_workspace_rollback_cli(to_channel: &str) -> i32 {
    if to_channel.is_empty() || !to_channel.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_') {
        eprintln!("error: channel must contain only letters, numbers, hyphens, or underscores");
        return 1;
    }
    let source = crate::config::config_dir();
    let destination = host_config_dir(Some(to_channel));
    match crate::workspace::export_v2_rollback(&source, &destination) {
        Ok(()) => {
            println!("Exported the pre-migration workspace to channel {to_channel}. Later changes remain in the current channel.");
            0
        }
        Err(error) => {
            log::error!("host workspace rollback export failed: {error}");
            eprintln!("error: workspace rollback export failed: {error}");
            1
        }
    }
}

fn restart_hint() -> String {
    let name = crate::config::current_exe_basename();
    format!("{name} host stop && {name} host start")
}

/// `plexi host status [--json]` — ready/not-ready, pane count, pid, socket path.
pub fn host_status_cli(json: bool) -> i32 {
    let channel = crate::config::build_channel();
    let socket_path = ipc::endpoint_in(&host_config_dir(channel.as_deref()));
    let pane_count = query_ready_status(&socket_path, &host_config_dir(channel.as_deref()), Instant::now() + crate::rpc::DEFAULT_TIMEOUT);
    let running = running_build_info();
    let pid = running.as_ref().and_then(reported_pid).or_else(|| detect_pid(&socket_path));
    let ready = pane_count.is_some();
    log::info!(
        "host_status: channel={channel:?} pid={pid:?} socket={socket_path:?} ready={ready} pane_count={pane_count:?}"
    );

    if json {
        let payload = serde_json::json!({
            "ready": ready,
            "cli": crate::distribution::build_info(),
            "running": running,
            "installed": crate::distribution::installed().ok().flatten(),
            "pane_count": pane_count,
            "pid": pid,
            "socket": socket_path.to_string_lossy(),
        });
        println!("{payload}");
    } else if ready {
        println!(
            "Plexi host is running (pid {}), {} pane(s), socket {}",
            pid.map(|p| p.to_string())
                .unwrap_or_else(|| "unknown".to_string()),
            pane_count.unwrap_or(0),
            socket_path.display()
        );
        for line in host_identity_lines(running.as_ref(), installed_identity().as_ref().map(|(t, b)| (t.as_str(), b.as_str())), &restart_hint()) {
            println!("{line}");
        }
    } else {
        println!(
            "Plexi host is not running (socket {})",
            socket_path.display()
        );
    }
    0
}

/// `plexi host log --source <tool> <message>` — write one info-level marker
/// line into the running host's channel log via `AppRequest::LogMarker`.
///
/// Like every `host` subcommand it talks to the channel's `notify.sock`
/// directly (derived from the running binary's own channel, see the module
/// doc) — never through `PLEXI_SOCKET`, which is absent when a release gate
/// runs from a plain terminal. Waits for the host's `{"ok":true}` ack so a
/// nonzero exit reliably means the marker did not land.
pub fn host_log_cli(source: &str, message: &str) -> i32 {
    let channel = crate::config::build_channel();
    let profile_dir = host_config_dir(channel.as_deref());
    let socket_path = ipc::endpoint_in(&profile_dir);
    let response_file = crate::rpc::response_file_in(&profile_dir, "host-log-response", "json");
    log::info!(
        "host_log:cli: channel={channel:?} source={source} message_chars={}",
        message.chars().count()
    );

    let payload = serde_json::json!({
        "type": "log_marker",
        "source": source,
        "message": message,
        "response_file": response_file,
    });
    let mut stream = match IpcStream::connect(&socket_path) {
        Ok(stream) => stream,
        Err(e) => {
            log::error!("host_log: could not connect to {socket_path:?}: {e}");
            eprintln!(
                "error: no Plexi host is running for this channel (socket {}): {e}",
                socket_path.display()
            );
            return 1;
        }
    };
    if let Err(e) = stream.write_all(format!("{payload}\n").as_bytes()) {
        log::error!("host_log: could not send marker over {socket_path:?}: {e}");
        eprintln!("error: could not send log marker: {e}");
        return 1;
    }
    crate::rpc::register_client_stream(response_file.clone(), stream);

    match crate::rpc::poll_string(&response_file, Some(crate::rpc::DEFAULT_TIMEOUT)) {
        Ok(_) => 0,
        Err(crate::rpc::PollError::TimedOut) => {
            eprintln!("error: timed out waiting for host log acknowledgement");
            1
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

/// `plexi host key <key>` — inject a synthetic key into the host-level input
/// path. This deliberately targets the host rather than a pane, so a chord
/// such as `cmd+1` exercises numbered context switching without writing bytes
/// into a focused terminal's PTY.
pub fn host_key_cli(key: &str) -> i32 {
    log::info!(
        "host_key:cli: key={key:?} key_chars={}",
        key.chars().count()
    );
    let content = match super::request(
        serde_json::json!({
            "type": "host_key",
            "key": key,
        }),
        "host-key-response",
        "host key",
    ) {
        Ok(content) => content,
        Err(code) => return code,
    };
    match serde_json::from_str::<serde_json::Value>(&content) {
        Ok(reply) if reply.get("error").is_some() => {
            eprintln!(
                "error: {}",
                reply["error"].as_str().unwrap_or("host key failed")
            );
            1
        }
        Ok(_) => 0,
        Err(error) => {
            eprintln!("error: malformed host key response: {error}");
            1
        }
    }
}

/// `plexi host screenshot [--pane <id>] [--output <path>]` — capture the
/// live host window (optionally cropped to one pane) through the real
/// render pipeline. Prints the written PNG path on success.
pub fn host_screenshot_cli(pane: Option<u64>, output: Option<&str>) -> i32 {
    let output_path = match output {
        Some(path) => path.to_string(),
        None => {
            let stamp = crate::platform::clock::now_secs();
            crate::config::config_dir()
                .join("screenshots")
                .join(format!("host-{stamp}.png"))
                .to_string_lossy()
                .into_owned()
        }
    };
    log::info!("host_screenshot:cli: pane={pane:?} output_path={output_path}");

    // The capture round-trips through the GPU (request frame -> readback ->
    // next frame's input), so allow a little longer than plain state reads.
    let content = match super::request_with(
        serde_json::json!({
            "type": "screenshot",
            "pane_id": pane,
            "output_path": output_path,
        }),
        "screenshot-response",
        "screenshot",
        Duration::from_secs(10),
    ) {
        Ok(content) => content,
        Err(code) => return code,
    };
    match serde_json::from_str::<serde_json::Value>(&content) {
        Ok(v) if v.get("error").is_some() => {
            eprintln!(
                "error: {}",
                v["error"].as_str().unwrap_or("screenshot failed")
            );
            1
        }
        Ok(v) => {
            println!(
                "{} ({}x{})",
                v["path"].as_str().unwrap_or(&output_path),
                v["width"],
                v["height"]
            );
            0
        }
        Err(e) => {
            eprintln!("error: malformed screenshot response: {e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    #[test]
    fn fallback_cleanup_guard_keeps_the_calling_cli_alive() {
        let _guard = super::FallbackSignalGuard::ignore_hup_and_term();
        // This is the disposition a marked parent shell can effectively
        // deliver to its foreground CLI when pane cleanup closes the shell.
        assert_eq!(unsafe { libc::raise(libc::SIGHUP) }, 0);
        assert_eq!(unsafe { libc::raise(libc::SIGTERM) }, 0);
    }

    #[cfg(unix)]
    #[test]
    fn host_stop_socket_failure_cleanup_stops_marked_group() {
        use std::os::unix::process::CommandExt;
        let profile = tempfile::tempdir().unwrap();
        let marker = uuid::Uuid::new_v4().to_string();
        std::fs::write(
            profile.path().join("pane-session"),
            format!("{}\n{marker}\n", std::process::id()),
        )
        .unwrap();
        let mut command = std::process::Command::new("/bin/sh");
        command
            .args([
                "-c",
                "trap '' HUP TERM; : > \"$READY\"; while :; do sleep 600; done",
            ])
            .env("READY", profile.path().join("ready"))
            .env(crate::host::process_cleanup::SESSION_ENV, &marker);
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 { Err(std::io::Error::last_os_error()) } else { Ok(()) }
            });
        }
        let mut child = command.spawn().unwrap();
        // Signals sent before the shell installs its trap would just kill it.
        let ready_deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !profile.path().join("ready").exists() {
            assert!(
                std::time::Instant::now() < ready_deadline,
                "fixture never became ready"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let _registration = crate::host::process_cleanup::PaneProcessGroup::record_for_profile(
            child.id(),
            &marker,
            profile.path(),
        );
        // The fallback runs in a different CLI process, which has no in-host
        // root registry. Keep only the durable registration for this seam.
        crate::host::process_cleanup::forget_pane_root_for_test(child.id());
        super::clean_panes_after_socket_failure(profile.path(), std::process::id());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            // Cleanup reaps its pane roots, so ECHILD means it already exited.
            match child.try_wait() {
                Ok(None) => {}
                Ok(Some(_)) => return,
                Err(error) if error.raw_os_error() == Some(libc::ECHILD) => return,
                Err(error) => panic!("wait on marked group: {error}"),
            }
            if std::time::Instant::now() >= deadline {
                unsafe { libc::kill(-(child.id() as i32), libc::SIGKILL) };
                let _ = child.wait();
                panic!("marked group survived host stop socket fallback");
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    #[test]
    fn socket_failure_cleanup_rejects_mismatched_session_record_pid() {
        let profile = tempfile::tempdir().unwrap();
        std::fs::write(
            profile.path().join("pane-session"),
            format!("{}\nmarker\n", std::process::id().saturating_add(1)),
        )
        .unwrap();
        assert!(crate::host::process_cleanup::recorded_session_marker(
            profile.path(),
            std::process::id()
        )
        .is_none());
    }

    #[test]
    fn identity_lines_prompt_restart_when_running_build_is_older() {
        let running = serde_json::json!({"tag": "v0.1.16", "build_id": "old"});
        let lines = super::host_identity_lines(Some(&running), Some(("v0.2.0", "new")), "plexi host stop && plexi host start");
        assert_eq!(lines.len(), 3);
        assert!(lines[0].contains("v0.1.16") && lines[0].contains("old"));
        assert!(lines[1].contains("v0.2.0") && lines[1].contains("new"));
        assert!(lines[2].contains("plexi host stop && plexi host start"));
    }

    #[test]
    fn identity_lines_have_no_prompt_when_builds_match() {
        let running = serde_json::json!({"tag": "v0.2.0", "build_id": "same"});
        let lines = super::host_identity_lines(Some(&running), Some(("v0.2.0", "same")), "restart");
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn identity_lines_are_empty_without_both_identities() {
        let running = serde_json::json!({"tag": "v0.2.0", "build_id": "x"});
        assert!(super::host_identity_lines(Some(&running), None, "r").is_empty());
        assert!(super::host_identity_lines(None, Some(("v1", "x")), "r").is_empty());
        assert!(super::host_identity_lines(Some(&serde_json::json!({})), Some(("v1", "x")), "r").is_empty());
    }

    #[test]
    fn distribution_socket_inode_preserves_spaces_in_the_path() {
        let path = "/tmp/Plexi home with  spaces/.plexi/notify.sock";
        let line = format!("000000: 00000002 00000000 00010000 0001 01 4711 {path}");
        assert_eq!(super::unix_socket_inode(&line, path), Some("4711"));
        assert_eq!(super::unix_socket_inode(&line, "/tmp/Plexi"), None);
    }

    #[cfg(unix)]
    #[test]
    fn distribution_shutdown_confirms_original_exit_with_replacement_socket_live() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("notify.sock");
        let _replacement = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        let mut original = std::process::Command::new("sleep").arg("30").spawn().unwrap();
        let pid = original.id();
        original.kill().unwrap();
        original.wait().unwrap();
        assert!(super::shutdown_complete(Some(pid), &socket));
        assert!(socket.exists(), "the replacement endpoint must be retained");
        assert!(!super::shutdown_complete(Some(std::process::id()), &socket));
    }

    #[test]
    fn readiness_rejects_reply_after_start_deadline() {
        use crate::platform::ipc::IpcListener;
        use std::io::{BufRead, BufReader};
        let dir = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        let endpoint = dir.path().join("late.sock");
        #[cfg(windows)]
        let endpoint = PathBuf::from(format!(r"\\.\pipe\plexi-late-{}", uuid::Uuid::new_v4()));
        let listener = IpcListener::bind(&endpoint).unwrap();
        let server = std::thread::spawn(move || {
            let stream = listener.incoming().next().unwrap().unwrap();
            let mut line = String::new();
            BufReader::new(stream).read_line(&mut line).unwrap();
            let request: serde_json::Value = serde_json::from_str(&line).unwrap();
            std::thread::sleep(Duration::from_millis(200));
            let response = request["response_file"].as_str().unwrap();
            crate::rpc::write_json_response(response, serde_json::json!([{"id": 1}]));
        });
        let result = wait_for_ready(&endpoint, Duration::from_millis(40), dir.path());
        server.join().unwrap();
        assert!(result.is_err(), "a reply after the startup deadline must not mean ready");
    }

    use super::*;

    #[cfg(windows)]
    #[test]
    fn status_pid_is_the_named_pipe_server() {
        let endpoint = PathBuf::from(format!(r"\\.\pipe\plexi-owner-{}", uuid::Uuid::new_v4()));
        let _listener = crate::platform::ipc::IpcListener::bind(&endpoint).unwrap();
        assert_eq!(detect_pid(&endpoint), Some(std::process::id()));
    }

    #[test]
    fn readiness_zero_budget_does_not_connect() {
        let dir = tempfile::tempdir().unwrap();
        assert!(wait_for_ready(&dir.path().join("absent"), Duration::ZERO, dir.path()).is_err());
        assert!(!dir.path().join("rpc").exists());
    }

    /// Actual framed IPC -> HostHarness application dispatch -> response file.
    /// Complements pane-lifecycle.toml: repeated fresh transports must survive
    /// readiness, spawn, focus, list, close and list, without a paint pass.
    #[test]
    fn readiness_then_pane_new_focus_close_burst() {
        use crate::platform::ipc::IpcListener;
        use std::io::{BufRead, BufReader};
        let dir = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        let endpoint = dir.path().join("burst.sock");
        #[cfg(windows)]
        let endpoint = PathBuf::from(format!(r"\\.\pipe\plexi-burst-{}", uuid::Uuid::new_v4()));
        let listener = IpcListener::bind(&endpoint).unwrap();
        let server = std::thread::spawn(move || {
            let mut host = crate::testing::HostHarness::new();
            host.add_test_pane();
            loop {
                let stream = listener.incoming().next().unwrap().unwrap();
                let mut line = String::new();
                BufReader::new(stream).read_line(&mut line).unwrap();
                if line.trim() == "stop" { break; }
                host.inject_ipc(serde_json::from_str(&line).unwrap());
                host.hidden_frame();
            }
        });
        let timeout = crate::testing::load_aware_timeout(Duration::from_secs(5));
        wait_for_ready(&endpoint, timeout, dir.path()).expect("application ready");
        let send = |request: serde_json::Value| {
            super::super::send_line_to_socket(&endpoint, format!("{request}\n").as_bytes(), timeout).unwrap();
        };
        let list = || {
            let response = crate::rpc::response_file_in(dir.path(), "list", "json");
            send(serde_json::json!({"type":"list_panes", "response_file": response}));
            serde_json::from_str::<Vec<serde_json::Value>>(
                &crate::rpc::poll_string(&response, Some(timeout)).unwrap_or_else(|e| panic!("{e}"))).unwrap()
        };
        for cycle in 0..20 {
            let response = crate::rpc::response_file_in(dir.path(), "spawn", "json");
            send(serde_json::json!({"type":"spawn_pane", "type_id":"file_browser",
                "name": format!("host01-{cycle}"), "response_file": response}));
            let reply = crate::rpc::poll_string(&response, Some(timeout)).unwrap_or_else(|e| panic!("{e}"));
            let reply: serde_json::Value = serde_json::from_str(&reply).unwrap();
            let id = reply["pane_id"].as_u64().expect("spawn pane id");
            send(serde_json::json!({"type":"focus_pane", "pane_id":id}));
            assert!(list().iter().any(|p| p["id"] == id && p["focused"] == true));
            send(serde_json::json!({"type":"close_pane", "pane_id":id}));
            assert!(list().iter().all(|p| p["id"] != id));
        }
        super::super::send_line_to_socket(&endpoint, b"stop\n", timeout).unwrap();
        server.join().unwrap();
    }

    // ── parse_pane_flag ─────────────────────────────────────────────────

    #[test]
    fn parse_pane_flag_cwd_only() {
        let spec = parse_pane_flag("cwd=/tmp").expect("valid spec");
        assert_eq!(
            spec,
            PaneSpec {
                cwd: Some("/tmp".to_string()),
                cmd: None,
                placement: None,
            }
        );
    }

    #[test]
    fn parse_pane_flag_cwd_and_cmd() {
        let spec = parse_pane_flag("cwd=/tmp,cmd=htop").expect("valid spec");
        assert_eq!(spec.cwd, Some("/tmp".to_string()));
        assert_eq!(spec.cmd, Some("htop".to_string()));
        assert_eq!(spec.placement, None);
    }

    #[test]
    fn parse_pane_flag_tab_placement() {
        let spec = parse_pane_flag("cwd=/tmp,tab").expect("valid spec");
        assert_eq!(spec.placement, Some("tab".to_string()));
    }

    #[test]
    fn parse_pane_flag_window_placement() {
        let spec = parse_pane_flag("cwd=/tmp,cmd=top,window").expect("valid spec");
        assert_eq!(spec.placement, Some("window".to_string()));
        assert_eq!(spec.cmd, Some("top".to_string()));
    }

    #[test]
    fn parse_pane_flag_errors_on_missing_cwd() {
        let err = parse_pane_flag("cmd=htop").unwrap_err();
        assert!(
            err.contains("cwd"),
            "error should mention missing cwd: {err}"
        );
    }

    #[test]
    fn parse_pane_flag_errors_on_unknown_key() {
        let err = parse_pane_flag("cwd=/tmp,bogus=1").unwrap_err();
        assert!(
            err.contains("bogus"),
            "error should name the bad key: {err}"
        );
    }

    #[test]
    fn parse_pane_flag_errors_on_unrecognized_token() {
        let err = parse_pane_flag("cwd=/tmp,overlay").unwrap_err();
        assert!(
            err.contains("overlay"),
            "error should name the bad token: {err}"
        );
    }

    #[test]
    fn parse_pane_flag_errors_on_empty_string() {
        assert!(parse_pane_flag("").is_err());
    }

    #[test]
    fn parse_pane_flag_errors_on_duplicate_cwd() {
        assert!(parse_pane_flag("cwd=/tmp,cwd=/other").is_err());
    }

    // ── PaneSpec / LayoutFile TOML deserialization ─────────────────────

    #[test]
    fn layout_file_parses_multiple_pane_tables() {
        let toml_src = r#"
            [[pane]]
            cwd = "/tmp"
            cmd = "htop"

            [[pane]]
            cwd = "/tmp/two"
            placement = "tab"
        "#;
        let layout: LayoutFile = toml::from_str(toml_src).expect("valid layout toml");
        assert_eq!(layout.pane.len(), 2);
        assert_eq!(layout.pane[0].cwd, Some("/tmp".to_string()));
        assert_eq!(layout.pane[0].cmd, Some("htop".to_string()));
        assert_eq!(layout.pane[1].placement, Some("tab".to_string()));
    }

    #[test]
    fn layout_file_defaults_to_empty_pane_list() {
        let layout: LayoutFile = toml::from_str("").expect("empty layout toml is valid");
        assert!(layout.pane.is_empty());
    }

    #[test]
    fn pane_spec_placement_omitted_is_none() {
        let toml_src = r#"cwd = "/tmp""#;
        let spec: PaneSpec = toml::from_str(toml_src).expect("valid pane toml");
        assert_eq!(spec.placement, None);
    }

    // ── placement_to_layout ─────────────────────────────────────────────

    #[test]
    fn placement_to_layout_defaults_to_split_h() {
        let spec = PaneSpec::default();
        assert_eq!(placement_to_layout(&spec).unwrap(), "split_h");
    }

    #[test]
    fn placement_to_layout_maps_tab_and_window() {
        let tab = PaneSpec {
            placement: Some("tab".to_string()),
            ..Default::default()
        };
        let window = PaneSpec {
            placement: Some("window".to_string()),
            ..Default::default()
        };
        assert_eq!(placement_to_layout(&tab).unwrap(), "tab");
        assert_eq!(placement_to_layout(&window).unwrap(), "new_window");
    }

    #[test]
    fn placement_to_layout_rejects_unknown_value() {
        let spec = PaneSpec {
            placement: Some("overlay".to_string()),
            ..Default::default()
        };
        assert!(placement_to_layout(&spec).is_err());
    }

    // ── filter_child_env ─────────────────────────────────────────────────

    #[test]
    fn filter_child_env_strips_pane_scoped_vars_and_sets_channel() {
        let parent = vec![
            ("PLEXI_SOCKET".to_string(), "/tmp/x.sock".to_string()),
            ("PLEXI_PANE_ID".to_string(), "7".to_string()),
            (crate::host::process_cleanup::SESSION_ENV.to_string(), "parent-host".to_string()),
            ("PLEXI_CONTEXT_ID".to_string(), "3".to_string()),
            ("PLEXI_CONTEXT_NAME".to_string(), "work".to_string()),
            ("PLEXI_CONTEXT_DESCRIPTION".to_string(), "work context".to_string()),
            ("PLEXI_CONTEXT_ROOT".to_string(), "/tmp".to_string()),
            ("PLEXI_CONTEXT_DEPTH".to_string(), "1".to_string()),
            ("PLEXI_RUNNING".to_string(), "1".to_string()),
            ("PLEXI_CALL_CREDENTIAL".to_string(), "secret".to_string()),
            ("PLEXI_AGENT_SPAWNED".to_string(), "1".to_string()),
            ("PLEXI_AGENT_HARNESS".to_string(), "codex".to_string()),
            ("PLEXI_AGENT_ROLE".to_string(), "worker".to_string()),
            ("PLEXI_PARENT_PANE".to_string(), "6".to_string()),
            ("PLEXI_EPHEMERAL".to_string(), "1".to_string()),
            ("PLEXI_RESUME_COMMAND".to_string(), "resume".to_string()),
            ("PLEXI_HOST_MCP_PORT".to_string(), "9999".to_string()),
            ("PLEXI_HOST_MCP_TOKEN".to_string(), "secret".to_string()),
            ("PLEXI_CHANNEL".to_string(), "beta".to_string()), // must be overridden, not inherited
            (
                crate::workspace::EPHEMERAL_SESSION_ENV.to_string(),
                "1".to_string(), // must not cascade into later host starts
            ),
            (
                crate::workspace::BACKGROUND_SESSION_ENV.to_string(),
                "1".to_string(), // must not cascade into later host starts
            ),
            ("PATH".to_string(), "/usr/bin".to_string()),
        ];
        let env = filter_child_env(parent, Some("alpha"));
        let map: std::collections::HashMap<_, _> = env.into_iter().collect();
        assert_eq!(map.get(crate::workspace::EPHEMERAL_SESSION_ENV), None);
        assert_eq!(map.get(crate::workspace::BACKGROUND_SESSION_ENV), None);
        assert_eq!(map.get("PLEXI_SOCKET"), None);
        assert_eq!(map.get("PLEXI_PANE_ID"), None);
        assert_eq!(map.get(crate::host::process_cleanup::SESSION_ENV), None);
        assert_eq!(map.get("PLEXI_CONTEXT_ID"), None);
        assert_eq!(map.get("PLEXI_CONTEXT_NAME"), None);
        assert_eq!(map.get("PLEXI_CONTEXT_DESCRIPTION"), None);
        assert_eq!(map.get("PLEXI_CONTEXT_ROOT"), None);
        assert_eq!(map.get("PLEXI_CONTEXT_DEPTH"), None);
        assert_eq!(map.get("PLEXI_RUNNING"), None);
        assert_eq!(map.get("PLEXI_CALL_CREDENTIAL"), None);
        assert_eq!(map.get("PLEXI_AGENT_SPAWNED"), None);
        assert_eq!(map.get("PLEXI_AGENT_HARNESS"), None);
        assert_eq!(map.get("PLEXI_AGENT_ROLE"), None);
        assert_eq!(map.get("PLEXI_PARENT_PANE"), None);
        assert_eq!(map.get("PLEXI_EPHEMERAL"), None);
        assert_eq!(map.get("PLEXI_RESUME_COMMAND"), None);
        assert_eq!(map.get("PLEXI_HOST_MCP_PORT"), None);
        assert_eq!(map.get("PLEXI_HOST_MCP_TOKEN"), None);
        assert_eq!(map.get("PLEXI_CHANNEL"), Some(&"alpha".to_string()));
        assert_eq!(map.get("PATH"), Some(&"/usr/bin".to_string()));
    }

    #[test]
    fn filter_child_env_leaves_channel_unset_for_main() {
        let parent = vec![("PLEXI_CHANNEL".to_string(), "beta".to_string())];
        let env = filter_child_env(parent, None);
        assert!(
            env.iter().all(|(k, _)| k != "PLEXI_CHANNEL"),
            "main channel must never inherit a stale PLEXI_CHANNEL: {env:?}"
        );
    }

    #[test]
    fn restart_coordinator_command_scrubs_host_marker_and_pane_identity() {
        let parent = vec![
            (crate::host::process_cleanup::SESSION_ENV.to_string(), "old-host".to_string()),
            ("PLEXI_SOCKET".to_string(), "/tmp/notify.sock".to_string()),
            ("PLEXI_PANE_ID".to_string(), "7".to_string()),
            ("PLEXI_CONTEXT_ROOT".to_string(), "/work".to_string()),
            ("PLEXI_AGENT_SPAWNED".to_string(), "1".to_string()),
            ("PLEXI_CALL_CREDENTIAL".to_string(), "secret".to_string()),
            ("PLEXI_CHANNEL".to_string(), "beta".to_string()),
            ("PATH".to_string(), "/usr/bin".to_string()),
        ];
        let command = restart_coordinator_command(Path::new("/bin/true"), parent, Some("alpha"));
        let env: std::collections::HashMap<_, _> = command
            .get_envs()
            .filter_map(|(key, value)| value.map(|value| (key, value)))
            .collect();
        for key in [
            crate::host::process_cleanup::SESSION_ENV,
            "PLEXI_SOCKET",
            "PLEXI_PANE_ID",
            "PLEXI_CONTEXT_ROOT",
            "PLEXI_AGENT_SPAWNED",
            "PLEXI_CALL_CREDENTIAL",
        ] {
            assert!(!env.contains_key(std::ffi::OsStr::new(key)), "leaked {key}");
        }
        assert_eq!(env.get(std::ffi::OsStr::new("PLEXI_CHANNEL")), Some(&std::ffi::OsStr::new("alpha")));
        assert_eq!(env.get(std::ffi::OsStr::new("PATH")), Some(&std::ffi::OsStr::new("/usr/bin")));
    }

    #[test]
    fn child_env_adds_background_only_when_explicit() {
        let inherited = vec![(
            crate::workspace::BACKGROUND_SESSION_ENV.to_string(),
            "1".to_string(),
        )];
        let foreground = host_child_env(inherited.clone(), Some("alpha"), false, false);
        assert!(
            foreground
                .iter()
                .all(|(key, _)| key != crate::workspace::BACKGROUND_SESSION_ENV)
        );

        let background = host_child_env(inherited, Some("alpha"), false, true);
        assert_eq!(
            background
                .iter()
                .filter(|(key, value)| {
                    key == crate::workspace::BACKGROUND_SESSION_ENV && value == "1"
                })
                .count(),
            1
        );
    }

    // ── host_config_dir ─────────────────────────────────────────────────

    #[test]
    fn host_config_dir_depends_only_on_channel_argument() {
        // Regression for a P1 found in Codex review of stint 0342: host
        // start/stop/status must resolve their profile dir from the
        // resolved binary channel only, never from an inherited
        // PLEXI_CHANNEL env var (e.g. when run from inside a
        // differently-channeled pane). Unlike `config::config_dir()`,
        // `host_config_dir` reads no environment or global state at all —
        // calling it twice with the same argument from concurrent test
        // threads (this suite runs threaded) must always agree, which
        // would not hold if it consulted process env underneath.
        assert_eq!(
            host_config_dir(Some("alpha")),
            host_config_dir(Some("alpha"))
        );
        assert!(
            host_config_dir(Some("alpha")).ends_with(".plexi-alpha"),
            "expected .plexi-alpha profile dir"
        );
    }

    #[test]
    fn host_config_dir_main_channel_has_no_suffix() {
        let dir = host_config_dir(None);
        assert!(
            dir.ends_with(".plexi"),
            "expected .plexi profile dir, got {dir:?}"
        );
    }

    // ── collect_pane_specs ordering ───────────────────────────────────────

    #[test]
    fn collect_pane_specs_layout_file_before_cli_panes() {
        let dir = std::env::temp_dir().join(format!("plexi-host-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let layout_path = dir.join("layout.toml");
        std::fs::write(&layout_path, "[[pane]]\ncwd = \"/from/layout\"\n").unwrap();

        let panes = vec!["cwd=/from/flag".to_string()];
        let specs = collect_pane_specs(Some(layout_path.to_str().unwrap()), &panes)
            .expect("valid combined specs");

        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].cwd, Some("/from/layout".to_string()));
        assert_eq!(specs[1].cwd, Some("/from/flag".to_string()));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn collect_pane_specs_errors_on_malformed_layout_toml() {
        let dir = std::env::temp_dir().join(format!("plexi-host-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let layout_path = dir.join("bad.toml");
        std::fs::write(&layout_path, "not valid toml [[[").unwrap();

        let err = collect_pane_specs(Some(layout_path.to_str().unwrap()), &[]).unwrap_err();
        assert!(err.contains("could not parse layout file"), "got: {err}");

        let _ = std::fs::remove_dir_all(&dir);
    }
}

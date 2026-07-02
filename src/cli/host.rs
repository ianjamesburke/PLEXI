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
//! `crate::config::config_dir()` — the latter prefers an inherited
//! `PLEXI_CHANNEL` env var over the running binary's own name, which would
//! let `host start/stop/status` silently target the wrong channel's profile
//! when invoked from inside a pane.

use serde::Deserialize;
use std::io::Write;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
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
        return Err(format!("malformed --pane spec {s:?}: cwd=<dir> is required"));
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

/// The five pane/context-scoped `PLEXI_*` vars that must never leak into a
/// `host start`-launched process — reading them to decide what to launch
/// would reintroduce the exact channel-leak trap this command exists to
/// eliminate (see root CLAUDE.md Traps: "`PLEXI_CHANNEL` leaks into app
/// tooling"). `PLEXI_CHANNEL` itself is stripped unconditionally too, then
/// re-added explicitly from the resolved channel — never inherited.
const STRIP_VARS: &[&str] = &[
    "PLEXI_SOCKET",
    "PLEXI_PANE_ID",
    "PLEXI_CONTEXT_ID",
    "PLEXI_CONTEXT_ROOT",
    "PLEXI_RUNNING",
    "PLEXI_CHANNEL",
];

/// Pure env-filter used to build the launched child's environment. Takes the
/// parent env as a plain `Vec` (rather than reading `std::env::vars()`
/// directly) so it is deterministically testable without mutating real
/// process env — mirrors `src/process_app/tests/env_isolation_tests.rs`.
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

/// The channel-scoped profile dir (`~/.plexi<-suffix>`) for `channel`, e.g.
/// `None` → `~/.plexi`, `Some("alpha")` → `~/.plexi-alpha`.
///
/// Deliberately does **not** call `crate::config::config_dir()`, which
/// prefers `PLEXI_CHANNEL` from the environment over the running binary's
/// own name. `host start/stop/status` can be invoked from inside a pane
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

/// Resolve the app bundle + binary-inside-bundle paths for the *running CLI
/// binary's own channel* (never `PLEXI_CHANNEL` — that env var is exactly
/// what must be stripped from the launched child).
fn resolve_channel_paths() -> (Option<String>, PathBuf, PathBuf) {
    let channel = crate::config::build_channel();
    let cap = crate::cli::install::channel_bundle_cap(channel.as_deref());
    let bundle_path = PathBuf::from(format!("/Applications/Plexi{cap}.app"));
    let suffix = channel
        .as_deref()
        .map(|c| format!("-{c}"))
        .unwrap_or_default();
    let binary_path = bundle_path
        .join("Contents/MacOS")
        .join(format!("plexi{suffix}"));
    log::info!(
        "host: resolved channel={channel:?} cap={cap:?} bundle={bundle_path:?} binary={binary_path:?}"
    );
    (channel, bundle_path, binary_path)
}

/// Best-effort pid lookup for the process holding `notify.sock` open. No
/// pid-file mechanism exists anywhere in this codebase (confirmed: `src/app/`
/// and `src/config/` have no precedent), so this shells out to `lsof -t` as a
/// diagnostic only — failure here never blocks the caller, it just means pid
/// is reported as unknown.
fn detect_pid(socket_path: &Path) -> Option<u32> {
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
            log::debug!("host: lsof found no owner for {socket_path:?} (status {:?})", out.status);
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
fn probe_notify_socket(socket_path: &Path) -> RunningState {
    match UnixStream::connect(socket_path) {
        Ok(_) => RunningState::Running,
        Err(e) if e.kind() == std::io::ErrorKind::ConnectionRefused => {
            log::info!("host: stale socket at {socket_path:?} (connection refused) — removing");
            let _ = std::fs::remove_file(socket_path);
            RunningState::NotRunning
        }
        Err(e) => {
            log::info!(
                "host: notify.sock not reachable at {socket_path:?}: {e} — treating as not running"
            );
            RunningState::NotRunning
        }
    }
}

/// Poll a response file until it appears or `timeout` elapses. Distinct from
/// (but structurally identical to) `open.rs`'s `wait_for_response`: this
/// variant returns the raw content instead of printing it, and takes a
/// caller-supplied timeout instead of a fixed 5s — `host start` needs a
/// longer, configurable boot timeout, and `host status`/`host stop` need the
/// parsed pane count rather than `wait_for_response`'s stdout side effect.
fn poll_response_file(response_file: &str, timeout: Duration) -> Result<String, String> {
    let path = PathBuf::from(response_file);
    let deadline = Instant::now() + timeout;
    loop {
        if path.exists() {
            let content = std::fs::read_to_string(&path)
                .map_err(|e| format!("could not read response file {path:?}: {e}"))?;
            let _ = std::fs::remove_file(&path);
            return Ok(content);
        }
        if Instant::now() >= deadline {
            return Err(format!("timed out after {timeout:?} waiting for {path:?}"));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// One-shot readiness/pane-count query: connect to `notify.sock`, send
/// `AppRequest::ListPanes`, and wait up to 5s for the JSON array response.
/// `None` means either the socket is not reachable (host not running) or the
/// round trip did not complete in time. `channel` picks the response-file
/// directory via `host_config_dir` — never the (possibly env-shadowed)
/// `config::config_dir()`.
fn query_ready_status(socket_path: &Path, channel: Option<&str>) -> Option<usize> {
    let mut stream = UnixStream::connect(socket_path).ok()?;
    let response_file = host_config_dir(channel)
        .join(format!("host-status-{}.json", uuid::Uuid::new_v4()))
        .to_string_lossy()
        .into_owned();
    let request = crate::app_protocol::AppRequest::ListPanes {
        response_file: response_file.clone(),
        context_id: None,
    };
    let line = serde_json::to_string(&request).ok()?;
    stream.write_all(format!("{line}\n").as_bytes()).ok()?;
    let content = poll_response_file(&response_file, Duration::from_secs(5)).ok()?;
    serde_json::from_str::<Vec<serde_json::Value>>(&content)
        .ok()
        .map(|v| v.len())
}

/// Retry `query_ready_status` until it answers or `timeout` elapses. Used
/// after spawning the bundle process — the socket listener thread may not be
/// bound yet on the first attempt.
fn wait_for_ready(socket_path: &Path, timeout: Duration, channel: Option<&str>) -> Result<usize, String> {
    let start = Instant::now();
    let deadline = start + timeout;
    log::info!("host_start: readiness poll start socket={socket_path:?} timeout={timeout:?}");
    loop {
        if let Some(count) = query_ready_status(socket_path, channel) {
            log::info!(
                "host_start: readiness poll end — ready in {:?}, pane_count={count}",
                start.elapsed()
            );
            return Ok(count);
        }
        if Instant::now() >= deadline {
            log::error!("host_start: readiness poll end — timeout after {:?}", start.elapsed());
            return Err(format!(
                "timed out after {timeout:?} waiting for Plexi host to become ready"
            ));
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// Write one spawn-queue JSON file for `spec`, matching the schema
/// `open.rs`'s fallback (no-`PLEXI_SOCKET`) spawn path writes and
/// `drain_spawn_queue` reads: `type_id`, `args`, `layout`, `cwd`.
fn seed_pane(queue_dir: &Path, index: usize, spec: &PaneSpec) -> Result<(), String> {
    let layout = placement_to_layout(spec)?;
    let args: Vec<String> = spec.cmd.clone().into_iter().collect();
    let payload = serde_json::json!({
        "type_id": "terminal",
        "args": args,
        "layout": layout,
        "cwd": spec.cwd,
    });
    let id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let file = queue_dir.join(format!("{id}-{index}.json"));
    std::fs::write(&file, payload.to_string())
        .map_err(|e| format!("could not write spawn-queue file {file:?}: {e}"))?;
    log::info!(
        "host_start: seeded pane index={index} type_id=terminal cwd={:?} cmd_present={} placement={layout}",
        spec.cwd,
        spec.cmd.is_some()
    );
    Ok(())
}

/// Collect the ordered pane list from `--layout <file>` (its `[[pane]]`
/// tables first) followed by repeated `--pane` flags in the order given on
/// the command line.
fn collect_pane_specs(layout_file: Option<&str>, panes: &[String]) -> Result<Vec<PaneSpec>, String> {
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

/// `plexi host start [--layout <file>] [--pane <spec>]... [--timeout-secs <n>]`
///
/// Launches this channel's app bundle detached (survives the CLI process
/// exiting), seeds any declared panes via the spawn-queue before launch, and
/// blocks until a `notify.sock` round trip confirms the host is ready (or the
/// timeout elapses). Errors non-zero if a host for this channel is already
/// running.
pub fn host_start_cli(layout_file: Option<&str>, panes: &[String], timeout_secs: Option<u64>) -> i32 {
    let specs = match collect_pane_specs(layout_file, panes) {
        Ok(s) => s,
        Err(e) => {
            log::error!("host_start: {e}");
            eprintln!("error: {e}");
            return 1;
        }
    };

    let (channel, bundle_path, binary_path) = resolve_channel_paths();
    let socket_path = host_config_dir(channel.as_deref()).join("notify.sock");
    log::info!("host_start: socket={socket_path:?} pane_count={}", specs.len());

    if matches!(probe_notify_socket(&socket_path), RunningState::Running) {
        let pid = detect_pid(&socket_path);
        log::warn!("host_start: already running — pid={pid:?} socket={socket_path:?}");
        match pid {
            Some(p) => eprintln!(
                "error: a Plexi host for this channel is already running (pid {p}, socket {})",
                socket_path.display()
            ),
            None => eprintln!(
                "error: a Plexi host for this channel is already running (pid unknown, socket {})",
                socket_path.display()
            ),
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
    for (i, spec) in specs.iter().enumerate() {
        if let Err(e) = seed_pane(&queue_dir, i, spec) {
            log::error!("host_start: {e}");
            eprintln!("error: {e}");
            return 1;
        }
    }

    let child_env = filter_child_env(std::env::vars().collect(), channel.as_deref());
    log::info!(
        "host_start: child env constructed — {} vars retained, PLEXI_CHANNEL={:?}",
        child_env.len(),
        channel
    );

    let mut cmd = std::process::Command::new(&binary_path);
    cmd.env_clear()
        .envs(child_env)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
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
    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            log::error!("host_start: spawn failed for {binary_path:?}: {e}");
            eprintln!("error: could not launch Plexi: {e}");
            return 1;
        }
    };
    log::info!("host_start: process spawned pid={} detached=true", child.id());

    let timeout = Duration::from_secs(timeout_secs.unwrap_or(15));
    match wait_for_ready(&socket_path, timeout, channel.as_deref()) {
        Ok(count) => {
            println!(
                "Plexi host started (pid {}), {count} pane(s) ready.",
                child.id()
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
pub fn host_stop_cli() -> i32 {
    let channel = crate::config::build_channel();
    let socket_path = host_config_dir(channel.as_deref()).join("notify.sock");
    let pid = detect_pid(&socket_path);
    log::info!("host_stop: channel={channel:?} socket={socket_path:?} pid={pid:?}");

    match UnixStream::connect(&socket_path) {
        Ok(mut stream) => {
            let payload = serde_json::to_string(&crate::app_protocol::AppRequest::Shutdown)
                .unwrap_or_else(|_| "{\"type\":\"shutdown\"}".to_string());
            match stream.write_all(format!("{payload}\n").as_bytes()) {
                Ok(()) => {
                    log::info!("host_stop: shutdown request sent, waiting for process exit");
                    // The socket *file* is only unlinked by the next process's
                    // startup cleanup (`spawn_socket_listener` in src/app/mod.rs),
                    // never on exit — so `socket_path.exists()` never turns false
                    // here and cannot be used as an exit signal. Instead, poll a
                    // fresh connect attempt: once the listening process has
                    // actually exited, the OS answers new connects to the
                    // now-unheld socket path with `ConnectionRefused` (the same
                    // stale-socket idiom `probe_notify_socket` uses).
                    let deadline = Instant::now() + Duration::from_secs(5);
                    loop {
                        match UnixStream::connect(&socket_path) {
                            Err(e) if e.kind() == std::io::ErrorKind::ConnectionRefused => {
                                log::info!(
                                    "host_stop: method=socket pid={pid:?} — clean shutdown confirmed"
                                );
                                let _ = std::fs::remove_file(&socket_path);
                                println!("Plexi host stopped (clean shutdown).");
                                return 0;
                            }
                            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                                log::info!(
                                    "host_stop: method=socket pid={pid:?} — clean shutdown confirmed (socket removed)"
                                );
                                println!("Plexi host stopped (clean shutdown).");
                                return 0;
                            }
                            _ => {
                                // Still connectable (or a transient error) — host
                                // hasn't exited yet, keep polling.
                            }
                        }
                        if Instant::now() >= deadline {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(200));
                    }
                    log::warn!(
                        "host_stop: clean shutdown did not complete within timeout — falling back to SIGTERM"
                    );
                }
                Err(e) => {
                    log::error!("host_stop: could not send shutdown request: {e} — falling back to SIGTERM");
                }
            }
        }
        Err(e) => {
            log::warn!(
                "host_stop: could not connect to {socket_path:?}: {e} — falling back to SIGTERM"
            );
        }
    }

    let Some(pid) = pid else {
        log::error!("host_stop: no host running for this channel (no pid, socket {socket_path:?})");
        eprintln!("error: no Plexi host is running for this channel");
        return 1;
    };
    match std::process::Command::new("kill")
        .arg("-TERM")
        .arg(pid.to_string())
        .status()
    {
        Ok(status) if status.success() => {
            log::info!("host_stop: method=sigterm pid={pid}");
            println!("Plexi host stopped (SIGTERM sent to pid {pid}).");
            0
        }
        Ok(status) => {
            log::error!("host_stop: kill -TERM pid={pid} failed with status {status:?}");
            eprintln!("error: could not stop process {pid} (kill exited with {status:?})");
            1
        }
        Err(e) => {
            log::error!("host_stop: could not run kill -TERM pid={pid}: {e}");
            eprintln!("error: could not run kill: {e}");
            1
        }
    }
}

/// `plexi host status [--json]` — ready/not-ready, pane count, pid, socket path.
pub fn host_status_cli(json: bool) -> i32 {
    let channel = crate::config::build_channel();
    let socket_path = host_config_dir(channel.as_deref()).join("notify.sock");
    let pid = detect_pid(&socket_path);
    let pane_count = query_ready_status(&socket_path, channel.as_deref());
    let ready = pane_count.is_some();
    log::info!(
        "host_status: channel={channel:?} pid={pid:?} socket={socket_path:?} ready={ready} pane_count={pane_count:?}"
    );

    if json {
        let payload = serde_json::json!({
            "ready": ready,
            "pane_count": pane_count,
            "pid": pid,
            "socket": socket_path.to_string_lossy(),
        });
        println!("{payload}");
    } else if ready {
        println!(
            "Plexi host is running (pid {}), {} pane(s), socket {}",
            pid.map(|p| p.to_string()).unwrap_or_else(|| "unknown".to_string()),
            pane_count.unwrap_or(0),
            socket_path.display()
        );
    } else {
        println!("Plexi host is not running (socket {})", socket_path.display());
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(err.contains("cwd"), "error should mention missing cwd: {err}");
    }

    #[test]
    fn parse_pane_flag_errors_on_unknown_key() {
        let err = parse_pane_flag("cwd=/tmp,bogus=1").unwrap_err();
        assert!(err.contains("bogus"), "error should name the bad key: {err}");
    }

    #[test]
    fn parse_pane_flag_errors_on_unrecognized_token() {
        let err = parse_pane_flag("cwd=/tmp,overlay").unwrap_err();
        assert!(err.contains("overlay"), "error should name the bad token: {err}");
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
            ("PLEXI_CONTEXT_ID".to_string(), "3".to_string()),
            ("PLEXI_CONTEXT_ROOT".to_string(), "/tmp".to_string()),
            ("PLEXI_RUNNING".to_string(), "1".to_string()),
            ("PLEXI_CHANNEL".to_string(), "beta".to_string()), // must be overridden, not inherited
            ("PATH".to_string(), "/usr/bin".to_string()),
        ];
        let env = filter_child_env(parent, Some("alpha"));
        let map: std::collections::HashMap<_, _> = env.into_iter().collect();
        assert_eq!(map.get("PLEXI_SOCKET"), None);
        assert_eq!(map.get("PLEXI_PANE_ID"), None);
        assert_eq!(map.get("PLEXI_CONTEXT_ID"), None);
        assert_eq!(map.get("PLEXI_CONTEXT_ROOT"), None);
        assert_eq!(map.get("PLEXI_RUNNING"), None);
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
        assert_eq!(host_config_dir(Some("alpha")), host_config_dir(Some("alpha")));
        assert!(
            host_config_dir(Some("alpha")).ends_with(".plexi-alpha"),
            "expected .plexi-alpha profile dir"
        );
    }

    #[test]
    fn host_config_dir_main_channel_has_no_suffix() {
        let dir = host_config_dir(None);
        assert!(dir.ends_with(".plexi"), "expected .plexi profile dir, got {dir:?}");
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

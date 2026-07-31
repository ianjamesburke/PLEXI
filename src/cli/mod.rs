use serde::Deserialize;
use std::collections::HashMap;

pub(super) const APP_ID: &str = "plexi-run";
pub(super) fn commands_file() -> String {
    format!("{}/commands.toml", crate::config::workspace_channel_dir())
}

/// Parsed .plexi/commands.toml
#[derive(Deserialize)]
pub struct PlexiCommands {
    #[serde(default)]
    pub secrets: SecretsConfig,
    #[serde(default)]
    pub commands: HashMap<String, CommandEntry>,
}

#[derive(Deserialize, Default)]
pub struct SecretsConfig {
    #[serde(default)]
    pub required: Vec<String>,
}

/// A command entry: either a bare string (`build = "cargo build"`) or an inline table
/// (`build = { run = "cargo build", description = "..." }`).
/// The old nested-section form (`[commands.build]\nrun = "..."`) is TOML-equivalent to the
/// inline-table form and parses identically — no migration needed.
#[derive(Deserialize)]
#[serde(untagged)]
pub enum CommandEntry {
    Simple(String),
    Full(CommandDef),
}

impl CommandEntry {
    pub fn run(&self) -> &str {
        match self {
            CommandEntry::Simple(s) => s,
            CommandEntry::Full(d) => &d.run,
        }
    }

    pub fn description(&self) -> Option<&str> {
        match self {
            CommandEntry::Simple(_) => None,
            CommandEntry::Full(d) => d.description.as_deref(),
        }
    }

    pub fn secrets(&self) -> &[String] {
        match self {
            CommandEntry::Simple(_) => &[],
            CommandEntry::Full(d) => &d.secrets,
        }
    }
}

#[derive(Deserialize)]
pub struct CommandDef {
    pub run: String,
    pub description: Option<String>,
    #[serde(default)]
    pub secrets: Vec<String>,
}

/// List executable files in a scripts directory.
pub(super) fn list_global_scripts(scripts_dir: &std::path::Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(scripts_dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter(|e| {
            let path = e.path();
            path.is_file() && is_executable(&path)
        })
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    names.sort();
    names
}

pub(super) fn is_executable(path: &std::path::Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        path.exists()
    }
}

/// Scope of a user-authored command surfaced in the command palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserCommandScope {
    /// Defined in the focused workspace's channel-scoped `commands.toml`.
    Workspace,
    /// A global profile script under `config_dir()/scripts/`.
    Global,
}

/// A user-authored command resolved for the command palette.
///
/// Mirrors exactly what `plexi run <name>` would resolve from the given
/// workspace cwd, so the palette never lists a command `plexi run` cannot
/// execute.
#[derive(Debug, Clone)]
pub struct ResolvedUserCommand {
    pub name: String,
    /// Shell snippet for `commands.toml` entries (shown as preview text).
    /// `None` for global scripts — the run target is the script file itself.
    pub run: Option<String>,
    pub description: Option<String>,
    pub scope: UserCommandScope,
}

/// Resolve user commands for the command palette, mirroring `plexi run`
/// precedence: a workspace `commands.toml` shadows global scripts entirely
/// (same fallback chain as [`run::run_command`]). A present-but-unparseable
/// `commands.toml` yields no commands rather than silently falling back to
/// global scripts, because `plexi run` would error in that state.
///
/// Reads the file fresh on every call, so the palette hot-reloads on the next
/// open with no mtime bookkeeping — same pattern as palette note loading.
pub fn resolve_user_commands(workspace_root: Option<&std::path::Path>) -> Vec<ResolvedUserCommand> {
    if let Some(root) = workspace_root {
        let config_path = root.join(commands_file());
        match std::fs::read_to_string(&config_path) {
            Ok(contents) => {
                // commands.toml present — its commands are the entire set; global
                // scripts are NOT consulted (mirrors run_command's fallback chain).
                match toml::from_str::<PlexiCommands>(&contents) {
                    Ok(config) => {
                        let mut cmds: Vec<ResolvedUserCommand> = config
                            .commands
                            .iter()
                            .map(|(name, entry)| ResolvedUserCommand {
                                name: name.clone(),
                                run: Some(entry.run().to_string()),
                                description: entry.description().map(|s| s.to_string()),
                                scope: UserCommandScope::Workspace,
                            })
                            .collect();
                        cmds.sort_by(|a, b| a.name.cmp(&b.name));
                        log::info!(
                            "palette: resolved {} workspace command(s) from {}",
                            cmds.len(),
                            config_path.display()
                        );
                        return cmds;
                    }
                    Err(e) => {
                        log::warn!(
                            "palette: failed to parse {}: {e}; no workspace commands surfaced",
                            config_path.display()
                        );
                        return Vec::new();
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // No workspace commands.toml — fall through to global scripts.
            }
            Err(e) => {
                log::warn!(
                    "palette: could not read {}: {e}; no workspace commands surfaced",
                    config_path.display()
                );
                return Vec::new();
            }
        }
    }

    // Global fallback: scripts dir, same as `plexi run` with no commands.toml.
    let scripts_dir = crate::config::config_dir().join("scripts");
    let mut cmds: Vec<ResolvedUserCommand> = list_global_scripts(&scripts_dir)
        .into_iter()
        .map(|name| ResolvedUserCommand {
            name,
            run: None,
            description: None,
            scope: UserCommandScope::Global,
        })
        .collect();
    cmds.sort_by(|a, b| a.name.cmp(&b.name));
    log::info!(
        "palette: resolved {} global script(s) from {}",
        cmds.len(),
        scripts_dir.display()
    );
    cmds
}

pub mod args;
pub mod crawl;
pub mod help;
pub mod registry;
pub mod setup;
#[cfg(test)]
mod skill_surface;
pub mod subprocess;
#[cfg(test)]
pub mod test_env;

pub mod account;
pub mod agent;
pub mod ai;
pub mod app;
pub mod app_check;
pub mod app_state;
pub mod completions;
pub mod config_cli;
pub mod context_cli;
pub mod demo;
pub mod descriptor;
pub mod doctor;
pub mod events;
pub mod host;
pub mod install;
pub mod install_host;
pub mod list;
pub mod marketplace;
pub mod notes;
pub mod notify;
pub mod open;
pub mod pane;
pub mod registry_watch;
pub mod release_resolver;
pub mod routine;
pub mod run;
pub mod updater;
pub mod validate;
pub mod workspace;

pub(super) fn print_tip(msg: &str) {
    let config = crate::config::PlexiConfig::load_with_workspace(
        crate::config::active_workspace_root().as_deref(),
    );
    let enabled = config.cli.as_ref().and_then(|c| c.tips).unwrap_or(true);
    if enabled {
        log::info!("cli:tip: {msg}");
        if std::env::var_os("NO_COLOR").is_none() {
            eprintln!("\x1b[2mtip: {msg}\x1b[0m");
        } else {
            eprintln!("tip: {msg}");
        }
    }
}

#[cfg(test)]
mod socket_resolution_tests {
    use super::{command_socket_available_from, resolve_command_socket_from};
    use std::ffi::OsStr;
    use std::path::{Path, PathBuf};

    #[test]
    fn pr_binary_uses_own_channel_socket_when_ambient_socket_mismatches() {
        let actual = resolve_command_socket_from(
            None,
            Some("pr-2384"),
            Some(OsStr::new("/tmp/alpha.sock")),
            Path::new("/Users/test"),
        );

        assert_eq!(
            actual,
            Some(PathBuf::from("/Users/test/.plexi-pr-2384/notify.sock"))
        );
    }

    #[test]
    fn alpha_binary_uses_own_channel_socket_when_ambient_socket_mismatches() {
        let actual = resolve_command_socket_from(
            None,
            Some("alpha"),
            Some(OsStr::new("/tmp/beta.sock")),
            Path::new("/Users/test"),
        );

        assert_eq!(
            actual,
            Some(PathBuf::from("/Users/test/.plexi-alpha/notify.sock"))
        );
    }

    #[test]
    fn beta_binary_uses_own_channel_socket_when_ambient_socket_mismatches() {
        let actual = resolve_command_socket_from(
            None,
            Some("beta"),
            Some(OsStr::new("/tmp/alpha.sock")),
            Path::new("/Users/test"),
        );

        assert_eq!(
            actual,
            Some(PathBuf::from("/Users/test/.plexi-beta/notify.sock"))
        );
    }

    #[test]
    fn bare_binary_preserves_ambient_socket() {
        let actual = resolve_command_socket_from(
            None,
            None,
            Some(OsStr::new("/tmp/ambient.sock")),
            Path::new("/Users/test"),
        );

        assert_eq!(actual, Some(PathBuf::from("/tmp/ambient.sock")));
    }

    #[test]
    fn suffixed_binary_accepts_matching_channel_socket() {
        let actual = resolve_command_socket_from(
            None,
            Some("alpha"),
            Some(OsStr::new("/Users/test/.plexi-alpha/notify.sock")),
            Path::new("/Users/test"),
        );

        assert_eq!(
            actual,
            Some(PathBuf::from("/Users/test/.plexi-alpha/notify.sock"))
        );
    }

    #[test]
    fn explicit_socket_override_wins_for_suffixed_binary() {
        let actual = resolve_command_socket_from(
            Some(Path::new("/tmp/explicit.sock")),
            Some("pr-2384"),
            Some(OsStr::new("/tmp/ambient.sock")),
            Path::new("/Users/test"),
        );

        assert_eq!(actual, Some(PathBuf::from("/tmp/explicit.sock")));
    }

    #[test]
    fn explicit_socket_override_enables_direct_dispatch_without_ambient_socket() {
        assert!(command_socket_available_from(true, false));
    }

    #[test]
    fn binary_suffix_alone_does_not_enable_direct_dispatch() {
        let resolved =
            resolve_command_socket_from(None, Some("alpha"), None, Path::new("/Users/test"));

        assert_eq!(
            (resolved, command_socket_available_from(false, false)),
            (
                Some(PathBuf::from("/Users/test/.plexi-alpha/notify.sock")),
                false,
            )
        );
    }
}

fn resolve_command_socket_from(
    explicit_socket: Option<&std::path::Path>,
    binary_channel: Option<&str>,
    ambient_socket: Option<&std::ffi::OsStr>,
    home_dir: &std::path::Path,
) -> Option<std::path::PathBuf> {
    if let Some(socket) = explicit_socket {
        return Some(socket.to_path_buf());
    }
    if let Some(channel) = binary_channel {
        return Some(
            home_dir
                .join(format!(".plexi-{channel}"))
                .join("notify.sock"),
        );
    }
    ambient_socket.map(std::path::PathBuf::from)
}

static COMMAND_SOCKET_OVERRIDE: std::sync::OnceLock<std::path::PathBuf> =
    std::sync::OnceLock::new();

pub fn set_command_socket_override(path: std::path::PathBuf) {
    if COMMAND_SOCKET_OVERRIDE.set(path.clone()).is_err() {
        log::warn!("cli: command socket override was already set; keeping the first path");
    } else {
        log::info!("cli: set explicit command socket override path={path:?}");
    }
}

fn command_socket_available_from(explicit_override: bool, ambient_socket: bool) -> bool {
    explicit_override || ambient_socket
}

pub(super) fn command_socket_available() -> bool {
    command_socket_available_from(
        COMMAND_SOCKET_OVERRIDE.get().is_some(),
        std::env::var_os("PLEXI_SOCKET").is_some(),
    )
}

/// Poll an RPC response file with the standard CLI error reporting: timeout
/// and read failures print `error: ...` to stderr and yield exit code 1.
/// `label` names the command in the timeout message.
pub(super) fn poll_rpc(response_file: &str, label: &str) -> Result<String, i32> {
    poll_rpc_with(response_file, label, crate::rpc::DEFAULT_TIMEOUT)
}

pub(super) fn poll_rpc_with(
    response_file: &str,
    label: &str,
    timeout: std::time::Duration,
) -> Result<String, i32> {
    match crate::rpc::poll_string(response_file, Some(timeout)) {
        Ok(content) => Ok(content),
        Err(crate::rpc::PollError::TimedOut) => {
            eprintln!("error: timed out waiting for {label} response");
            Err(1)
        }
        Err(e) => {
            log::warn!("{label}:cli: {e}");
            eprintln!("error: {e}");
            Err(1)
        }
    }
}

/// Spawn-queue writer gate (stint 0532): outside a Plexi pane, a spawn
/// request may only be queued when this channel's host is actually accepting
/// IPC on its notify socket. A file queued into the void fires on some later
/// host boot as a window nothing attributes — an implicit desktop escape.
/// Fail loudly and typed instead. `plexi host start` is exempt by design: it
/// seeds the queue explicitly and launches the draining host itself in the
/// same command.
pub(super) fn require_spawn_servicing_host(intent: &str) -> Result<(), i32> {
    let socket = crate::config::config_dir().join("notify.sock");
    if spawn_socket_accepting(&socket) {
        return Ok(());
    }
    let channel = channel_label_from(
        crate::config::config_dir()
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(""),
    );
    log::warn!(
        "spawn_gate: refused '{intent}' — host not servicing pane spawns on channel {channel} (socket {socket:?})"
    );
    eprintln!("{}", spawn_gate_error(&channel, &socket));
    print_tip("start a host for this channel with `plexi host start`, or run this from inside a Plexi pane");
    Err(1)
}

fn spawn_socket_accepting(socket: &std::path::Path) -> bool {
    std::os::unix::net::UnixStream::connect(socket).is_ok()
}

/// Human channel name from a profile dir basename: `.plexi-alpha` → `alpha`,
/// bare `.plexi` → `default`.
fn channel_label_from(profile_dir_basename: &str) -> String {
    match profile_dir_basename.strip_prefix(".plexi-") {
        Some(ch) if !ch.is_empty() => ch.to_string(),
        _ => "default".to_string(),
    }
}

fn spawn_gate_error(channel: &str, socket: &std::path::Path) -> String {
    format!(
        "error: host not servicing pane spawns on channel {channel} (no running host at {}); nothing was spawned",
        socket.display()
    )
}

/// Unix-epoch milliseconds stamp for spawn-queue file attribution.
pub(super) fn spawn_queued_at_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn command_binary_channel() -> Option<String> {
    let channel = crate::config::build_channel();
    #[cfg(test)]
    {
        let is_cargo_test_artifact = std::env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(|parent| parent.ends_with("deps")))
            .unwrap_or(false);
        if is_cargo_test_artifact {
            return None;
        }
    }
    channel
}

pub(super) fn resolve_command_socket() -> Option<std::path::PathBuf> {
    let channel = command_binary_channel();
    let ambient = std::env::var_os("PLEXI_SOCKET");
    let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    let explicit = COMMAND_SOCKET_OVERRIDE.get();
    let socket = resolve_command_socket_from(
        explicit.map(std::path::PathBuf::as_path),
        channel.as_deref(),
        ambient.as_deref(),
        &home,
    );
    if let Some(path) = socket.as_ref() {
        let source = if explicit.is_some() {
            "--socket"
        } else if channel.is_some() {
            "binary-channel"
        } else {
            "PLEXI_SOCKET"
        };
        log::info!(
            "cli: resolved command socket source={source} binary_channel={channel:?} path={path:?}"
        );
    }
    socket
}

const SOCKET_TRANSPORT_DEADLINE: std::time::Duration = std::time::Duration::from_secs(2);

#[derive(Debug)]
enum SocketTransportError {
    ConnectTimeout,
    Connect(std::io::Error),
    WriteTimeout {
        bytes_written: usize,
    },
    Write {
        source: std::io::Error,
        bytes_written: usize,
    },
}

fn wait_for_socket(
    fd: std::os::fd::RawFd,
    events: libc::c_short,
    deadline: std::time::Instant,
) -> std::io::Result<bool> {
    loop {
        let now = std::time::Instant::now();
        if now >= deadline {
            return Ok(false);
        }
        let remaining = deadline.saturating_duration_since(now);
        let timeout_ms = remaining.as_millis().max(1).min(i32::MAX as u128) as i32;
        let mut poll_fd = libc::pollfd {
            fd,
            events,
            revents: 0,
        };
        let rc = unsafe { libc::poll(&mut poll_fd, 1, timeout_ms) };
        if rc > 0 {
            return Ok(true);
        }
        if rc == 0 {
            return Ok(false);
        }
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}

fn classify_connect_wait(result: std::io::Result<bool>) -> Result<(), SocketTransportError> {
    match result {
        Ok(true) => Ok(()),
        Ok(false) => Err(SocketTransportError::ConnectTimeout),
        Err(error) => Err(SocketTransportError::Connect(error)),
    }
}

fn connect_unix_deadline(
    socket_path: &std::path::Path,
    deadline: std::time::Instant,
) -> Result<std::os::fd::OwnedFd, SocketTransportError> {
    use std::os::fd::FromRawFd as _;
    use std::os::unix::ffi::OsStrExt as _;

    let path = socket_path.as_os_str().as_bytes();
    let mut addr: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    if path.len() >= addr.sun_path.len() {
        return Err(SocketTransportError::Connect(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Unix socket path is too long",
        )));
    }
    addr.sun_family = libc::AF_UNIX as libc::sa_family_t;
    #[cfg(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly"
    ))]
    {
        addr.sun_len = (std::mem::offset_of!(libc::sockaddr_un, sun_path) + path.len() + 1) as u8;
    }
    for (dst, src) in addr.sun_path.iter_mut().zip(path.iter().copied()) {
        *dst = src as libc::c_char;
    }

    let raw_fd = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0) };
    if raw_fd < 0 {
        return Err(SocketTransportError::Connect(
            std::io::Error::last_os_error(),
        ));
    }
    let fd = unsafe { std::os::fd::OwnedFd::from_raw_fd(raw_fd) };
    let flags = unsafe { libc::fcntl(raw_fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(raw_fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(SocketTransportError::Connect(
            std::io::Error::last_os_error(),
        ));
    }

    let addr_len =
        (std::mem::offset_of!(libc::sockaddr_un, sun_path) + path.len() + 1) as libc::socklen_t;
    let rc = unsafe {
        libc::connect(
            raw_fd,
            &addr as *const libc::sockaddr_un as *const libc::sockaddr,
            addr_len,
        )
    };
    if rc == 0 {
        return Ok(fd);
    }
    let error = std::io::Error::last_os_error();
    if !error.raw_os_error().is_some_and(|code| {
        code == libc::EINPROGRESS || code == libc::EAGAIN || code == libc::EWOULDBLOCK
    }) {
        return Err(SocketTransportError::Connect(error));
    }
    classify_connect_wait(wait_for_socket(raw_fd, libc::POLLOUT, deadline))?;
    let mut socket_error: libc::c_int = 0;
    let mut socket_error_len = std::mem::size_of_val(&socket_error) as libc::socklen_t;
    let rc = unsafe {
        libc::getsockopt(
            raw_fd,
            libc::SOL_SOCKET,
            libc::SO_ERROR,
            &mut socket_error as *mut _ as *mut libc::c_void,
            &mut socket_error_len,
        )
    };
    if rc < 0 {
        return Err(SocketTransportError::Connect(
            std::io::Error::last_os_error(),
        ));
    }
    if socket_error != 0 {
        return Err(SocketTransportError::Connect(
            std::io::Error::from_raw_os_error(socket_error),
        ));
    }
    Ok(fd)
}

fn send_line_to_socket(
    socket_path: &std::path::Path,
    line: &[u8],
    timeout: std::time::Duration,
) -> Result<(), SocketTransportError> {
    use std::os::fd::AsRawFd as _;

    let deadline = std::time::Instant::now() + timeout;
    let fd = connect_unix_deadline(socket_path, deadline)?;
    let raw_fd = fd.as_raw_fd();
    let mut written = 0;
    while written < line.len() {
        if std::time::Instant::now() >= deadline {
            return Err(SocketTransportError::WriteTimeout {
                bytes_written: written,
            });
        }
        let rc = unsafe {
            libc::write(
                raw_fd,
                line[written..].as_ptr() as *const libc::c_void,
                line.len() - written,
            )
        };
        if rc > 0 {
            written += rc as usize;
            continue;
        }
        if rc == 0 {
            return Err(SocketTransportError::Write {
                source: std::io::Error::new(
                    std::io::ErrorKind::WriteZero,
                    "socket accepted zero bytes",
                ),
                bytes_written: written,
            });
        }
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::Interrupted {
            continue;
        }
        if error
            .raw_os_error()
            .is_some_and(|code| code == libc::EAGAIN || code == libc::EWOULDBLOCK)
        {
            match wait_for_socket(raw_fd, libc::POLLOUT, deadline) {
                Ok(true) => continue,
                Ok(false) => {
                    return Err(SocketTransportError::WriteTimeout {
                        bytes_written: written,
                    });
                }
                Err(source) => {
                    return Err(SocketTransportError::Write {
                        source,
                        bytes_written: written,
                    });
                }
            }
        }
        return Err(SocketTransportError::Write {
            source: error,
            bytes_written: written,
        });
    }
    Ok(())
}

pub(super) fn send_to_socket(payload: serde_json::Value) -> i32 {
    let socket_path = match resolve_command_socket() {
        Some(path) => path,
        None => {
            eprintln!("error: PLEXI_SOCKET is not set — run this inside a Plexi terminal pane");
            return 1;
        }
    };
    let line = format!("{}\n", payload);
    let started = std::time::Instant::now();
    match send_line_to_socket(&socket_path, line.as_bytes(), SOCKET_TRANSPORT_DEADLINE) {
        Ok(()) => {
            log::info!(
                "cli: socket transport complete path={socket_path:?} bytes={} elapsed_ms={}",
                line.len(),
                started.elapsed().as_millis()
            );
            0
        }
        Err(SocketTransportError::Connect(e))
            if e.kind() == std::io::ErrorKind::ConnectionRefused =>
        {
            let _ = std::fs::remove_file(&socket_path);
            eprintln!("error: Plexi is not responding (stale socket removed). Is Plexi running?");
            1
        }
        Err(SocketTransportError::ConnectTimeout) => {
            log::warn!(
                "cli: socket connect timed out path={socket_path:?} deadline_ms={}",
                SOCKET_TRANSPORT_DEADLINE.as_millis()
            );
            eprintln!(
                "error: timed out connecting to PLEXI_SOCKET {socket_path:?}; request was not delivered"
            );
            1
        }
        Err(SocketTransportError::Connect(e)) => {
            log::warn!("cli: socket connect failed path={socket_path:?}: {e}");
            eprintln!("error: could not connect to PLEXI_SOCKET {socket_path:?}: {e}");
            1
        }
        Err(SocketTransportError::WriteTimeout { bytes_written }) => {
            log::warn!(
                "cli: socket write timed out path={socket_path:?} bytes_written={bytes_written} payload_bytes={} deadline_ms={}",
                line.len(),
                SOCKET_TRANSPORT_DEADLINE.as_millis()
            );
            if bytes_written == 0 {
                eprintln!(
                    "error: timed out writing request to PLEXI_SOCKET {socket_path:?}; the incomplete frame was not delivered and is safe to retry"
                );
            } else {
                eprintln!(
                    "error: timed out writing request to PLEXI_SOCKET {socket_path:?} after {bytes_written}/{} bytes; the incomplete frame was not delivered and is safe to retry",
                    line.len()
                );
            }
            1
        }
        Err(SocketTransportError::Write {
            source,
            bytes_written,
        }) => {
            log::warn!(
                "cli: socket write failed path={socket_path:?} bytes_written={bytes_written} payload_bytes={}: {source}",
                line.len()
            );
            if bytes_written == 0 {
                eprintln!("error: could not write request to socket: {source}");
            } else {
                eprintln!(
                    "error: socket write failed after {bytes_written}/{} bytes: {source}; the incomplete frame was not delivered and is safe to retry",
                    line.len()
                );
            }
            1
        }
    }
}

#[cfg(test)]
mod transport_deadline_tests {
    use super::{connect_unix_deadline, send_line_to_socket, SocketTransportError};
    use std::io::{BufRead as _, BufReader};
    use std::os::unix::net::UnixListener;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    fn unique_socket_dir(label: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "pxt-{label}-{}-{:x}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
                & 0xffff_ffff
        ));
        std::fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    #[test]
    fn stalled_write_returns_with_phase_and_partial_byte_count() {
        let dir = unique_socket_dir("write");
        let socket = dir.join("notify.sock");
        let listener = UnixListener::bind(&socket).expect("bind");
        let peer = std::thread::spawn(move || {
            let (_stream, _) = listener.accept().expect("accept");
            std::thread::sleep(Duration::from_millis(500));
        });

        let started = Instant::now();
        let line = vec![b'x'; 32 * 1024 * 1024];
        let error = send_line_to_socket(&socket, &line, Duration::from_millis(100))
            .expect_err("stalled write must time out");
        let elapsed = started.elapsed();
        peer.join().expect("peer");

        assert!(matches!(
            error,
            SocketTransportError::WriteTimeout { bytes_written } if bytes_written > 0
        ));
        assert!(
            elapsed < Duration::from_millis(300),
            "write exceeded strict deadline: {elapsed:?}"
        );
    }

    #[test]
    fn saturated_listener_returns_connect_phase_within_deadline() {
        let dir = unique_socket_dir("connect");
        let socket = dir.join("notify.sock");
        let _listener = UnixListener::bind(&socket).expect("bind");
        let mut queued = Vec::new();
        let started = Instant::now();
        let error = loop {
            match connect_unix_deadline(&socket, Instant::now() + Duration::from_millis(25)) {
                Ok(fd) => queued.push(fd),
                Err(error) => break error,
            }
            assert!(queued.len() < 512, "listener backlog did not saturate");
        };
        assert!(matches!(
            error,
            SocketTransportError::ConnectTimeout | SocketTransportError::Connect(_)
        ));
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "connect saturation probe exceeded bound"
        );
    }

    #[test]
    fn connect_poll_expiry_is_typed_as_connect_timeout() {
        let error = super::classify_connect_wait(Ok(false)).expect_err("timeout");
        assert!(matches!(error, SocketTransportError::ConnectTimeout));
    }

    #[test]
    fn normal_listener_receives_exact_payload_once() {
        let dir = unique_socket_dir("exact");
        let socket = dir.join("notify.sock");
        let listener = UnixListener::bind(&socket).expect("bind");
        let handle = std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept");
            let mut line = String::new();
            BufReader::new(stream)
                .read_line(&mut line)
                .expect("read request");
            line
        });
        let payload = b"{\"type\":\"transport_exactly_once\"}\n";
        send_line_to_socket(&socket, payload, Duration::from_secs(1)).expect("send");
        assert_eq!(
            handle.join().expect("listener"),
            String::from_utf8_lossy(payload)
        );
    }
}

/// Best-effort wake of a running instance after a spawn-queue file write.
///
/// A fully idle Plexi produces zero frames and only drains the spawn-queue
/// during a frame, so without this nudge an outside-shell `plexi pane new`
/// hangs until the user touches the window. Connects to the channel
/// profile's `notify.sock` and sends a no-op `AppRequest::Wake`; the socket
/// listener requests a repaint, the resulting frame drains the queue.
///
/// All errors are ignored silently and intentionally: no instance running
/// means the queue is drained at next startup — the designed fallback.
pub(super) fn nudge_running_instance() {
    use std::io::Write;
    use std::os::unix::net::UnixStream;
    let path = crate::config::config_dir().join("notify.sock");
    let Ok(mut stream) = UnixStream::connect(&path) else {
        return;
    };
    let Ok(payload) = serde_json::to_string(&crate::app_protocol::AppRequest::Wake) else {
        return;
    };
    if stream.write_all(format!("{payload}\n").as_bytes()).is_ok() {
        log::debug!("cli: sent wake nudge to running instance at {path:?}");
    }
}

pub(super) fn binary_in_path(name: &str) -> bool {
    std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).any(|dir| dir.join(name).is_file()))
        .unwrap_or(false)
}

// ── Public re-exports (preserve crate::cli::fn_name() call sites in main.rs) ──
pub use account::{account_login_cli, account_logout_cli, account_status_cli};
pub use agent::{
    agent_add, agent_hook_install_cli, agent_hook_uninstall_cli, agent_init, agent_list,
    agent_report_cli, agent_status_cli, agent_update,
};
pub use ai::{ai_doctor_cli, ai_onboard_cli, ai_setup_cli};
pub use app::{
    app_action_cli, app_info, app_init, app_inspect_cli, app_install_package, app_install_with_pin,
    app_list, app_package_cli, app_prune_cli, app_render, app_test_cli, app_uninstall,
    app_update_cli, InstallConfirm,
};
pub use app_check::app_check_cli;
pub use completions::{complete_open_cli, complete_run_cli, completions_cli};
pub use config_cli::{
    config_check, config_edit, config_get, config_list, config_reset, config_set,
};
pub use context_cli::{
    context_current_cli, context_describe_cli, context_list_cli, context_new_cli, context_open_cli,
    context_push_cli, context_set_root_cli, context_sub_cli, context_zoom_cli,
    context_zoom_out_cli,
};
pub use demo::demo_cli;
pub use doctor::doctor_cli;
pub use events::{
    events_declare_cli, events_emit_cli, events_list_cli, events_mcp_config_cli,
    events_subscribe_cli, EmitArgs,
};
pub use host::{host_log_cli, host_screenshot_cli, host_start_cli, host_status_cli, host_stop_cli};
pub use install::{
    install_cli, install_pack_cli, install_workspace_pack_cli, plexi_uninstall_cli,
    self_update_cli, update_cli,
};
pub use list::{freeze_cli, parse_notify_choice};
pub use marketplace::{app_browse_cli, app_publish_cli, app_search_cli};
pub use notes::{notes_list_cli, notes_open_cli};
pub use notify::{dismiss_notify_cli, notify_cli, parse_notify_scope};
pub use open::{
    app_trust_cli, mcp_pane_title, open_cli, open_cli_by_name, open_mcp_by_name, pane_new_cli,
    parse_prefix, AgentBootRequest, OpenPrefix,
};
pub use pane::{
    pane_capture_cli, pane_click_cli, pane_click_node_cli, pane_close_cli, pane_drag_cli,
    pane_drop_cli, pane_focus_cli, pane_heartbeat_cli, pane_info_cli, pane_key_cli, pane_list_cli,
    pane_self_cli, pane_send_cli, pane_set_title_cli, pane_slot_delete_cli, pane_slot_list_cli,
    pane_slot_read_cli, pane_slot_wait_cli, pane_slot_write_cli, pane_state_cli, pane_status_cli,
};
pub use routine::{routine_add, routine_list, routine_remove, routine_run, routine_set_enabled};
pub use run::{run_command, run_list_commands};
pub use validate::validate_cli;
pub use workspace::{
    workspace_clean_cli, workspace_init, workspace_secret_delete, workspace_secret_get,
    workspace_secret_list, workspace_secret_set,
};

#[cfg(test)]
mod resolve_user_commands_tests {
    use super::{resolve_user_commands, UserCommandScope};

    /// Build a workspace tempdir whose channel-scoped commands.toml holds `body`.
    fn workspace_with_commands(body: &str) -> (tempfile::TempDir, crate::config::TestChannelGuard) {
        let guard = crate::config::set_test_channel("test");
        let tmp = tempfile::tempdir().unwrap();
        let chan_dir = tmp.path().join(".plexi-test");
        std::fs::create_dir_all(&chan_dir).unwrap();
        std::fs::write(chan_dir.join("commands.toml"), body).unwrap();
        (tmp, guard)
    }

    #[test]
    fn workspace_commands_resolve_sorted_with_scope() {
        let (tmp, _g) = workspace_with_commands(
            r#"
[commands]
zeta = "echo z"
build = { run = "cargo build", description = "Build it" }
"#,
        );
        let cmds = resolve_user_commands(Some(tmp.path()));
        assert_eq!(cmds.len(), 2);
        assert_eq!(cmds[0].name, "build");
        assert_eq!(cmds[0].description.as_deref(), Some("Build it"));
        assert_eq!(cmds[0].run.as_deref(), Some("cargo build"));
        assert_eq!(cmds[0].scope, UserCommandScope::Workspace);
        assert_eq!(cmds[1].name, "zeta");
    }

    #[test]
    fn workspace_commands_shadow_global_scripts() {
        // commands.toml present → global scripts must NOT appear (mirrors `plexi run`).
        let (tmp, _g) = workspace_with_commands("[commands]\nonly = \"echo hi\"\n");
        let profile = tempfile::tempdir().unwrap();
        let scripts = profile.path().join("scripts");
        std::fs::create_dir_all(&scripts).unwrap();
        write_executable(&scripts.join("global-only"));
        let _pg = crate::config::set_test_profile_dir(profile.path().to_path_buf());

        let cmds = resolve_user_commands(Some(tmp.path()));
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].name, "only");
        assert_eq!(cmds[0].scope, UserCommandScope::Workspace);
    }

    #[test]
    fn hot_reload_rereads_file_each_call() {
        let (tmp, _g) = workspace_with_commands("[commands]\nfirst = \"echo 1\"\n");
        let first = resolve_user_commands(Some(tmp.path()));
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].name, "first");

        std::fs::write(
            tmp.path().join(".plexi-test").join("commands.toml"),
            "[commands]\nfirst = \"echo 1\"\nsecond = \"echo 2\"\n",
        )
        .unwrap();
        let second = resolve_user_commands(Some(tmp.path()));
        assert_eq!(second.len(), 2);
        assert!(second.iter().any(|c| c.name == "second"));
    }

    #[test]
    fn global_scripts_when_no_commands_toml() {
        let _g = crate::config::set_test_channel("test");
        let tmp = tempfile::tempdir().unwrap(); // workspace root with no commands.toml
        let profile = tempfile::tempdir().unwrap();
        let scripts = profile.path().join("scripts");
        std::fs::create_dir_all(&scripts).unwrap();
        write_executable(&scripts.join("deploy"));
        let _pg = crate::config::set_test_profile_dir(profile.path().to_path_buf());

        let cmds = resolve_user_commands(Some(tmp.path()));
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].name, "deploy");
        assert_eq!(cmds[0].scope, UserCommandScope::Global);
        assert!(cmds[0].run.is_none());
    }

    #[test]
    fn unparseable_commands_toml_yields_no_commands() {
        // Present-but-broken commands.toml → empty, never silent global fallback.
        let (tmp, _g) = workspace_with_commands("this is = not valid toml [[[\n");
        let profile = tempfile::tempdir().unwrap();
        let scripts = profile.path().join("scripts");
        std::fs::create_dir_all(&scripts).unwrap();
        write_executable(&scripts.join("should-not-appear"));
        let _pg = crate::config::set_test_profile_dir(profile.path().to_path_buf());

        let cmds = resolve_user_commands(Some(tmp.path()));
        assert!(cmds.is_empty());
    }

    fn write_executable(path: &std::path::Path) {
        std::fs::write(path, "#!/bin/sh\necho hi\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(path).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(path, perms).unwrap();
        }
    }
}

#[cfg(test)]
mod spawn_gate_tests {
    use super::{channel_label_from, spawn_gate_error, spawn_socket_accepting};

    #[test]
    fn missing_socket_is_not_servicing() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(!spawn_socket_accepting(&tmp.path().join("notify.sock")));
    }

    #[test]
    fn bound_socket_is_servicing() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("notify.sock");
        let _listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        assert!(spawn_socket_accepting(&path));
    }

    #[test]
    fn gate_error_is_typed_and_names_channel_and_socket() {
        let msg = spawn_gate_error("alpha", std::path::Path::new("/tmp/x/notify.sock"));
        assert!(msg.contains("host not servicing pane spawns on channel alpha"));
        assert!(msg.contains("/tmp/x/notify.sock"));
        assert!(msg.contains("nothing was spawned"));
    }

    #[test]
    fn channel_label_derivation() {
        assert_eq!(channel_label_from(".plexi-alpha"), "alpha");
        assert_eq!(channel_label_from(".plexi-pr-2470"), "pr-2470");
        assert_eq!(channel_label_from(".plexi"), "default");
        assert_eq!(channel_label_from(""), "default");
    }
}

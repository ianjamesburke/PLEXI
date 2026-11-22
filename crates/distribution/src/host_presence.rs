//! Whether a host is running on a profile, observed from outside (stint 1605).
//!
//! Two facts a host publishes answer it without running any build. Every host
//! since the host-instance guard holds an exclusive lock on [`LOCK_FILE`] in
//! its profile for its whole life and writes its pid there; the operating
//! system drops the lock on exit or crash. Every host, including ones from
//! before that guard, listens on the channel endpoint ([`endpoint_in`]).
//! [`probe`] reads both and changes nothing: it never creates the profile,
//! the lock file or the endpoint, and never removes a stale endpoint.

use std::fs::{File, TryLockError};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// The host-instance lock file, in the profile directory.
pub const LOCK_FILE: &str = "host.lock";

/// `.plexi` for the stable channel, `.plexi-<channel>` for every other.
pub fn profile_dir_name(channel: &str) -> String {
    match channel {
        "stable" | "main" => ".plexi".to_string(),
        other => format!(".plexi-{other}"),
    }
}

/// The endpoint belonging to a specific profile, named by its profile
/// directory (`~/.plexi`, `~/.plexi-alpha`, `/scratch/.plexi-pr-1604`, ...).
///
/// Commands that target another profile (`plexi host stop`, the spawn gate,
/// `--socket` resolution, the installer) know the profile dir, not the
/// endpoint. On Unix the endpoint lives inside that dir; on Windows it does
/// not exist on disk at all, so the pipe name is derived from the whole dir
/// ([`pipe_name_in`]). Keeping both derivations here is what stops the two
/// from drifting.
pub fn endpoint_in(profile_dir: &Path) -> PathBuf {
    #[cfg(unix)]
    {
        profile_dir.join("notify.sock")
    }
    #[cfg(windows)]
    {
        let (canonical, account_rooted) = crate::profile_target::profile_identity(profile_dir);
        PathBuf::from(pipe_name_in(&canonical, account_rooted))
    }
}

/// The Windows pipe prefix: `\\.\pipe\plexi`.
pub const PIPE_PREFIX: &str = r"\\.\pipe\plexi";

/// Separates a scratch-root pipe from the account namespace. A profile name
/// cannot contain it, so the two namespaces are disjoint by grammar.
pub const SCRATCH_PIPE_SEPARATOR: char = '.';

/// The Windows pipe for a profile (stint 1695). An account-rooted profile
/// keeps `\\.\pipe\plexi[-<name>]`, so old hosts and clients still meet. Any
/// other root is `\\.\pipe\plexi.<32 hex of sha256(canonical dir)>`: no name
/// is embedded and nothing parses it back. Pure, so every platform tests it.
pub fn pipe_name_in(canonical_dir: &Path, account_rooted: bool) -> String {
    if !account_rooted {
        use sha2::Digest;
        let digest = sha2::Sha256::digest(canonical_dir.to_string_lossy().as_bytes());
        let hex: String = digest.iter().take(16).map(|b| format!("{b:02x}")).collect();
        return format!("{PIPE_PREFIX}{SCRATCH_PIPE_SEPARATOR}{hex}");
    }
    match canonical_dir
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_prefix(".plexi-"))
        .filter(|suffix| !suffix.is_empty())
    {
        Some(suffix) => format!("{PIPE_PREFIX}-{suffix}"),
        None => PIPE_PREFIX.to_string(),
    }
}

/// The profile's spawn-queue dir, which its host drains on its first frame.
pub const SPAWN_QUEUE_DIR: &str = "spawn-queue";

/// Refuse a profile dir (or its `host` or spawn-queue dir) that is a symlink
/// or not a dir, and a host endpoint that is a symlink or has more than one
/// link: resolving a host socket, seed, lock, installed marker or integrity
/// account through one would reach another profile. Shared by the CLI
/// lifecycle verbs and the installer. A missing entry is fine.
pub fn ensure_real_profile_dir(profile: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let endpoint = endpoint_in(profile);
        match std::fs::symlink_metadata(&endpoint) {
            // A hard link names another profile's listening socket too.
            Ok(meta) if meta.file_type().is_symlink() || meta.nlink() != 1 => {
                log::error!(
                    "host_presence: refusing linked host endpoint {endpoint:?} (links={})",
                    meta.nlink()
                );
                return Err(format!(
                    "refusing to address a host through linked host endpoint {}",
                    endpoint.display()
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("inspect {}: {error}", endpoint.display())),
        }
    }
    for path in [
        profile.to_path_buf(),
        profile.join("host"),
        profile.join(SPAWN_QUEUE_DIR),
    ] {
        match std::fs::symlink_metadata(&path) {
            Ok(meta) if meta.file_type().is_symlink() || !meta.is_dir() => {
                log::error!("host_presence: refusing profile through non-directory {path:?}");
                return Err(format!(
                    "refusing to address a host through non-directory profile path {}",
                    path.display()
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("inspect {}: {error}", path.display())),
        }
    }
    Ok(())
}

/// What showed a host to be running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Evidence {
    /// It holds the host-instance lock.
    Lock,
    /// It listens on the channel endpoint (a host from before the lock).
    Endpoint,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Presence {
    Running {
        pid: Option<u32>,
        evidence: Evidence,
    },
    NotRunning,
    /// The probe could not tell; the reason names what failed.
    Unknown(String),
}

/// Whether a host is running on `profile_dir`. Read-only; the endpoint
/// connect is abandoned at `deadline`, and a connect still pending then
/// counts as running because only a live listener leaves one pending.
pub fn probe(profile_dir: &Path, deadline: Instant) -> Presence {
    let lock = profile_dir.join(LOCK_FILE);
    match File::open(&lock) {
        Ok(mut file) => match file.try_lock_shared() {
            Err(TryLockError::WouldBlock) => {
                // Unix advisory locks allow the read; on Windows the locked
                // range is unreadable and the pid stays unknown.
                let mut text = String::new();
                let pid = file
                    .read_to_string(&mut text)
                    .ok()
                    .and_then(|_| text.trim().parse().ok());
                return Presence::Running {
                    pid,
                    evidence: Evidence::Lock,
                };
            }
            Ok(()) => drop(file),
            Err(TryLockError::Error(error)) => {
                return Presence::Unknown(format!("lock {}: {error}", lock.display()));
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Presence::Unknown(format!("open {}: {error}", lock.display())),
    }
    endpoint_presence(&endpoint_in(profile_dir), deadline)
}

fn running_on_endpoint() -> Presence {
    Presence::Running {
        pid: None,
        evidence: Evidence::Endpoint,
    }
}

/// Probe only the endpoint, for callers that already hold the host lock.
#[cfg(unix)]
pub fn endpoint_presence(endpoint: &Path, deadline: Instant) -> Presence {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::ffi::OsStrExt;

    let unknown = |what: &str, error: std::io::Error| {
        Presence::Unknown(format!("{what} {}: {error}", endpoint.display()))
    };
    let path = endpoint.as_os_str().as_bytes();
    // SAFETY: sockaddr_un is plain data; all-zero is a valid value.
    let mut addr: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    if path.len() >= addr.sun_path.len() {
        return Presence::Unknown(format!("endpoint path too long: {}", endpoint.display()));
    }
    addr.sun_family = libc::AF_UNIX as libc::sa_family_t;
    let addr_len = std::mem::offset_of!(libc::sockaddr_un, sun_path) + path.len() + 1;
    #[cfg(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly"
    ))]
    {
        addr.sun_len = addr_len as u8;
    }
    for (dst, src) in addr.sun_path.iter_mut().zip(path.iter().copied()) {
        *dst = src as libc::c_char;
    }
    // SAFETY: plain socket(2); the result is checked before use.
    let raw = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0) };
    if raw < 0 {
        return unknown("socket for", std::io::Error::last_os_error());
    }
    // SAFETY: raw is a fresh descriptor this function owns.
    let fd = unsafe { OwnedFd::from_raw_fd(raw) };
    // SAFETY: fcntl on an owned descriptor.
    let flags = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFL) };
    // SAFETY: as above.
    if flags < 0
        || unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
    {
        return unknown("configure socket for", std::io::Error::last_os_error());
    }
    // SAFETY: addr is initialized and addr_len covers the path and its NUL.
    let rc = unsafe {
        libc::connect(
            fd.as_raw_fd(),
            (&raw const addr).cast::<libc::sockaddr>(),
            addr_len as libc::socklen_t,
        )
    };
    if rc == 0 {
        return running_on_endpoint();
    }
    let error = std::io::Error::last_os_error();
    match error.raw_os_error() {
        Some(libc::ENOENT | libc::ECONNREFUSED) => Presence::NotRunning,
        // A full accept queue (unix(7)): only a live listener has one.
        Some(libc::EAGAIN) => running_on_endpoint(),
        Some(libc::EINPROGRESS) => {
            let mut poll = libc::pollfd {
                fd: fd.as_raw_fd(),
                events: libc::POLLOUT,
                revents: 0,
            };
            loop {
                let wait = deadline.saturating_duration_since(Instant::now());
                let wait_ms = libc::c_int::try_from(wait.as_millis()).unwrap_or(libc::c_int::MAX);
                // SAFETY: one valid pollfd.
                let ready = unsafe { libc::poll(&mut poll, 1, wait_ms) };
                if ready > 0 {
                    break;
                }
                // Still pending at the deadline: only a live listener does that.
                if ready == 0 {
                    return running_on_endpoint();
                }
                let error = std::io::Error::last_os_error();
                if error.raw_os_error() == Some(libc::EINTR) && Instant::now() < deadline {
                    continue;
                }
                if error.raw_os_error() == Some(libc::EINTR) {
                    return running_on_endpoint();
                }
                return unknown("poll connect to", error);
            }
            let mut so_error: libc::c_int = 0;
            let mut len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
            // SAFETY: so_error and len are valid out-parameters.
            let rc = unsafe {
                libc::getsockopt(
                    fd.as_raw_fd(),
                    libc::SOL_SOCKET,
                    libc::SO_ERROR,
                    (&raw mut so_error).cast(),
                    &mut len,
                )
            };
            match (rc, so_error) {
                (0, 0) => running_on_endpoint(),
                (0, libc::ENOENT | libc::ECONNREFUSED) => Presence::NotRunning,
                (0, code) => unknown("connect to", std::io::Error::from_raw_os_error(code)),
                _ => unknown("connect to", std::io::Error::last_os_error()),
            }
        }
        _ => unknown("connect to", error),
    }
}

/// Probe only the endpoint, for callers that already hold the host lock.
#[cfg(windows)]
pub fn endpoint_presence(endpoint: &Path, _deadline: Instant) -> Presence {
    const ERROR_PIPE_BUSY: i32 = 231;
    match std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(endpoint)
    {
        Ok(_) => running_on_endpoint(),
        Err(error) if error.raw_os_error() == Some(ERROR_PIPE_BUSY) => running_on_endpoint(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Presence::NotRunning,
        Err(error) => Presence::Unknown(format!("open {}: {error}", endpoint.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn soon() -> Instant {
        Instant::now() + Duration::from_secs(2)
    }

    #[test]
    fn profile_dir_names_follow_the_channel() {
        assert_eq!(profile_dir_name("beta"), ".plexi-beta");
        assert_eq!(profile_dir_name("main"), ".plexi");
        assert_eq!(profile_dir_name("stable"), ".plexi");
        assert_eq!(profile_dir_name("pr-12"), ".plexi-pr-12");
    }

    #[test]
    fn a_missing_profile_is_not_running_and_stays_missing() {
        let home = tempfile::tempdir().unwrap();
        let profile = home.path().join(".plexi-beta");
        assert_eq!(probe(&profile, soon()), Presence::NotRunning);
        assert!(!profile.exists());
    }

    #[test]
    fn a_held_lock_is_running_with_its_pid() {
        let profile = tempfile::tempdir().unwrap();
        let path = profile.path().join(LOCK_FILE);
        std::fs::write(&path, "4242\n").unwrap();
        let held = File::options().read(true).write(true).open(&path).unwrap();
        held.try_lock().unwrap();
        let expected_pid = if cfg!(windows) { None } else { Some(4242) };
        assert_eq!(
            probe(profile.path(), soon()),
            Presence::Running {
                pid: expected_pid,
                evidence: Evidence::Lock
            }
        );
        // Not asserting the release here: a child forked by a concurrent test
        // can briefly share this descriptor, and with it the flock.
        drop(held);
    }

    #[cfg(unix)]
    #[test]
    fn a_listener_without_the_lock_is_running() {
        let profile = tempfile::tempdir().unwrap();
        std::fs::write(profile.path().join(LOCK_FILE), "").unwrap();
        let _listener =
            std::os::unix::net::UnixListener::bind(endpoint_in(profile.path())).unwrap();
        assert_eq!(
            probe(profile.path(), soon()),
            Presence::Running {
                pid: None,
                evidence: Evidence::Endpoint
            }
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_stale_endpoint_is_not_running_and_is_kept() {
        let profile = tempfile::tempdir().unwrap();
        let endpoint = endpoint_in(profile.path());
        drop(std::os::unix::net::UnixListener::bind(&endpoint).unwrap());
        assert_eq!(probe(profile.path(), soon()), Presence::NotRunning);
        assert!(endpoint.exists(), "the probe removed a stale endpoint");
    }

    #[cfg(unix)]
    #[test]
    fn an_unreadable_lock_or_endpoint_is_unknown_and_named() {
        use std::os::unix::fs::PermissionsExt;
        let mode = |path: &Path, mode| {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap()
        };
        let profile = tempfile::tempdir().unwrap();
        let lock = profile.path().join(LOCK_FILE);
        std::fs::write(&lock, "").unwrap();
        mode(&lock, 0o000);
        let answer = probe(profile.path(), soon());
        mode(&lock, 0o644);
        assert!(
            matches!(&answer, Presence::Unknown(reason) if reason.contains(LOCK_FILE)),
            "{answer:?}"
        );

        let endpoint = endpoint_in(profile.path());
        let _listener = std::os::unix::net::UnixListener::bind(&endpoint).unwrap();
        mode(&endpoint, 0o000);
        let answer = probe(profile.path(), soon());
        mode(&endpoint, 0o755);
        assert!(
            matches!(&answer, Presence::Unknown(reason) if reason.contains("notify.sock")),
            "{answer:?}"
        );
    }

    /// Design 1695 (test 2): the account and scratch pipe namespaces are
    /// disjoint for every valid name, the adversarial digest-shaped ones
    /// included, and one name in two roots gets two pipes.
    #[test]
    fn account_and_scratch_pipes_never_share_a_name() {
        let names = ["", "-alpha", "-x-r0123456789abcdef", "-r0123456789abcdef"];
        for name in names {
            let dir = format!(".plexi{name}");
            let account = pipe_name_in(&Path::new("/home/ian").join(&dir), true);
            let rest = account.strip_prefix(PIPE_PREFIX).unwrap();
            assert!(!rest.contains(SCRATCH_PIPE_SEPARATOR), "{account}");
            assert_eq!(rest, name);
            let a = pipe_name_in(&Path::new("/scratch/a").join(&dir), false);
            let b = pipe_name_in(&Path::new("/scratch/b").join(&dir), false);
            assert_ne!(a, b);
            assert_ne!(a, account);
            let digest = a
                .strip_prefix(PIPE_PREFIX)
                .and_then(|rest| rest.strip_prefix(SCRATCH_PIPE_SEPARATOR))
                .unwrap();
            assert_eq!(digest.len(), 32, "{a}");
            assert!(digest.bytes().all(|b| b.is_ascii_hexdigit()), "{a}");
            assert_eq!(a, pipe_name_in(&Path::new("/scratch/a").join(&dir), false));
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_linked_profile_dir_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let owner = root.path().join("owner/.plexi-alpha");
        std::fs::create_dir_all(&owner).unwrap();
        let linked = root.path().join("scratch/.plexi-alpha");
        std::fs::create_dir_all(linked.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&owner, &linked).unwrap();
        let error = ensure_real_profile_dir(&linked).unwrap_err();
        assert!(error.contains("non-directory profile path"), "{error}");
        assert!(ensure_real_profile_dir(&owner).is_ok());
        assert!(ensure_real_profile_dir(&root.path().join("missing")).is_ok());
    }
}

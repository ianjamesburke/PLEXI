//! One host per profile (stint 1459). A host holds an exclusive lock on
//! `host.lock` in its profile for its whole life; the operating system drops
//! the lock when the process exits, so a crashed host leaves nothing to clean
//! up and the installer's update restart (which waits for the old pid to exit
//! before relaunching) hands over without help. A second launch on a profile
//! with a live host is refused before it opens a window or touches the
//! socket, whichever path launched it.
use std::fs::{File, OpenOptions, TryLockError};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The lock file, in the profile directory.
pub(crate) const LOCK_FILE: &str = "host.lock";

/// Exit status of a launch refused because a host already runs.
pub(crate) const ALREADY_RUNNING_EXIT: i32 = 3;

/// How long the socket probe waits for a pre-guard host to answer.
const PROBE_TIMEOUT: Duration = Duration::from_millis(500);

/// How long a launch waits for a lock that is being released. An exiting
/// host removes its socket just before the process ends, and `plexi host
/// restart` may start the next host as soon as the socket is gone.
const EXIT_GRACE: Duration = Duration::from_secs(3);

/// Proof that this process is the profile's host. Dropping it (or exiting)
/// releases the lock.
pub(crate) struct HostInstance {
    _lock: File,
}

/// Why this launch may not become the profile's host.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// Another host holds the lock; `pid` is what it recorded, if readable.
    LockHeld { lock: PathBuf, pid: Option<u32> },
    /// The lock was free but the socket answered: a host from a build before
    /// this guard is running and owns it.
    SocketLive { endpoint: PathBuf },
    /// The lock file could not be opened or locked at all.
    LockUnavailable(String),
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LockHeld { lock, pid } => {
                let profile = lock.parent().unwrap_or(lock).display();
                match pid {
                    Some(pid) => write!(
                        f,
                        "Plexi is already running on this profile ({profile}, pid {pid}); use that window"
                    ),
                    None => write!(
                        f,
                        "Plexi is already running on this profile ({profile}); use that window"
                    ),
                }
            }
            Self::SocketLive { endpoint } => write!(
                f,
                "Plexi is already running on this profile (its socket {} answers); use that window",
                endpoint.display()
            ),
            Self::LockUnavailable(reason) => {
                write!(f, "cannot check for a running Plexi host: {reason}")
            }
        }
    }
}

/// Become the host for the profile at `profile_dir`, whose socket is
/// `endpoint`, or say why not. Never removes or binds the socket.
pub(crate) fn claim(profile_dir: &Path, endpoint: &Path) -> Result<HostInstance, Refusal> {
    claim_within(profile_dir, endpoint, EXIT_GRACE)
}

fn claim_within(
    profile_dir: &Path,
    endpoint: &Path,
    grace: Duration,
) -> Result<HostInstance, Refusal> {
    let path = profile_dir.join(LOCK_FILE);
    std::fs::create_dir_all(profile_dir).map_err(|error| {
        Refusal::LockUnavailable(format!("create {}: {error}", profile_dir.display()))
    })?;
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|error| Refusal::LockUnavailable(format!("open {}: {error}", path.display())))?;
    let deadline = std::time::Instant::now() + grace;
    let locked = loop {
        match file.try_lock() {
            Err(TryLockError::WouldBlock) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(50));
            }
            other => break other,
        }
    };
    match locked {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => {
            let pid = std::fs::read_to_string(&path)
                .ok()
                .and_then(|text| text.trim().parse().ok());
            return Err(Refusal::LockHeld { lock: path, pid });
        }
        Err(TryLockError::Error(error)) => {
            return Err(Refusal::LockUnavailable(format!(
                "lock {}: {error}",
                path.display()
            )));
        }
    }
    if matches!(
        crate::cli::host::probe_notify_socket(endpoint, PROBE_TIMEOUT),
        crate::cli::host::RunningState::Running
    ) {
        return Err(Refusal::SocketLive {
            endpoint: endpoint.to_path_buf(),
        });
    }
    // The pid is for the refusal message only; the lock is the guard.
    if let Err(error) = file
        .set_len(0)
        .and_then(|()| writeln!(file, "{}", std::process::id()))
    {
        log::warn!(
            "host_instance: could not record pid in {}: {error}",
            path.display()
        );
    }
    log::info!(
        "host_instance: this process (pid {}) is the host for {}",
        std::process::id(),
        profile_dir.display()
    );
    Ok(HostInstance { _lock: file })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn endpoint_in(dir: &Path) -> PathBuf {
        dir.join("notify.sock")
    }

    /// Stint 1459: a second launch on a profile whose host holds the lock is
    /// refused and never touches the socket.
    #[test]
    fn second_claim_on_a_live_profile_is_refused_and_leaves_the_socket() {
        let profile = tempfile::tempdir().unwrap();
        let endpoint = endpoint_in(profile.path());
        let first = claim(profile.path(), &endpoint).expect("the first launch is the host");
        let listener = crate::platform::ipc::IpcListener::bind(&endpoint).unwrap();

        let refusal = match claim_within(profile.path(), &endpoint, Duration::ZERO) {
            Ok(_) => panic!("a second host must be refused"),
            Err(refusal) => refusal,
        };
        assert_eq!(
            refusal,
            Refusal::LockHeld {
                lock: profile.path().join(LOCK_FILE),
                pid: Some(std::process::id()),
            }
        );
        assert!(refusal
            .to_string()
            .contains("already running on this profile"));
        #[cfg(unix)]
        assert!(
            endpoint.exists(),
            "the live host's socket must not be unlinked"
        );
        drop(listener);
        drop(first);
    }

    /// A host from a build before this guard takes no lock, but its socket
    /// answers: the newcomer is still refused.
    #[test]
    fn a_live_socket_without_the_lock_is_refused() {
        let profile = tempfile::tempdir().unwrap();
        let endpoint = endpoint_in(profile.path());
        let listener = crate::platform::ipc::IpcListener::bind(&endpoint).unwrap();
        let accept = std::thread::spawn(move || {
            let _ = listener.incoming().next();
        });

        let refusal = match claim(profile.path(), &endpoint) {
            Ok(_) => panic!("a live pre-guard host must refuse the newcomer"),
            Err(refusal) => refusal,
        };
        assert_eq!(
            refusal,
            Refusal::SocketLive {
                endpoint: endpoint.clone()
            }
        );
        #[cfg(unix)]
        assert!(
            endpoint.exists(),
            "the live host's socket must not be unlinked"
        );
        accept.join().unwrap();
    }

    /// `plexi host restart` can start the next host while the old process is
    /// still finishing its exit: a lock released within the grace is taken.
    #[test]
    fn a_host_finishing_its_exit_hands_over_within_the_grace() {
        let profile = tempfile::tempdir().unwrap();
        let endpoint = endpoint_in(profile.path());
        let old = claim_within(profile.path(), &endpoint, Duration::ZERO).unwrap();
        let exiting = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            drop(old);
        });
        claim_within(profile.path(), &endpoint, Duration::from_secs(3))
            .expect("the lock is released while the new host waits");
        exiting.join().unwrap();
    }

    /// The update restart waits for the old host to exit; once it has, the
    /// lock is free and the dead host's leftover socket file does not block
    /// the new build.
    #[test]
    fn the_next_host_claims_the_profile_after_the_old_one_exits() {
        let profile = tempfile::tempdir().unwrap();
        let endpoint = endpoint_in(profile.path());
        let old = claim(profile.path(), &endpoint).expect("old build is the host");
        let listener = crate::platform::ipc::IpcListener::bind(&endpoint).unwrap();
        // The old host exits without cleaning up: its socket file stays.
        drop(listener);
        drop(old);

        claim(profile.path(), &endpoint).expect("the restarted build becomes the host");
    }
}

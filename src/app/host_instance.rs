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

/// What a launch is, decided before anything claims the profile.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum HostLaunch {
    /// A CLI command: never a host, never claims.
    Cli,
    /// A bare launch typed inside a Plexi pane (`PLEXI_RUNNING=1`): prints
    /// help instead of starting a second window, and never claims.
    InsidePlexi,
    /// A host launch: claims the profile.
    Host,
}

/// Classify a launch from whether argv starts a host and the value of
/// `PLEXI_RUNNING`.
pub(crate) fn host_launch(cli_mode: bool, plexi_running: Option<&str>) -> HostLaunch {
    if cli_mode {
        HostLaunch::Cli
    } else if plexi_running == Some("1") {
        HostLaunch::InsidePlexi
    } else {
        HostLaunch::Host
    }
}

/// The startup steps around the claim, in the order `main` runs them (stint
/// 1487). `main` supplies the real ones; tests record the calls, so moving the
/// claim after any later step fails a test rather than an installed check.
/// The window and the socket come after this sequence returns.
pub(crate) trait StartupSteps {
    type Instance;
    /// Claim the profile for this host.
    fn claim(&mut self) -> Result<Self::Instance, Refusal>;
    /// Install broker custody (host launches only).
    fn install_custody(&mut self) -> Result<(), String>;
    /// Early trace replay and startup logs.
    fn after_claim(&mut self, host: bool);
    /// Startup history and log retention (`host` false only logs the skip).
    fn retention(&mut self, host: bool);
}

/// Why startup stops in the sequence before the window.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum StartupStop {
    /// A bare launch inside a Plexi pane: print help.
    InsidePlexi,
    /// Another host owns the profile: exit `ALREADY_RUNNING_EXIT`.
    Refused(Refusal),
    /// The check itself failed: the startup-error path.
    LockFailure(Refusal),
    /// Custody could not be installed.
    Custody(String),
}

/// Run the steps up to retention for `launch`. A host launch claims the
/// profile before anything else; a refusal or lock failure returns before any
/// later step runs. `Ok(None)` is a CLI command, which never claims.
pub(crate) fn startup_sequence<S: StartupSteps>(
    launch: HostLaunch,
    steps: &mut S,
) -> Result<Option<S::Instance>, StartupStop> {
    let instance = match launch {
        HostLaunch::Cli => None,
        HostLaunch::InsidePlexi => return Err(StartupStop::InsidePlexi),
        HostLaunch::Host => match steps.claim() {
            Ok(instance) => Some(instance),
            Err(refusal) if refusal.is_another_host() => return Err(StartupStop::Refused(refusal)),
            Err(failure) => return Err(StartupStop::LockFailure(failure)),
        },
    };
    let host = instance.is_some();
    if host {
        steps.install_custody().map_err(StartupStop::Custody)?;
    }
    steps.after_claim(host);
    steps.retention(host);
    Ok(instance)
}

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

impl Refusal {
    /// Another host owns the profile. Otherwise the check itself failed and
    /// must be shown as a startup error, not as a refused second launch.
    pub(crate) fn is_another_host(&self) -> bool {
        !matches!(self, Self::LockUnavailable(_))
    }
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

    /// Records each startup step; `claim` answers with the configured result.
    struct Recorder {
        claim: Option<Refusal>,
        calls: Vec<&'static str>,
    }

    impl Recorder {
        fn new(claim: Option<Refusal>) -> Self {
            Self {
                claim,
                calls: Vec::new(),
            }
        }
    }

    impl StartupSteps for Recorder {
        type Instance = ();
        fn claim(&mut self) -> Result<(), Refusal> {
            self.calls.push("claim");
            self.claim.take().map_or(Ok(()), Err)
        }
        fn install_custody(&mut self) -> Result<(), String> {
            self.calls.push("custody");
            Ok(())
        }
        fn after_claim(&mut self, _host: bool) {
            self.calls.push("after_claim");
        }
        fn retention(&mut self, _host: bool) {
            self.calls.push("retention");
        }
    }

    fn held() -> Refusal {
        Refusal::LockHeld {
            lock: PathBuf::from("/profile/host.lock"),
            pid: Some(7),
        }
    }

    /// Stint 1487 (e): a host launch claims first; the claim moved after
    /// custody or removed fails here.
    #[test]
    fn host_startup_claims_before_every_later_step() {
        let mut steps = Recorder::new(None);
        assert_eq!(startup_sequence(HostLaunch::Host, &mut steps), Ok(Some(())));
        assert_eq!(
            steps.calls,
            ["claim", "custody", "after_claim", "retention"]
        );
    }

    /// A refused claim returns the refusal before custody, retention, and
    /// everything after them (the rpc sweep, the window, the socket).
    #[test]
    fn refused_claim_stops_before_any_later_step() {
        let mut steps = Recorder::new(Some(held()));
        assert_eq!(
            startup_sequence(HostLaunch::Host, &mut steps),
            Err(StartupStop::Refused(held()))
        );
        assert_eq!(steps.calls, ["claim"]);
    }

    #[test]
    fn lock_failure_takes_the_startup_error_path_before_any_later_step() {
        let failure = Refusal::LockUnavailable("open host.lock: denied".into());
        let mut steps = Recorder::new(Some(Refusal::LockUnavailable(
            "open host.lock: denied".into(),
        )));
        assert_eq!(
            startup_sequence(HostLaunch::Host, &mut steps),
            Err(StartupStop::LockFailure(failure))
        );
        assert_eq!(steps.calls, ["claim"]);
    }

    #[test]
    fn cli_and_inside_plexi_never_claim() {
        let mut cli = Recorder::new(None);
        assert_eq!(startup_sequence(HostLaunch::Cli, &mut cli), Ok(None));
        assert_eq!(cli.calls, ["after_claim", "retention"]);
        let mut inside = Recorder::new(None);
        assert_eq!(
            startup_sequence(HostLaunch::InsidePlexi, &mut inside),
            Err(StartupStop::InsidePlexi)
        );
        assert!(inside.calls.is_empty());
    }

    /// Critic on PR #3133: a bare launch typed inside a Plexi pane prints
    /// help as before; it never claims, waits or reports "already running".
    #[test]
    fn a_launch_inside_a_plexi_pane_never_claims_the_profile() {
        assert_eq!(host_launch(false, Some("1")), HostLaunch::InsidePlexi);
        assert_eq!(host_launch(true, Some("1")), HostLaunch::Cli);
        assert_eq!(host_launch(false, None), HostLaunch::Host);
        assert_eq!(host_launch(false, Some("0")), HostLaunch::Host);
    }

    /// A lock that cannot be taken at all is a startup error, not a second
    /// host.
    #[test]
    fn a_lock_failure_is_not_reported_as_another_host() {
        assert!(!Refusal::LockUnavailable("lock host.lock: ENOLCK".into()).is_another_host());
        assert!(Refusal::LockHeld {
            lock: PathBuf::from("/p/host.lock"),
            pid: None
        }
        .is_another_host());
        assert!(Refusal::SocketLive {
            endpoint: PathBuf::from("/p/notify.sock")
        }
        .is_another_host());
    }

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

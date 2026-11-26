//! One host per profile (stint 1459). A host holds an exclusive lock on
//! `host.lock` in its profile for its whole life; the operating system drops
//! the lock when the process exits, so a crashed host leaves nothing to clean
//! up and the installer's update restart (which waits for the old pid to exit
//! before relaunching) hands over without help. A second launch can hand off
//! to the lock holder once its socket answers. Only a launch that claims the
//! profile can start another host.
use plexi_distribution::host_presence::LOCK_FILE;
use std::fs::{File, OpenOptions, TryLockError};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Exit status of a launch refused because a host already runs.
pub(crate) const ALREADY_RUNNING_EXIT: i32 = 3;

/// How long the socket probe waits for a pre-guard host to answer.
const PROBE_TIMEOUT: Duration = Duration::from_millis(500);

/// Maximum time for one readiness round trip to a host holding the lock.
const LIVE_PROBE_TIMEOUT: Duration = Duration::from_millis(300);

/// How long a launch waits for a live host to open its path.
const FORWARD_TIMEOUT: Duration = Duration::from_secs(2);

/// How long a launch waits for the lock holder to answer or release the lock.
/// An exiting host can remove its socket before `plexi host restart` starts
/// the next host, while the old process still holds the lock.
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

/// Proof that this process is the profile's host. Dropping it (or exiting)
/// releases the lock.
pub(crate) struct HostInstance {
    _lock: File,
}

/// Why this launch may not become the profile's host.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// Another host holds the lock and its socket answers: it is live, so
    /// the launch hands its path to that host and exits 0 (stint 1543).
    HostLive {
        lock: PathBuf,
        pid: Option<u32>,
        endpoint: PathBuf,
    },
    /// Another host held the lock for the whole grace. `socket_timed_out`
    /// distinguishes readiness polling from waiting for a lock release
    /// after a failed handoff.
    LockHeld {
        lock: PathBuf,
        pid: Option<u32>,
        socket_timed_out: bool,
    },
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

    pub(crate) fn launch_diagnostic(&self, forward: Option<&Path>) -> String {
        match (self, forward) {
            (
                Self::LockHeld {
                    socket_timed_out, ..
                },
                Some(path),
            ) if *socket_timed_out => {
                format!(
                    "{self}. Requested path {} was not delivered.",
                    path.display()
                )
            }
            _ => self.to_string(),
        }
    }
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::HostLive { lock, pid, .. } | Self::LockHeld { lock, pid, .. } => {
                let profile = lock.parent().unwrap_or(lock).display();
                write!(f, "Plexi is already running on this profile ({profile}")?;
                if let Some(pid) = pid {
                    write!(f, ", pid {pid}")?;
                }
                if matches!(self, Self::LockHeld { socket_timed_out, .. } if *socket_timed_out) {
                    f.write_str("); host socket did not answer before the startup deadline")
                } else {
                    f.write_str("); use that window")
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

/// What a host launch became.
pub(crate) enum Launch {
    /// This process is the profile's host.
    Host(HostInstance),
    /// A live host answered and took this launch's path (if any); the line
    /// says what happened, for a terminal launch.
    HandedOff(String),
}

/// Become the host for the profile at `profile_dir`, whose socket is
/// `endpoint`, or hand the launch to a host whose socket answers within the
/// grace. The host must acknowledge opening `forward` or showing its window.
/// After a failed handoff, wait for the lock release so a quitting host can
/// be replaced. Only the lock holder may remove or bind the socket.
pub(crate) fn claim(
    profile_dir: &Path,
    endpoint: &Path,
    forward: Option<&Path>,
) -> Result<Launch, Refusal> {
    claim_or_hand_off_with(
        profile_dir,
        endpoint,
        forward,
        EXIT_GRACE,
        &|endpoint, remaining| {
            crate::cli::host::host_answers(endpoint, profile_dir, LIVE_PROBE_TIMEOUT.min(remaining))
        },
        &|endpoint, root| {
            crate::cli::host::forward_host_launch(endpoint, profile_dir, root, FORWARD_TIMEOUT)
        },
        &|pid| {
            crate::platform::attention::yield_activation_to_host(pid);
        },
    )
}

type ReadinessProbe<'a> = dyn Fn(&Path, Duration) -> bool + 'a;
type HandOffResult = Result<(), String>;

fn claim_or_hand_off_with(
    profile_dir: &Path,
    endpoint: &Path,
    forward: Option<&Path>,
    grace: Duration,
    live: &ReadinessProbe<'_>,
    open: &dyn Fn(&Path, Option<&Path>) -> HandOffResult,
    yield_activation: &dyn Fn(u32),
) -> Result<Launch, Refusal> {
    let refusal = match claim_within(profile_dir, endpoint, grace, Some(live)) {
        Ok(instance) => return Ok(Launch::Host(instance)),
        Err(refusal) => refusal,
    };
    let Refusal::HostLive { pid, .. } = &refusal else {
        return Err(refusal);
    };
    if let Some(pid) = pid {
        yield_activation(*pid);
    } else {
        log::info!("host_startup: activation yield not available: live host pid missing");
    }
    let handed = match forward {
        None => open(endpoint, None).map(|()| refusal.to_string()),
        Some(root) => open(endpoint, Some(root))
            .map(|()| format!("Opened {} in the running Plexi.", root.display())),
    };
    match handed {
        Ok(line) => {
            log::info!("host_startup: handed the launch to the live host: {line}");
            Ok(Launch::HandedOff(line))
        }
        Err(error) => {
            log::warn!(
                "host_startup: the live host did not take the launch ({error}); waiting on the lock"
            );
            claim_within(profile_dir, endpoint, grace, None).map(Launch::Host)
        }
    }
}

fn claim_within(
    profile_dir: &Path,
    endpoint: &Path,
    grace: Duration,
    live: Option<&ReadinessProbe<'_>>,
) -> Result<HostInstance, Refusal> {
    let path = profile_dir.join(LOCK_FILE);
    std::fs::create_dir_all(profile_dir).map_err(|error| {
        Refusal::LockUnavailable(format!("create {}: {error}", profile_dir.display()))
    })?;
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    // On Unix the lock is this profile's own file: never followed through a
    // symlink or shared by hard link, so the pid never lands in another
    // profile. Windows has no such link refusal here.
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options
        .open(&path)
        .map_err(|error| Refusal::LockUnavailable(format!("open {}: {error}", path.display())))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let meta = file.metadata().map_err(|error| {
            Refusal::LockUnavailable(format!("inspect {}: {error}", path.display()))
        })?;
        if !meta.file_type().is_file() || meta.nlink() != 1 {
            log::error!(
                "host_instance: refusing linked lock {path:?} (links={})",
                meta.nlink()
            );
            return Err(Refusal::LockUnavailable(format!(
                "refusing linked host lock {}",
                path.display()
            )));
        }
    }
    let read_pid = |path: &Path| {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| text.trim().parse().ok())
    };
    // A held lock can belong to a starting or quitting host. Poll for a
    // readiness reply and retry the lock until either succeeds or time runs out.
    let deadline = Instant::now() + grace;
    loop {
        match file.try_lock() {
            Ok(()) => break,
            Err(TryLockError::WouldBlock) => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err(Refusal::LockHeld {
                        pid: read_pid(&path),
                        lock: path,
                        socket_timed_out: live.is_some(),
                    });
                }
                if live.is_some_and(|probe| probe(endpoint, remaining)) {
                    return Err(Refusal::HostLive {
                        pid: read_pid(&path),
                        lock: path,
                        endpoint: endpoint.to_path_buf(),
                    });
                }
                let remaining = deadline.saturating_duration_since(Instant::now());
                std::thread::sleep(remaining.min(Duration::from_millis(50)));
            }
            Err(TryLockError::Error(error)) => {
                return Err(Refusal::LockUnavailable(format!(
                    "lock {}: {error}",
                    path.display()
                )));
            }
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

    /// Round 7: host startup never writes its pid through a linked lock.
    #[cfg(unix)]
    #[test]
    fn a_linked_lock_is_refused_and_left_untouched() {
        let home = tempfile::tempdir().unwrap();
        let other = home.path().join("other.lock");
        std::fs::write(&other, "4242\n").unwrap();
        for link in ["symlink", "hard"] {
            let profile = home.path().join(format!(".plexi-{link}"));
            std::fs::create_dir(&profile).unwrap();
            let lock = profile.join(LOCK_FILE);
            if link == "symlink" {
                std::os::unix::fs::symlink(&other, &lock).unwrap();
            } else {
                std::fs::hard_link(&other, &lock).unwrap();
            }
            let endpoint = endpoint_in(&profile);
            let refusal = claim_within(&profile, &endpoint, Duration::ZERO, None)
                .err()
                .unwrap_or_else(|| panic!("{link}: a linked lock was claimed"));
            assert!(
                matches!(&refusal, Refusal::LockUnavailable(why) if why.contains("lock")),
                "{link}"
            );
            assert_eq!(std::fs::read_to_string(&other).unwrap(), "4242\n", "{link}");
        }
    }

    #[test]
    fn activation_is_yielded_to_the_live_host_before_show_request() {
        let profile = tempfile::tempdir().unwrap();
        let endpoint = endpoint_in(profile.path());
        let _host = claim_host(profile.path(), &endpoint).unwrap();
        std::fs::write(profile.path().join(LOCK_FILE), "4242\n").unwrap();
        let events = std::cell::RefCell::new(Vec::new());
        let result = claim_or_hand_off_with(
            profile.path(),
            &endpoint,
            None,
            EXIT_GRACE,
            &|_, _| true,
            &|_, root| {
                assert!(root.is_none());
                events.borrow_mut().push(("show", 4242));
                Ok(())
            },
            &|pid| events.borrow_mut().push(("yield", pid)),
        );
        assert!(matches!(result, Ok(Launch::HandedOff(_))));
        assert_eq!(*events.borrow(), [("yield", 4242), ("show", 4242)]);
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
            pid: None,
            socket_timed_out: true,
        }
        .is_another_host());
        assert!(Refusal::SocketLive {
            endpoint: PathBuf::from("/p/notify.sock")
        }
        .is_another_host());
    }

    /// The installer's outside view (stint 1605) sees a claimed host as
    /// running by its lock, with its pid. Fails if either side changes the
    /// lock file, the lock mode or the profile layout.
    #[test]
    fn a_claimed_host_is_running_to_the_installers_presence_probe() {
        use plexi_distribution::host_presence::{probe, Evidence, Presence};
        let profile = tempfile::tempdir().unwrap();
        let endpoint = endpoint_in(profile.path());
        let _host = claim_host(profile.path(), &endpoint).unwrap();
        let expected_pid = if cfg!(windows) {
            None
        } else {
            Some(std::process::id())
        };
        assert_eq!(
            probe(
                profile.path(),
                std::time::Instant::now() + Duration::from_secs(2)
            ),
            Presence::Running {
                pid: expected_pid,
                evidence: Evidence::Lock
            }
        );
    }

    /// Become the host, for a test that needs one holding the lock.
    fn claim_host(profile_dir: &Path, endpoint: &Path) -> Result<HostInstance, Refusal> {
        claim_within(profile_dir, endpoint, EXIT_GRACE, None)
    }

    /// A liveness probe with half the grace, not the production 300 ms, so a
    /// loaded runner's slow stand-in still counts as live; it still ends
    /// inside the grace (stint 1574).
    fn live_within_grace(profile_dir: &Path) -> impl Fn(&Path, Duration) -> bool + '_ {
        |endpoint, remaining| {
            crate::cli::host::host_answers(endpoint, profile_dir, remaining.min(EXIT_GRACE / 2))
        }
    }

    fn endpoint_in(dir: &Path) -> PathBuf {
        dir.join("notify.sock")
    }

    /// A stand-in host on `endpoint` that answers the next `count` requests
    /// the way the real one does: `[]` to the readiness round trip, and to
    /// `focus_context` either a context or `focus_error`. Returns the roots it
    /// was asked to open. No-root launches must use `show_host`.
    #[cfg(unix)]
    fn answering_host(
        endpoint: &Path,
        count: usize,
        focus_error: Option<&'static str>,
    ) -> std::thread::JoinHandle<Vec<PathBuf>> {
        use std::io::BufRead as _;
        let listener = crate::platform::ipc::IpcListener::bind(endpoint).unwrap();
        listener.set_nonblocking(true).unwrap();
        std::thread::spawn(move || {
            let mut roots = Vec::new();
            // Fail rather than hang when fewer connections arrive.
            let deadline = std::time::Instant::now()
                + crate::testing::load_aware_timeout(Duration::from_secs(10));
            for _ in 0..count {
                let stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(
                                std::time::Instant::now() < deadline,
                                "the stand-in host got {} of {count} connections",
                                roots.len()
                            );
                            std::thread::sleep(Duration::from_millis(10));
                        }
                        Err(e) => panic!("accept: {e}"),
                    }
                };
                stream.set_nonblocking(false).unwrap();
                let mut line = String::new();
                std::io::BufReader::new(stream)
                    .read_line(&mut line)
                    .unwrap();
                let request: serde_json::Value = serde_json::from_str(&line).unwrap();
                let file = request["response_file"].as_str().unwrap();
                let reply = match request.get("root").and_then(|r| r.as_str()) {
                    Some(root) => {
                        roots.push(PathBuf::from(root));
                        match focus_error {
                            Some(error) => serde_json::json!({ "error": error }),
                            None => serde_json::json!({ "context_id": 1, "root": root }),
                        }
                    }
                    None => match request["type"].as_str().unwrap() {
                        "list_panes" => serde_json::json!([]),
                        "show_host" => match focus_error {
                            Some(error) => serde_json::json!({ "error": error }),
                            None => serde_json::json!({ "ok": true }),
                        },
                        other => panic!("unexpected request: {other}"),
                    },
                };
                // The host's own write: atomic, so the launch never reads a
                // half-written reply (stint 1574).
                assert!(
                    crate::rpc::write_response(file, reply.to_string().as_bytes()),
                    "the stand-in host could not answer"
                );
            }
            roots
        })
    }

    /// Window restoration must be serviced even when eframe skips App::ui.
    #[test]
    fn a_show_request_restores_the_window_during_hidden_logic() {
        let mut h = crate::testing::HostHarness::new();
        let reply_dir = tempfile::tempdir().unwrap();
        let reply = reply_dir.path().join("show.json");
        let request = serde_json::from_value(serde_json::json!({
            "type": "show_host", "response_file": reply,
        }))
        .expect("show request wire format");
        h.inject_ipc(request);
        let ctx = h.app.ctx.clone();
        let mut input = egui::RawInput::default();
        let viewport = input.viewports.entry(egui::ViewportId::ROOT).or_default();
        viewport.occluded = Some(true);
        viewport.minimized = Some(true);
        let output = ctx.run_ui(input, |ui| {
            eframe::App::logic(&mut h.app, ui.ctx(), &mut eframe::Frame::_new_kittest());
        });
        let commands = &output.viewport_output[&egui::ViewportId::ROOT].commands;
        assert!(
            commands.contains(&egui::ViewportCommand::Visible(true)),
            "{commands:?}"
        );
        assert!(
            commands.contains(&egui::ViewportCommand::Minimized(false)),
            "{commands:?}"
        );
        assert!(
            commands.contains(&egui::ViewportCommand::Focus),
            "{commands:?}"
        );
        let response: serde_json::Value =
            serde_json::from_slice(&std::fs::read(reply).unwrap()).unwrap();
        assert_eq!(response["ok"], true);
    }

    /// A no-root relaunch must ask the live host to show its window.
    #[test]
    fn a_no_root_launch_requests_the_live_window() {
        let profile = tempfile::tempdir().unwrap();
        let endpoint = endpoint_in(profile.path());
        let _first = claim_host(profile.path(), &endpoint).expect("first host");
        let sent = std::cell::Cell::new(false);
        let launch = claim_or_hand_off_with(
            profile.path(),
            &endpoint,
            None,
            EXIT_GRACE,
            &|_, _| true,
            &|target, root| {
                assert_eq!(target, endpoint);
                assert!(root.is_none());
                sent.set(true);
                Ok(())
            },
            &|_| {},
        );
        assert!(matches!(launch, Ok(Launch::HandedOff(_))));
        assert!(sent.get(), "a no-root relaunch must send a show request");
    }

    /// Stint 1543 (and 1459): a second launch on a profile whose host holds
    /// the lock and answers a round trip is told so at once, without spending
    /// the exit grace, and never touches the socket.
    #[cfg(unix)]
    #[test]
    fn a_second_launch_on_a_live_host_returns_at_once() {
        let profile = tempfile::tempdir().unwrap();
        let endpoint = endpoint_in(profile.path());
        let first = claim_host(profile.path(), &endpoint).expect("the first launch is the host");
        let host = answering_host(&endpoint, 2, None);

        let started = std::time::Instant::now();
        let launch = claim_or_hand_off_with(
            profile.path(),
            &endpoint,
            None,
            EXIT_GRACE,
            &live_within_grace(profile.path()),
            &|endpoint, root| {
                assert!(root.is_none());
                crate::cli::host::forward_host_launch(
                    endpoint,
                    profile.path(),
                    root,
                    FORWARD_TIMEOUT,
                )
            },
            &|_| {},
        );
        let elapsed = started.elapsed();
        let Ok(Launch::HandedOff(line)) = launch else {
            panic!("a live host must take the launch");
        };
        // Any wait on the lock lasts at least the whole grace (the lock stays
        // held), and the probe ends within half of it, so finishing inside the
        // grace proves it was skipped without racing the clock (stint 1574).
        assert!(
            elapsed < EXIT_GRACE,
            "a live host must be reported without waiting the {EXIT_GRACE:?} exit grace, took {elapsed:?}"
        );
        assert!(line.contains("already running on this profile"), "{line}");
        assert!(
            endpoint.exists(),
            "the live host's socket must not be unlinked"
        );
        host.join().unwrap();
        drop(first);
    }

    /// The owner's case: a launch while the old host is quitting. The old
    /// host still holds the lock and its socket still accepts connects, but
    /// nothing answers; it then removes its socket and exits. The launch must
    /// become the new host, not report the old one as running.
    #[cfg(unix)]
    #[test]
    fn a_launch_during_quit_becomes_the_next_host() {
        let profile = tempfile::tempdir().unwrap();
        let endpoint = endpoint_in(profile.path());
        let old = claim_host(profile.path(), &endpoint).expect("the old host");
        let listener = crate::platform::ipc::IpcListener::bind(&endpoint).unwrap();
        let sock = endpoint.clone();
        // exit_host releases the socket, then the process exit drops the lock.
        let quitting = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(500));
            drop(listener);
            let _ = std::fs::remove_file(&sock);
            drop(old);
        });

        match claim(profile.path(), &endpoint, None) {
            Ok(Launch::Host(_)) => {}
            Ok(Launch::HandedOff(line)) => {
                panic!("a quitting host was reported as running: {line}")
            }
            Err(refusal) => panic!("the launch must take over after the quit: {refusal}"),
        }
        quitting.join().unwrap();
    }

    /// A host that answers the readiness round trip with a trickle (one byte
    /// every 100 ms, never finishing) is not live: the launch gives up within
    /// the probe's bound and falls back to the lock wait.
    #[cfg(unix)]
    #[test]
    fn a_trickling_host_is_not_live_and_the_launch_waits_on_the_lock() {
        use std::io::{BufRead as _, Write as _};
        let profile = tempfile::tempdir().unwrap();
        let endpoint = endpoint_in(profile.path());
        let old = claim_host(profile.path(), &endpoint).expect("the old host");
        let listener = crate::platform::ipc::IpcListener::bind(&endpoint).unwrap();
        let sock = endpoint.clone();
        let trickling = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            std::io::BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut line)
                .unwrap();
            let mut stream = stream;
            let until = std::time::Instant::now() + Duration::from_millis(1500);
            while std::time::Instant::now() < until && stream.write_all(b" ").is_ok() {
                std::thread::sleep(Duration::from_millis(100));
            }
            drop(listener);
            let _ = std::fs::remove_file(&sock);
            drop(old);
        });

        let started = std::time::Instant::now();
        let first_probe_finished = std::cell::OnceCell::new();
        let launch = claim_or_hand_off_with(
            profile.path(),
            &endpoint,
            None,
            Duration::from_secs(5),
            &|endpoint, remaining| {
                let live = crate::cli::host::host_answers(
                    endpoint,
                    profile.path(),
                    LIVE_PROBE_TIMEOUT.min(remaining),
                );
                first_probe_finished.get_or_init(|| started.elapsed());
                live
            },
            &|_, _| unreachable!("no path"),
            &|_| {},
        );
        assert!(
            first_probe_finished.into_inner().unwrap() < LIVE_PROBE_TIMEOUT * 3,
            "the probe must give up within its bound"
        );
        assert!(
            matches!(launch, Ok(Launch::Host(_))),
            "the launch must fall back to the lock and take over"
        );
        trickling.join().unwrap();
    }

    /// The launch's path goes to exactly the live host's endpoint, as the
    /// canonical path main resolved, and the terminal gets one plain line.
    #[cfg(unix)]
    #[test]
    fn a_second_launch_opens_its_path_on_the_live_host() {
        let profile = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let root = project.path().canonicalize().unwrap();
        let endpoint = endpoint_in(profile.path());
        let first = claim_host(profile.path(), &endpoint).expect("the first launch is the host");
        let host = answering_host(&endpoint, 2, None);

        let launch = claim_or_hand_off_with(
            profile.path(),
            &endpoint,
            Some(&root),
            EXIT_GRACE,
            &live_within_grace(profile.path()),
            &|endpoint, root| {
                crate::cli::host::forward_host_launch(
                    endpoint,
                    profile.path(),
                    root,
                    FORWARD_TIMEOUT,
                )
            },
            &|_| {},
        );
        let Ok(Launch::HandedOff(line)) = launch else {
            panic!("a live host must take the launch's path");
        };
        assert_eq!(host.join().unwrap(), std::slice::from_ref(&root));
        assert_eq!(
            line,
            format!("Opened {} in the running Plexi.", root.display())
        );
        drop(first);
    }

    #[cfg(unix)]
    #[test]
    fn a_starting_host_receives_the_path_when_its_socket_answers() {
        let profile = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let root = project.path().canonicalize().unwrap();
        let endpoint = endpoint_in(profile.path());
        let _first = claim_host(profile.path(), &endpoint).expect("the first launch is the host");
        let grace = crate::testing::load_aware_timeout(EXIT_GRACE);
        let host = std::cell::RefCell::new(None);

        let started = std::time::Instant::now();
        let launch = claim_or_hand_off_with(
            profile.path(),
            &endpoint,
            Some(&root),
            grace,
            &|endpoint, remaining| {
                let live = crate::cli::host::host_answers(
                    endpoint,
                    profile.path(),
                    remaining.min(grace / 2),
                );
                if host.borrow().is_none() {
                    assert!(!live, "the first probe must miss the starting host");
                    *host.borrow_mut() = Some(answering_host(endpoint, 2, None));
                }
                live
            },
            &|endpoint, root| {
                crate::cli::host::forward_host_launch(endpoint, profile.path(), root, grace / 2)
            },
            &|_| {},
        );
        let elapsed = started.elapsed();
        let line = match launch {
            Ok(Launch::HandedOff(line)) => line,
            Ok(Launch::Host(_)) => panic!("the newcomer unexpectedly took the lock"),
            Err(refusal) => panic!("the starting host lost the path after {elapsed:?}: {refusal}"),
        };
        assert!(
            elapsed < grace,
            "the handoff spent the full grace: {elapsed:?}"
        );
        assert_eq!(
            host.into_inner().unwrap().join().unwrap(),
            std::slice::from_ref(&root)
        );
        assert_eq!(
            line,
            format!("Opened {} in the running Plexi.", root.display())
        );
    }

    /// A live host that does not take the path never ends the launch as a
    /// success: the launch falls back to waiting on the lock.
    #[cfg(unix)]
    #[test]
    fn a_failed_hand_off_falls_back_to_waiting_on_the_lock() {
        for with_root in [false, true] {
            let profile = tempfile::tempdir().unwrap();
            let project = tempfile::tempdir().unwrap();
            let endpoint = endpoint_in(profile.path());
            let first =
                claim_host(profile.path(), &endpoint).expect("the first launch is the host");
            let host = answering_host(&endpoint, 2, Some("context refused"));
            // The old host exits once it has refused the path, not after a timer,
            // so a slow launch still meets it holding the lock (stint 1574).
            let first = std::cell::RefCell::new(Some(first));
            let sock = endpoint.clone();

            let launch = claim_or_hand_off_with(
                profile.path(),
                &endpoint,
                with_root.then_some(project.path()),
                EXIT_GRACE,
                &live_within_grace(profile.path()),
                &|endpoint, root| {
                    let result = crate::cli::host::forward_host_launch(
                        endpoint,
                        profile.path(),
                        root,
                        FORWARD_TIMEOUT,
                    );
                    let _ = std::fs::remove_file(&sock);
                    drop(first.borrow_mut().take());
                    result
                },
                &|_| {},
            );
            assert!(
                matches!(launch, Ok(Launch::Host(_))),
                "a refused hand-off must fall back to the lock"
            );
            host.join().unwrap();
        }
    }

    #[test]
    fn a_starting_host_that_never_answers_reports_the_undelivered_path() {
        let profile = tempfile::tempdir().unwrap();
        let endpoint = endpoint_in(profile.path());
        let _first = claim_host(profile.path(), &endpoint).expect("the first launch is the host");
        let forward = profile.path().join("workspace");
        let grace = crate::testing::load_aware_timeout(Duration::from_millis(300));

        let started = std::time::Instant::now();
        let refusal = claim_or_hand_off_with(
            profile.path(),
            &endpoint,
            Some(&forward),
            grace,
            &live_within_grace(profile.path()),
            &|_, _| panic!("a host without a socket cannot accept a handoff"),
            &|_| panic!("a host without a socket is not ready for activation"),
        )
        .err()
        .expect("a held lock without readiness must refuse");
        let elapsed = started.elapsed();
        assert!(
            (grace..grace * 2).contains(&elapsed),
            "wait took {elapsed:?}"
        );
        assert_eq!(
            refusal,
            Refusal::LockHeld {
                lock: profile.path().join(LOCK_FILE),
                pid: Some(std::process::id()),
                socket_timed_out: true,
            }
        );
        let diagnostic = refusal.launch_diagnostic(Some(&forward));
        assert!(diagnostic.contains("host socket did not answer before the startup deadline"));
        assert!(diagnostic.contains(&format!(
            "Requested path {} was not delivered.",
            forward.display()
        )));
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

        let refusal = match claim_host(profile.path(), &endpoint) {
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
        let old = claim_within(profile.path(), &endpoint, Duration::ZERO, None).unwrap();
        let exiting = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            drop(old);
        });
        claim_within(profile.path(), &endpoint, Duration::from_secs(3), None)
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
        let old = claim_host(profile.path(), &endpoint).expect("old build is the host");
        let listener = crate::platform::ipc::IpcListener::bind(&endpoint).unwrap();
        // The old host exits without cleaning up: its socket file stays.
        drop(listener);
        drop(old);

        claim_host(profile.path(), &endpoint).expect("the restarted build becomes the host");
    }
}

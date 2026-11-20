//! `plexi host status|stop|restart --profile <name>` address the named
//! profile's host, never the binary's default profile (stint 1655). Each test
//! runs a channel-named copy of the binary under a throwaway HOME where both
//! the default profile and the named profile have a fake host socket, and
//! records which one the command talked to.
#![cfg(unix)]

use std::io::{BufRead, BufReader};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const CHANNEL: &str = "pr-1655";
const PROFILE: &str = "t1655";

/// A fake host endpoint that records every request frame and closes the
/// connection without answering.
struct FakeHost {
    requests: Arc<Mutex<Vec<serde_json::Value>>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl FakeHost {
    fn bind(profile_dir: &Path) -> Self {
        std::fs::create_dir_all(profile_dir).unwrap();
        let listener = UnixListener::bind(profile_dir.join("notify.sock")).unwrap();
        listener.set_nonblocking(true).unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (seen, done) = (requests.clone(), stop.clone());
        let thread = std::thread::spawn(move || {
            while !done.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        // A probe may hang up before these calls; the read
                        // below then records it as a bare probe.
                        let _ = stream.set_nonblocking(false);
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                        let mut line = String::new();
                        let _ = BufReader::new(stream).read_line(&mut line);
                        let frame = serde_json::from_str::<serde_json::Value>(&line)
                            .unwrap_or_else(|_| serde_json::json!({"type": "<probe>"}));
                        seen.lock().unwrap().push(frame);
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(10)),
                }
            }
        });
        Self {
            requests,
            stop,
            thread: Some(thread),
        }
    }

    /// The `type` of every request received.
    fn requests(&self) -> Vec<String> {
        self.frames()
            .iter()
            .map(|frame| frame["type"].as_str().unwrap_or("<untyped>").to_string())
            .collect()
    }

    fn frames(&self) -> Vec<serde_json::Value> {
        self.requests.lock().unwrap().clone()
    }
}

impl Drop for FakeHost {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct Fixture {
    _home: tempfile::TempDir,
    home: PathBuf,
    binary: PathBuf,
    default_host: FakeHost,
    named_host: FakeHost,
}

fn fixture() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().to_path_buf();
    let default_host = FakeHost::bind(&home.join(format!(".plexi-{CHANNEL}")));
    let named_host = FakeHost::bind(&home.join(format!(".plexi-{PROFILE}")));
    let binary = home.join(format!("plexi-{CHANNEL}"));
    std::fs::copy(env!("CARGO_BIN_EXE_plexi"), &binary).unwrap();
    Fixture {
        _home: temp,
        home,
        binary,
        default_host,
        named_host,
    }
}

fn spawn(fixture: &Fixture, args: &[&str]) -> Child {
    spawn_raw(fixture, &[&["--profile", PROFILE], args].concat(), &[])
}

/// Run the binary with exactly `args` and `env` on top of a scrubbed
/// environment.
fn spawn_raw(fixture: &Fixture, args: &[&str], env: &[(&str, &Path)]) -> Child {
    let mut command = Command::new(&fixture.binary);
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("PLEXI_") {
            command.env_remove(key);
        }
    }
    command
        .args(args)
        .current_dir(&fixture.home)
        .env("HOME", &fixture.home)
        // Env never routes host commands; a stray channel must not matter.
        .env("PLEXI_CHANNEL", "beta")
        .envs(env.iter().map(|(key, value)| (key, value)))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

fn assert_no_contact(fixture: &Fixture, command: &str) {
    let named = fixture.named_host.requests();
    let default = fixture.default_host.requests();
    assert!(
        named.is_empty() && default.is_empty(),
        "{command} contacted a host: named={named:?} default={default:?}"
    );
}

fn run(fixture: &Fixture, args: &[&str]) {
    let child = spawn(fixture, args);
    let output = child.wait_with_output().unwrap();
    eprintln!(
        "{args:?} exited {}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_only_named(fixture: &Fixture, command: &str, expect: &str) {
    let named = fixture.named_host.requests();
    let default = fixture.default_host.requests();
    assert!(
        default.is_empty(),
        "{command} --profile {PROFILE} contacted the default profile's host: {default:?}"
    );
    assert!(
        named.iter().any(|kind| kind == expect),
        "{command} --profile {PROFILE} never sent {expect:?} to the named profile: {named:?}"
    );
}

#[test]
fn host_status_with_profile_queries_only_the_named_profile() {
    let fixture = fixture();
    run(&fixture, &["host", "status", "--json"]);
    assert_only_named(&fixture, "host status", "get_build_info");
}

#[test]
fn host_stop_with_profile_stops_only_the_named_profile() {
    let fixture = fixture();
    run(&fixture, &["host", "stop"]);
    assert_only_named(&fixture, "host stop", "shutdown");
}

/// Restart's stop step runs in a child `host stop`; it must carry the profile.
/// The fake named host never exits, so restart waits; the test ends it once the
/// shutdown request has arrived, before any start step could launch a host.
#[test]
fn host_restart_with_profile_stops_only_the_named_profile() {
    let fixture = fixture();
    let mut child = spawn(&fixture, &["host", "restart"]);
    let deadline = Instant::now() + Duration::from_secs(60);
    while !fixture
        .named_host
        .requests()
        .iter()
        .any(|kind| kind == "shutdown")
        && !fixture
            .default_host
            .requests()
            .iter()
            .any(|kind| kind == "shutdown")
    {
        if Instant::now() >= deadline {
            break;
        }
        if child.try_wait().unwrap().is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    child.kill().ok();
    child.wait().ok();
    // The restart's own stop child may still be running; give it no chance
    // to outlive the fixture's sockets.
    std::thread::sleep(Duration::from_millis(100));
    assert_only_named(&fixture, "host restart", "shutdown");
}

/// From a pane of the named profile's host, `--profile` keeps the caller's
/// pane identity, so restart asks for `--yes` (naming the binary and profile)
/// instead of shutting down the host that owns the calling pane.
#[test]
fn host_restart_with_profile_from_its_own_pane_asks_for_confirmation() {
    let fixture = fixture();
    let socket = fixture.home.join(format!(".plexi-{PROFILE}/notify.sock"));
    let output = spawn_raw(
        &fixture,
        &["--profile", PROFILE, "host", "restart"],
        &[
            ("PLEXI_SOCKET", socket.as_path()),
            ("PLEXI_PANE_ID", Path::new("7")),
        ],
    )
    .wait_with_output()
    .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "{stderr}");
    assert!(
        stderr.contains(&format!(
            "plexi-{CHANNEL} --profile {PROFILE} host restart --yes"
        )),
        "{stderr}"
    );
    for host in [&fixture.named_host, &fixture.default_host] {
        assert!(
            !host.requests().iter().any(|kind| kind == "shutdown"),
            "restart from its own pane sent a shutdown: {:?}",
            host.requests()
        );
    }
}

/// Lifecycle verbs address their profile's host, not `--socket`: a pane of
/// the named host that also passes another host's socket is still that
/// host's own pane, so restart asks for `--yes` and sends no shutdown.
#[test]
fn host_restart_from_its_own_pane_ignores_an_explicit_socket() {
    let fixture = fixture();
    let own = fixture.home.join(format!(".plexi-{PROFILE}/notify.sock"));
    let other = fixture.home.join(format!(".plexi-{CHANNEL}/notify.sock"));
    let output = spawn_raw(
        &fixture,
        &[
            "--profile",
            PROFILE,
            "--socket",
            other.to_str().unwrap(),
            "host",
            "restart",
        ],
        &[
            ("PLEXI_SOCKET", own.as_path()),
            ("PLEXI_PANE_ID", Path::new("7")),
        ],
    )
    .wait_with_output()
    .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "{stderr}");
    assert!(stderr.contains("host restart --yes"), "{stderr}");
    assert!(
        !fixture
            .named_host
            .requests()
            .iter()
            .any(|kind| kind == "shutdown"),
        "restart from its own pane sent a shutdown: {:?}",
        fixture.named_host.requests()
    );
    // Round 6: the explicit `--socket` host is not the lifecycle target, so
    // it receives nothing, not even the invocation's trace.
    assert!(
        fixture.default_host.frames().is_empty(),
        "the --socket host received frames: {:?}",
        fixture.default_host.frames()
    );
}

/// Round 6: a lifecycle target is validated before any connection, trace or
/// signal. A named profile whose dir, or whose `notify.sock`, links to another
/// profile is refused, and the linked host receives zero frames.
#[test]
fn a_linked_named_profile_reaches_no_host() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().to_path_buf();
    let default_dir = home.join(format!(".plexi-{CHANNEL}"));
    let default_host = FakeHost::bind(&default_dir);
    let binary = home.join(format!("plexi-{CHANNEL}"));
    std::fs::copy(env!("CARGO_BIN_EXE_plexi"), &binary).unwrap();
    let linked_dir = home.join(".plexi-linkdir");
    std::os::unix::fs::symlink(&default_dir, &linked_dir).unwrap();
    let linked_sock = home.join(".plexi-linksock");
    std::fs::create_dir(&linked_sock).unwrap();
    std::os::unix::fs::symlink(
        default_dir.join("notify.sock"),
        linked_sock.join("notify.sock"),
    )
    .unwrap();
    let fixture = Fixture {
        named_host: FakeHost::bind(&home.join(".plexi-unused")),
        _home: temp,
        home,
        binary,
        default_host,
    };
    for profile in ["linkdir", "linksock"] {
        for verb in [
            &["status"][..],
            &["stop"],
            &["restart", "--yes"],
            &["log", "x"],
        ] {
            let args = [&["--profile", profile, "host"][..], verb].concat();
            let output = run_raw(&fixture, &args);
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert_eq!(output.status.code(), Some(1), "{args:?}: {stderr}");
            assert!(
                stderr.contains("refusing to address a host"),
                "{args:?}: {stderr}"
            );
            assert!(
                fixture.default_host.frames().is_empty(),
                "{args:?} reached the linked host: {:?}",
                fixture.default_host.frames()
            );
        }
    }
}

fn run_raw(fixture: &Fixture, args: &[&str]) -> std::process::Output {
    spawn_raw(fixture, args, &[]).wait_with_output().unwrap()
}

/// Round 2: repeated, interleaved or late-invalid `--profile` occurrences
/// were resolved first-wins or skipped by `--version`. Each is a usage error
/// (exit 2) that reaches neither host.
#[test]
fn ambiguous_or_invalid_profiles_reach_no_host() {
    let fixture = fixture();
    let cases: &[&[&str]] = &[
        &["--profile", "one", "--profile", "two", "host", "stop"],
        &["--profile", "one", "--profile", "one", "host", "stop"],
        &["host", "--profile", "one", "stop", "--profile", "two"],
        &["--profile", "one", "--profile", "two", "--version"],
        &["--profile", "nested/name", "host", "stop"],
        &["--profile", "../x", "host", "stop"],
        &["--profile=", "host", "stop"],
        &["host", "stop", "--profile"],
        // Round 3: a later duplicate after an incomplete command.
        &["--profile", "one", "host", "--profile", "two"],
        // Round 4: the experiment diagnostic never replaces the rejection.
        &[
            "--profile",
            "one",
            "assistant",
            "permission",
            "list",
            "--profile",
            "two",
        ],
        &[
            "--profile",
            "one",
            "assistant",
            "permission",
            "list",
            "--profile",
            "one",
        ],
    ];
    for args in cases {
        let output = run_raw(&fixture, args);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(2), "{args:?}: {stderr}");
        assert!(
            output.stdout.is_empty(),
            "{args:?} printed: {:?}",
            output.stdout
        );
        assert_no_contact(&fixture, &format!("{args:?}"));
    }
}

/// Round 2: identity guards compared path spellings. On a case-insensitive
/// volume `--profile Team` and a pane of `.plexi-team` are one host, so the
/// pane keeps its identity and restart asks for `--yes`.
#[test]
fn a_case_alias_of_the_callers_own_host_is_the_same_host() {
    let fixture = fixture();
    let own = FakeHost::bind(&fixture.home.join(".plexi-team"));
    if !fixture.home.join(".plexi-Team").exists() {
        eprintln!("skipped: {} is case-sensitive", fixture.home.display());
        return;
    }
    let socket = fixture.home.join(".plexi-team/notify.sock");
    let output = spawn_raw(
        &fixture,
        &["--profile", "Team", "host", "restart"],
        &[
            ("PLEXI_SOCKET", socket.as_path()),
            ("PLEXI_PANE_ID", Path::new("7")),
        ],
    )
    .wait_with_output()
    .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "{stderr}");
    assert!(
        stderr.contains(&format!(
            "plexi-{CHANNEL} --profile Team host restart --yes"
        )),
        "{stderr}"
    );
    for host in [&own, &fixture.named_host, &fixture.default_host] {
        assert!(
            !host.requests().iter().any(|kind| kind == "shutdown"),
            "restart from its own pane sent a shutdown: {:?}",
            host.requests()
        );
    }
}

/// The inverse of the own-pane case: a bare `plexi --profile <name>` run from
/// a pane of another host sends none of that pane's identity to the named
/// host.
#[test]
fn a_foreign_pane_identity_never_reaches_the_named_host() {
    let fixture = fixture();
    let foreign = FakeHost::bind(&fixture.home.join(".plexi-beta"));
    let bare = fixture.home.join("plexi");
    std::fs::copy(&fixture.binary, &bare).unwrap();
    let socket = fixture.home.join(".plexi-beta/notify.sock");
    let mut command = Command::new(&bare);
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("PLEXI_") {
            command.env_remove(key);
        }
    }
    let output = command
        .args([
            "--profile",
            PROFILE,
            "events",
            "emit",
            "probe",
            "probe.tick",
        ])
        .args(["--summary", "s", "--resource", "r", "--revision-after", "1"])
        .current_dir(&fixture.home)
        .env("HOME", &fixture.home)
        .env("PLEXI_SOCKET", &socket)
        .env("PLEXI_PANE_ID", "229")
        .env("PLEXI_CONTEXT_ID", "4")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    eprintln!(
        "emit exited {}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let emits: Vec<_> = fixture
        .named_host
        .frames()
        .into_iter()
        .filter(|frame| frame["type"] == "events_emit")
        .collect();
    assert_eq!(
        emits.len(),
        1,
        "named host frames: {:?}",
        fixture.named_host.frames()
    );
    assert!(
        emits[0]["from_pane_id"].is_null(),
        "foreign identity leaked: {}",
        emits[0]
    );
    assert!(
        !foreign.requests().iter().any(|kind| kind == "events_emit"),
        "the emit went to the caller's host: {:?}",
        foreign.requests()
    );
}

/// Round 3: an explicit `--socket` to another host is a different host by
/// endpoint identity, so the caller's pane identity never reaches it.
#[test]
fn an_explicit_socket_to_another_host_gets_no_pane_identity() {
    let fixture = fixture();
    let own = FakeHost::bind(&fixture.home.join(".plexi-own"));
    let own_socket = fixture.home.join(".plexi-own/notify.sock");
    let other_socket = fixture.home.join(format!(".plexi-{PROFILE}/notify.sock"));
    let output = spawn_raw(
        &fixture,
        &[
            "--socket",
            other_socket.to_str().unwrap(),
            "events",
            "emit",
            "probe",
            "probe.tick",
            "--summary",
            "s",
            "--resource",
            "r",
            "--revision-after",
            "1",
        ],
        &[
            ("PLEXI_SOCKET", own_socket.as_path()),
            ("PLEXI_PANE_ID", Path::new("229")),
        ],
    )
    .wait_with_output()
    .unwrap();
    eprintln!(
        "emit exited {}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let emits: Vec<_> = fixture
        .named_host
        .frames()
        .into_iter()
        .filter(|frame| frame["type"] == "events_emit")
        .collect();
    assert_eq!(emits.len(), 1, "{:?}", fixture.named_host.frames());
    assert!(
        emits[0]["from_pane_id"].is_null(),
        "the caller's pane id reached another host: {}",
        emits[0]
    );
    assert!(own.requests().iter().all(|kind| kind != "events_emit"));
}

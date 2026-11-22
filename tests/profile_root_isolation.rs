//! `--profile-root` isolates hosts of one channel (stint 1695). Two
//! disposable roots each hold the same channel's profile with a fake host;
//! every lifecycle verb and uninstall against one root reaches only that
//! root's host, the other stays reachable and its sentinel survives. An
//! invalid root, or a private `HOME` without one, reaches no host at all.
//! `HOME`, `PLEXI_CHANNEL` and `PLEXI_SOCKET` are poisoned throughout: none of
//! them routes a lifecycle verb.
#![cfg(unix)]

use std::io::{BufRead, BufReader};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const CHANNEL: &str = "pr-1695";

/// A fake host endpoint that records each request frame. A `shutdown`
/// request makes it exit the way a host does: it stops listening and its
/// endpoint disappears.
struct FakeHost {
    socket: PathBuf,
    frames: Arc<Mutex<Vec<serde_json::Value>>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl FakeHost {
    fn bind(profile_dir: &Path) -> Self {
        std::fs::create_dir_all(profile_dir).unwrap();
        let socket = profile_dir.join("notify.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        listener.set_nonblocking(true).unwrap();
        let frames = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (seen, done, path) = (frames.clone(), stop.clone(), socket.clone());
        let thread = std::thread::spawn(move || {
            while !done.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let _ = stream.set_nonblocking(false);
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                        let mut line = String::new();
                        let _ = BufReader::new(stream).read_line(&mut line);
                        let frame = serde_json::from_str::<serde_json::Value>(&line)
                            .unwrap_or_else(|_| serde_json::json!({"type": "<probe>"}));
                        let shutdown = frame["type"] == "shutdown";
                        seen.lock().unwrap().push(frame);
                        if shutdown {
                            let _ = std::fs::remove_file(&path);
                            return;
                        }
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(10)),
                }
            }
        });
        Self {
            socket,
            frames,
            stop,
            thread: Some(thread),
        }
    }

    fn requests(&self) -> Vec<String> {
        self.frames
            .lock()
            .unwrap()
            .iter()
            .map(|frame| frame["type"].as_str().unwrap_or("<untyped>").to_string())
            .collect()
    }

    fn reachable(&self) -> bool {
        std::os::unix::net::UnixStream::connect(&self.socket).is_ok()
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

struct Root {
    _dir: tempfile::TempDir,
    path: PathBuf,
}

impl Root {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().canonicalize().unwrap();
        Self { _dir: dir, path }
    }

    fn profile(&self) -> PathBuf {
        self.path.join(format!(".plexi-{CHANNEL}"))
    }
}

struct Fixture {
    a: Root,
    b: Root,
    /// A private `HOME` holding its own copy of the channel profile: the
    /// directory every lifecycle verb used before `--profile-root`.
    home: Root,
    binary: PathBuf,
    distribution: PathBuf,
    _scratch: tempfile::TempDir,
}

fn fixture() -> Fixture {
    let scratch = tempfile::tempdir().unwrap();
    let binary = scratch.path().join(format!("plexi-{CHANNEL}"));
    std::fs::copy(env!("CARGO_BIN_EXE_plexi"), &binary).unwrap();
    // A managed installation record for the channel, with nothing installed,
    // so uninstall reaches its host and data steps.
    let distribution = scratch.path().canonicalize().unwrap().join("distribution");
    let install = distribution.join(CHANNEL);
    std::fs::create_dir_all(install.join("generations")).unwrap();
    let receipt = serde_json::json!({
        "schema": 1,
        "channel": CHANNEL,
        "root": install,
        "bin_dir": scratch.path().join("bin"),
        "applications_dir": scratch.path().join("Applications"),
        "active": {
            "path": install.join("generations/none"),
            "build_id": "none",
            "tag": "none",
            "executable": "plexi",
        },
        "previous": null,
        "integrations": [],
    });
    std::fs::write(
        install.join("installation.json"),
        serde_json::to_vec(&receipt).unwrap(),
    )
    .unwrap();
    Fixture {
        a: Root::new(),
        b: Root::new(),
        home: Root::new(),
        binary,
        distribution,
        _scratch: scratch,
    }
}

/// Run the channel binary with `args`, a private `HOME`, and inherited
/// routing values that name root B's host.
fn run(fixture: &Fixture, args: &[&str]) -> Output {
    let mut command = Command::new(&fixture.binary);
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("PLEXI_") {
            command.env_remove(key);
        }
    }
    let output = command
        .args(args)
        .current_dir(&fixture.home.path)
        .env("HOME", &fixture.home.path)
        .env("PLEXI_DISTRIBUTION_HOME", &fixture.distribution)
        .env("PLEXI_CHANNEL", "beta")
        .env("PLEXI_SOCKET", fixture.b.profile().join("notify.sock"))
        .stdin(Stdio::null())
        .output()
        .unwrap();
    eprintln!(
        "{args:?} exited {}: {}{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn root_arg(root: &Root) -> String {
    root.path.to_str().unwrap().to_string()
}

/// Same channel in two roots: status, stop, restart and uninstall against
/// root A reach only A's host; B's host stays reachable and untouched, and
/// its sentinel survives.
#[test]
fn two_roots_of_one_channel_are_isolated() {
    let fixture = fixture();
    let a_host = FakeHost::bind(&fixture.a.profile());
    let b_host = FakeHost::bind(&fixture.b.profile());
    let home_host = FakeHost::bind(&fixture.home.profile());
    let sentinel = fixture.b.profile().join("sentinel");
    std::fs::write(&sentinel, "keep").unwrap();
    let a = root_arg(&fixture.a);

    run(
        &fixture,
        &["--profile-root", &a, "host", "status", "--json"],
    );
    assert!(a_host.requests().contains(&"get_build_info".to_string()));

    run(&fixture, &["host", "stop", "--profile-root", &a]);
    assert!(a_host.requests().contains(&"shutdown".to_string()));
    assert!(!a_host.reachable());

    // A second host on root A, then uninstall with root A's data kept.
    let a_again = FakeHost::bind(&fixture.a.profile());
    let output = run(
        &fixture,
        &["--profile-root", &a, "uninstall", "--keep-data", "--yes"],
    );
    assert!(output.status.success());
    assert!(a_again.requests().contains(&"shutdown".to_string()));
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains(&fixture.a.profile().display().to_string()),
        "uninstall must name the selected profile"
    );

    for (name, host) in [("B", &b_host), ("HOME", &home_host)] {
        assert!(
            host.requests().is_empty(),
            "root {name}'s host was contacted: {:?}",
            host.requests()
        );
        assert!(host.reachable(), "root {name}'s host is gone");
    }
    assert_eq!(std::fs::read_to_string(&sentinel).unwrap(), "keep");
}

/// An invalid `--profile-root` is a usage error before any profile access,
/// connection, signal or deletion; so is a private `HOME` with no root,
/// which once chose the profile.
#[test]
fn an_invalid_root_or_a_bare_private_home_reaches_no_host() {
    let fixture = fixture();
    let a_host = FakeHost::bind(&fixture.a.profile());
    let b_host = FakeHost::bind(&fixture.b.profile());
    let home_host = FakeHost::bind(&fixture.home.profile());
    let file = fixture.a.path.join("file");
    std::fs::write(&file, "x").unwrap();
    let missing = fixture.a.path.join("missing");
    let invalid = [
        "relative/root".to_string(),
        missing.to_str().unwrap().to_string(),
        file.to_str().unwrap().to_string(),
        format!("{}/../{}", fixture.a.path.display(), "x"),
    ];
    let verbs: &[&[&str]] = &[
        &["host", "status"],
        &["host", "stop"],
        &["host", "restart", "--yes"],
        &["host", "log", "x"],
        &["uninstall", "--delete-data", "--yes"],
        &["pane", "list"],
    ];
    for root in &invalid {
        for verb in verbs {
            let args = [&["--profile-root", root.as_str()][..], verb].concat();
            let output = run(&fixture, &args);
            assert_eq!(output.status.code(), Some(2), "{args:?}");
            assert!(output.stdout.is_empty(), "{args:?}");
            assert!(
                String::from_utf8_lossy(&output.stderr).contains("--profile-root"),
                "{args:?}"
            );
        }
    }
    for verb in verbs {
        let output = run(&fixture, verb);
        assert_eq!(output.status.code(), Some(2), "{verb:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("HOME is") && stderr.contains("--profile-root"),
            "{stderr}"
        );
    }
    for (name, host) in [("A", &a_host), ("B", &b_host), ("HOME", &home_host)] {
        assert!(
            host.requests().is_empty(),
            "root {name}'s host was contacted: {:?}",
            host.requests()
        );
    }
    assert!(
        missing.symlink_metadata().is_err(),
        "a missing root was created"
    );
}

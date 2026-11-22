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

/// The child half of [`a_bare_launcher_handoff_never_routes_by_the_pane`]:
/// run the installed Stable launcher (`transaction::launch`) exactly as the
/// `plexi` shim does. It hands the command to the pane's build by `exec`, so
/// this process becomes that real `plexi-beta`. Inert in an ordinary run.
#[test]
fn launcher_child() {
    let Some(root) = std::env::var_os("PLEXI_1695_LAUNCH_ROOT") else {
        return;
    };
    let args: Vec<std::ffi::OsString> =
        serde_json::from_str::<Vec<String>>(&std::env::var("PLEXI_1695_LAUNCH_ARGS").unwrap())
            .unwrap()
            .into_iter()
            .map(Into::into)
            .collect();
    let code = plexi_distribution::transaction::launch(Path::new(&root), &args)
        .unwrap_or_else(|error| panic!("launch failed: {error}"));
    std::process::exit(code);
}

/// A managed installation of `channel` whose active build is a copy of this
/// plexi binary named `executable`.
fn managed_install(distribution: &Path, channel: &str, executable: &str) -> PathBuf {
    let root = distribution.join(channel);
    let generation = root.join("generations/test");
    std::fs::create_dir_all(&generation).unwrap();
    std::fs::copy(env!("CARGO_BIN_EXE_plexi"), generation.join(executable)).unwrap();
    let receipt = serde_json::json!({
        "schema": 1,
        "channel": channel,
        "root": root,
        "bin_dir": distribution.join("bin"),
        "applications_dir": distribution.join("Applications"),
        "active": {
            "path": generation,
            "build_id": "test",
            "tag": "test",
            "executable": executable,
        },
        "previous": null,
        "integrations": [],
    });
    std::fs::write(
        root.join("installation.json"),
        serde_json::to_vec(&receipt).unwrap(),
    )
    .unwrap();
    let registry = distribution.join("installations");
    std::fs::create_dir_all(&registry).unwrap();
    std::fs::write(
        registry.join(format!("{channel}.json")),
        serde_json::to_vec(&root).unwrap(),
    )
    .unwrap();
    root
}

/// Codex 3288 R1: bare `plexi` typed in an installed Beta pane is handed by
/// the Stable launcher to the real `plexi-beta`. A lifecycle verb or
/// uninstall that arrives that way is refused (it would act on Beta's
/// installation and profile), and a bare command under a private `HOME` is
/// refused before the pane endpoint can root it. The pane's live host is
/// never contacted and its data survives.
#[test]
fn a_bare_launcher_handoff_never_routes_by_the_pane() {
    let scratch = Root::new();
    let distribution = scratch.path.join("distribution");
    let stable = managed_install(&distribution, "stable", "plexi");
    managed_install(&distribution, "beta", "plexi-beta");
    // The pane's host: Beta's profile under the account-like root.
    let owner = Root::new();
    let beta_profile = owner.path.join(".plexi-beta");
    let beta_host = FakeHost::bind(&beta_profile);
    let sentinel = beta_profile.join("sentinel");
    std::fs::write(&sentinel, "keep").unwrap();
    let private_home = Root::new();

    let launch = |args: &[&str], home: &Path| {
        let mut command = Command::new(std::env::current_exe().unwrap());
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("PLEXI_") {
                command.env_remove(key);
            }
        }
        let output = command
            .args([
                "--exact",
                "launcher_child",
                "--nocapture",
                "--test-threads",
                "1",
            ])
            .env("PLEXI_1695_LAUNCH_ROOT", &stable)
            .env(
                "PLEXI_1695_LAUNCH_ARGS",
                serde_json::to_string(&args).unwrap(),
            )
            .env("PLEXI_DISTRIBUTION_HOME", &distribution)
            .env("PLEXI_RUNNING", "1")
            .env("PLEXI_CHANNEL", "beta")
            .env("PLEXI_SOCKET", &beta_host.socket)
            .env("HOME", home)
            .current_dir(&scratch.path)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        eprintln!("{args:?} exited {}: {stderr}", output.status);
        (output.status.code(), stderr)
    };

    for verb in [
        &["uninstall", "--delete-data", "--yes"][..],
        &["host", "stop"],
        &["host", "status"],
    ] {
        let (code, stderr) = launch(verb, &owner.path);
        assert_eq!(code, Some(2), "{verb:?}: {stderr}");
        assert!(stderr.contains("never follow a pane"), "{verb:?}: {stderr}");
        assert!(stderr.contains("plexi-beta"), "{verb:?}: {stderr}");
    }
    // A bare command handed over under a private HOME.
    let (code, stderr) = launch(&["pane", "list"], &private_home.path);
    assert_eq!(code, Some(2), "{stderr}");
    assert!(stderr.contains("--profile-root"), "{stderr}");

    assert!(
        beta_host.requests().is_empty(),
        "the pane's host was contacted: {:?}",
        beta_host.requests()
    );
    assert!(beta_host.reachable());
    assert_eq!(std::fs::read_to_string(&sentinel).unwrap(), "keep");
}

/// Design 1695 (test 1): a direct host launch on a root whose profile links
/// to another directory exits before it locks or writes anything there.
#[test]
fn a_host_launch_on_a_linked_profile_writes_nothing_through_the_link() {
    let root = Root::new();
    let sentinel = Root::new();
    std::os::unix::fs::symlink(&sentinel.path, root.path.join(".plexi-x")).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_plexi"));
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("PLEXI_") {
            command.env_remove(key);
        }
    }
    let mut child = command
        .arg("--profile-root")
        .arg(&root.path)
        .args(["--profile", "x"])
        .current_dir(&root.path)
        .env("HOME", &root.path)
        .env("PLEXI_BACKGROUND_HOST", "1")
        .env("PLEXI_EPHEMERAL_SESSION", "1")
        .env("PLEXI_KEYCHAIN_PATH", root.path.join("t.keychain-db"))
        .env("PLEXI_KEYCHAIN_PASSWORD", "t1695-throwaway-keychain")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break Some(status);
        }
        if std::time::Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let written: Vec<_> = std::fs::read_dir(&sentinel.path)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert!(written.is_empty(), "wrote through the link: {written:?}");
    assert_eq!(status.and_then(|s| s.code()), Some(2));
}

/// A managed installation whose build is a script that records each run, as
/// an older receiver with a frozen grammar would be observed from outside.
fn recording_install(
    distribution: &Path,
    channel: &str,
    executable: &str,
    record: &Path,
    declares_handoff: bool,
) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let root = distribution.join(channel);
    let generation = root.join("generations/test");
    std::fs::create_dir_all(&generation).unwrap();
    let script = generation.join(executable);
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nprintf '%s marker=%s %s\\n' {channel} \"${{PLEXI_LAUNCHER_DELEGATED:-}}\" \"$*\" >> '{}'\n",
            record.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut manifest = serde_json::json!({
        "schema": 1, "version": "1.0.0", "tag": "v1.0.0", "build_id": "old",
        "source_commit": "old", "channel": channel, "platform": "test",
        "executable": executable, "resources": "resources", "files": {},
    });
    if declares_handoff {
        manifest["launcher_handoff"] = true.into();
    }
    std::fs::write(generation.join("package.json"), manifest.to_string()).unwrap();
    let receipt = serde_json::json!({
        "schema": 1,
        "channel": channel,
        "root": root,
        "bin_dir": distribution.join("bin"),
        "applications_dir": distribution.join("Applications"),
        "active": {"path": generation, "build_id": "old", "tag": "v1.0.0", "executable": executable},
        "previous": null,
        "integrations": [],
    });
    std::fs::write(root.join("installation.json"), receipt.to_string()).unwrap();
    let registry = distribution.join("installations");
    std::fs::create_dir_all(&registry).unwrap();
    std::fs::write(
        registry.join(format!("{channel}.json")),
        serde_json::to_vec(&root).unwrap(),
    )
    .unwrap();
    root
}

/// Design 1695 (test 5), through the real `transaction::launch`: a receiver
/// without the handoff never runs for a lifecycle verb or for a pane that is
/// not its own default profile, and does run for its default pane. A
/// receiver that declares the handoff gets ordinary commands with the marker.
#[test]
fn the_launcher_protects_an_older_receiver() {
    for declares_handoff in [false, true] {
        let scratch = Root::new();
        let distribution = scratch.path.join("distribution");
        let record = scratch.path.join("record");
        let stable = recording_install(&distribution, "stable", "plexi", &record, true);
        recording_install(
            &distribution,
            "beta",
            "plexi-beta",
            &record,
            declares_handoff,
        );
        let default_pane = scratch.path.join(".plexi-beta/notify.sock");
        let other_pane = scratch.path.join("other/.plexi-beta/notify.sock");
        let launch = |args: &[&str], pane: &Path| {
            let _ = std::fs::remove_file(&record);
            let mut command = Command::new(std::env::current_exe().unwrap());
            for (key, _) in std::env::vars_os() {
                if key.to_string_lossy().starts_with("PLEXI_") {
                    command.env_remove(key);
                }
            }
            let output = command
                .args([
                    "--exact",
                    "launcher_child",
                    "--nocapture",
                    "--test-threads",
                    "1",
                ])
                .env("PLEXI_1695_LAUNCH_ROOT", &stable)
                .env(
                    "PLEXI_1695_LAUNCH_ARGS",
                    serde_json::to_string(&args).unwrap(),
                )
                .env("PLEXI_DISTRIBUTION_HOME", &distribution)
                .env("PLEXI_RUNNING", "1")
                .env("PLEXI_CHANNEL", "beta")
                .env("PLEXI_SOCKET", pane)
                .env("HOME", &scratch.path)
                .current_dir(&scratch.path)
                .stdin(Stdio::null())
                .output()
                .unwrap();
            (
                output.status.code(),
                String::from_utf8_lossy(&output.stderr).to_string(),
                std::fs::read_to_string(&record).unwrap_or_default(),
            )
        };
        for args in [
            &["host", "stop"][..],
            &["host", "status"],
            &["uninstall", "--yes"],
            &["update"],
            &[],
        ] {
            for pane in [&default_pane, &other_pane] {
                let (code, stderr, ran) = launch(args, pane);
                assert_eq!(code, Some(2), "{args:?}: {stderr}");
                assert!(stderr.contains("never follow a pane"), "{stderr}");
                assert_eq!(ran, "", "{args:?} ran a build");
            }
        }
        let (_, _, ran) = launch(&["pane", "list"], &default_pane);
        let expected = if declares_handoff { "1" } else { "" };
        assert_eq!(ran, format!("beta marker={expected} pane list\n"));
        let (_, _, ran) = launch(&["pane", "list"], &other_pane);
        if declares_handoff {
            assert_eq!(ran, "beta marker=1 pane list\n");
        } else {
            assert_eq!(ran, "stable marker= pane list\n", "ran the older receiver");
        }
    }
}

/// A scratch root holds profiles the way a home directory does, so it is
/// never initialized as a workspace, whatever `HOME` says.
#[test]
fn a_scratch_root_is_never_a_workspace() {
    let fixture = fixture();
    let root = &fixture.a.path;
    let mut command = Command::new(&fixture.binary);
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("PLEXI_") {
            command.env_remove(key);
        }
    }
    let output = command
        .arg("--profile-root")
        .arg(root)
        .args(["workspace", "init"])
        .current_dir(root)
        .env("HOME", &fixture.home.path)
        .env("PLEXI_KEYCHAIN_PATH", root.join("t.keychain-db"))
        .env("PLEXI_KEYCHAIN_PASSWORD", "t1695-throwaway-keychain")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "{stderr}");
    assert!(
        !fixture.a.profile().join("workspace.toml").exists(),
        "{stderr}"
    );
}

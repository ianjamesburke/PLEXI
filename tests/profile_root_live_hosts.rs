//! Real hosts on disposable profile roots (stint 1695). Each test starts
//! real `plexi-pr-1695` hosts with a private `HOME`, an explicit
//! `--profile-root`, a throwaway keychain file and every inherited `PLEXI_*`
//! variable removed, so nothing reaches the account's profiles, endpoints
//! or login keychain. They open host windows in the background, so they are
//! opt-in: `cargo test --test profile_root_live_hosts -- --ignored`.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

const CHANNEL: &str = "pr-1695";

struct Sandbox {
    _dir: tempfile::TempDir,
    path: PathBuf,
    binary: PathBuf,
}

impl Sandbox {
    fn new() -> Self {
        // Short: every endpoint must fit the Unix socket path limit.
        let dir = tempfile::Builder::new()
            .prefix("p1695-")
            .tempdir_in("/tmp")
            .unwrap();
        // Canonical: a receipt names its own root exactly.
        let path = dir.path().canonicalize().unwrap();
        for sub in ["home", "a", "b", "bin", "dist"] {
            std::fs::create_dir_all(path.join(sub)).unwrap();
        }
        let binary = path.join(format!("bin/plexi-{CHANNEL}"));
        std::fs::copy(env!("CARGO_BIN_EXE_plexi"), &binary).unwrap();
        Self {
            _dir: dir,
            path,
            binary,
        }
    }

    /// A managed installation record with nothing installed, so uninstall
    /// reaches its host and data steps. Written after the hosts start: a
    /// channel host refuses to start against a receipt it cannot use.
    fn record_installation(&self) {
        let install = self.path.join("dist").join(CHANNEL);
        std::fs::create_dir_all(install.join("generations")).unwrap();
        let receipt = serde_json::json!({
            "schema": 1,
            "channel": CHANNEL,
            "root": install,
            "bin_dir": self.path.join("bin-unused"),
            "applications_dir": self.path.join("Applications"),
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
    }

    fn root(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }

    fn profile(&self, root: &str) -> PathBuf {
        self.root(root).join(format!(".plexi-{CHANNEL}"))
    }

    fn command(&self, root: &str, extra_env: &[(&str, &str)]) -> Command {
        self.command_for(&self.binary, root, extra_env)
    }

    fn command_for(&self, program: &Path, root: &str, extra_env: &[(&str, &str)]) -> Command {
        let mut command = Command::new(program);
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("PLEXI_") {
                command.env_remove(key);
            }
        }
        command
            .arg("--profile-root")
            .arg(self.root(root))
            .current_dir(self.root("home"))
            .env("HOME", self.root("home"))
            .env("PLEXI_DISTRIBUTION_HOME", self.root("dist"))
            .env("PLEXI_KEYCHAIN_PATH", self.path.join("t1695.keychain-db"))
            .env("PLEXI_KEYCHAIN_PASSWORD", "t1695-throwaway-keychain")
            .stdin(Stdio::null());
        for (key, value) in extra_env {
            command.env(key, value);
        }
        command
    }

    fn run(&self, root: &str, args: &[&str]) -> Output {
        let output = self.command(root, &[]).args(args).output().unwrap();
        eprintln!(
            "[{root}] {args:?} exited {}: {}{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }

    fn start(&self, root: &str) {
        let output = self.run(
            root,
            &[
                "host",
                "start",
                "--background",
                "--ephemeral",
                "--timeout-secs",
                "90",
            ],
        );
        assert!(output.status.success(), "host start in {root} failed");
    }

    fn status(&self, root: &str) -> serde_json::Value {
        let output = self.run(root, &["host", "status", "--json"]);
        serde_json::from_slice(&output.stdout).unwrap()
    }

    fn stop_quietly(&self, root: &str) {
        let _ = self.command(root, &[]).args(["host", "stop"]).output();
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        // The test receipt would stop `host stop` at profile initialization.
        let _ = std::fs::remove_dir_all(self.path.join("dist"));
        for root in ["a", "b"] {
            self.stop_quietly(root);
        }
    }
}

/// Same channel in two roots, both hosts really started: a delete-data
/// uninstall against root A stops A's host and deletes only A's profile;
/// B's host stays up and answers, and B's data survives.
#[test]
#[ignore = "starts real hosts; run with --ignored"]
fn uninstall_in_one_root_leaves_the_other_roots_host_and_data() {
    let sandbox = Sandbox::new();
    sandbox.start("a");
    sandbox.start("b");
    for root in ["a", "b"] {
        let status = sandbox.status(root);
        assert_eq!(status["ready"], true, "{root}: {status}");
        assert_eq!(
            Path::new(status["socket"].as_str().unwrap()),
            sandbox.profile(root).join("notify.sock"),
            "{root}'s host listens in its own root"
        );
        std::fs::write(sandbox.profile(root).join("sentinel"), root).unwrap();
    }
    let b_pid = sandbox.status("b")["pid"].clone();
    sandbox.record_installation();

    let output = sandbox.run("a", &["uninstall", "--delete-data", "--yes"]);
    assert!(output.status.success(), "uninstall in root a failed");
    assert!(!sandbox.profile("a").exists(), "root a's profile survived");

    let status = sandbox.status("b");
    assert_eq!(status["ready"], true, "{status}");
    assert_eq!(
        status["pid"], b_pid,
        "root b's host was restarted or replaced"
    );
    let panes = sandbox.run("b", &["pane", "list"]);
    assert!(panes.status.success());
    assert_eq!(
        std::fs::read_to_string(sandbox.profile("b").join("sentinel")).unwrap(),
        "b"
    );
}

/// A default-profile host launched directly with a `PLEXI_CHANNEL` (and a
/// build marker) in its environment passes neither to the process its pane
/// runs: the pane sees only the host's own routing values.
#[test]
#[ignore = "starts real hosts; run with --ignored"]
fn a_hosts_inherited_channel_never_reaches_its_panes() {
    let sandbox = Sandbox::new();
    // A bare host (default profile) so its panes carry no PLEXI_CHANNEL.
    let bare = sandbox.path.join("bin/plexi");
    std::fs::copy(&sandbox.binary, &bare).unwrap();
    let dump = sandbox.path.join("pane-env.txt");
    let mut host = sandbox.command_for(
        &bare,
        "a",
        &[
            ("PLEXI_CHANNEL", "poison1695"),
            ("PLEXI_BUILD_CHANNEL", "poison1695"),
            ("PLEXI_BACKGROUND_HOST", "1"),
            ("PLEXI_EPHEMERAL_SESSION", "1"),
        ],
    );
    let mut child = host
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let endpoint = sandbox.root("a").join(".plexi/notify.sock");
    let deadline = Instant::now() + Duration::from_secs(90);
    while !endpoint.exists() {
        assert!(Instant::now() < deadline, "the host never listened");
        assert!(child.try_wait().unwrap().is_none(), "the host exited");
        std::thread::sleep(Duration::from_millis(200));
    }
    let script = format!("env > {}; sleep 30", dump.display());
    let output = sandbox
        .command_for(&bare, "a", &[])
        .args(["pane", "new", "--no-focus", &script])
        .output()
        .unwrap();
    eprintln!(
        "pane new: {} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success());
    let deadline = Instant::now() + Duration::from_secs(30);
    let env = loop {
        if let Ok(text) = std::fs::read_to_string(&dump) {
            if text.contains("PLEXI_SOCKET=") {
                break text;
            }
        }
        assert!(
            Instant::now() < deadline,
            "the pane never wrote its environment"
        );
        std::thread::sleep(Duration::from_millis(200));
    };
    let _ = sandbox
        .command_for(&bare, "a", &[])
        .args(["host", "stop"])
        .output();
    let _ = child.wait();
    assert!(
        !env.contains("poison1695"),
        "an inherited value reached the pane:\n{env}"
    );
    assert!(
        !env.lines().any(|line| line.starts_with("PLEXI_CHANNEL=")),
        "a default-profile pane carries no PLEXI_CHANNEL:\n{env}"
    );
    assert!(env.contains("PLEXI_BUILD_CHANNEL=stable"), "{env}");
    assert!(
        env.contains(&format!(
            "PLEXI_SOCKET={}",
            sandbox.root("a").join(".plexi/notify.sock").display()
        )),
        "{env}"
    );
}

/// Design 1695 (test 7): a scratch-root host whose `HOME` is an account-like
/// sentinel writes its channel-neutral state under the root, never `$HOME`.
#[test]
#[ignore = "starts real hosts; run with --ignored"]
fn shared_state_follows_the_root_not_home() {
    let sandbox = Sandbox::new();
    let sentinel = sandbox.path.join("sentinel-home");
    std::fs::create_dir_all(&sentinel).unwrap();
    let home = sentinel.to_str().unwrap();
    let output = sandbox
        .command("a", &[("HOME", home)])
        .args([
            "host",
            "start",
            "--background",
            "--ephemeral",
            "--timeout-secs",
            "90",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let hook = sandbox
        .root("a")
        .join(".plexi/hooks/claude-code-agent-state.sh");
    let deadline = Instant::now() + Duration::from_secs(20);
    while !hook.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = sandbox
        .command("a", &[("HOME", home)])
        .args(["host", "stop"])
        .output();
    assert!(hook.is_file(), "the shared hook is not under the root");
    // The default context's workspace state follows HOME until stint 1682
    // gives panes and contexts the account home; the shared tier does not.
    assert!(
        !sentinel.join(".plexi/hooks").exists(),
        "shared state reached HOME"
    );
}

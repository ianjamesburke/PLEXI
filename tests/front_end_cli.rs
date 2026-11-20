//! The CLI front end's contract at the binary boundary.
//!
//! Every help, version and usage-error invocation exits with its contract
//! code and stream, opens no profile `config.toml` and connects to no socket.
//! The probes observe each effect independently: `config.toml` is a FIFO
//! whose open is seen by a writer thread, and the command socket is a live
//! listener whose accepts are counted. A valid experiment-dependent command is
//! the positive control for both probes.
#![cfg(unix)]

use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

const PROFILE: &str = "t1663";

struct Probe {
    home: tempfile::TempDir,
    bin_dir: tempfile::TempDir,
    /// The command socket every channel-free invocation would reach.
    socket: PathBuf,
}

struct Observed {
    output: Output,
    config_opens: usize,
    accepts: usize,
}

impl Probe {
    fn new() -> Self {
        let home = tempfile::tempdir().unwrap();
        let bin_dir = tempfile::tempdir().unwrap();
        let socket = home.path().join("cmd.sock");
        Self {
            home,
            bin_dir,
            socket,
        }
    }

    /// The selected profile's directory with `config.toml` as a FIFO.
    fn fifo_profile(&self, name: &str) -> PathBuf {
        let dir = self.home.path().join(format!(".plexi-{name}"));
        std::fs::create_dir_all(&dir).unwrap();
        let config = dir.join("config.toml");
        if !config.exists() {
            let c = std::ffi::CString::new(config.as_os_str().as_encoded_bytes()).unwrap();
            // SAFETY: c is a valid NUL-terminated path.
            assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
        }
        dir
    }

    /// A channel-named entry point for the binary under test.
    fn channel_binary(&self, name: &str) -> PathBuf {
        let link = self.bin_dir.path().join(name);
        if !link.exists() {
            std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_plexi"), &link).unwrap();
        }
        link
    }

    /// Every profile an invocation could read: the one under test, the bare
    /// binary's default, and each name the argv passes to `--profile`.
    fn watched_profiles(&self, profile_dir: &Path, args: &[&str]) -> Vec<PathBuf> {
        let mut dirs = vec![profile_dir.to_path_buf(), self.fifo_profile_dir(None)];
        for (i, arg) in args.iter().enumerate() {
            let name = match arg.strip_prefix("--profile=") {
                Some(name) => Some(name),
                None if *arg == "--profile" => args.get(i + 1).copied(),
                None => None,
            };
            if let Some(name) =
                name.filter(|n| !n.is_empty() && !n.contains('/') && !n.starts_with('-'))
            {
                dirs.push(self.fifo_profile_dir(Some(name)));
            }
        }
        dirs.sort();
        dirs.dedup();
        dirs
    }

    fn fifo_profile_dir(&self, name: Option<&str>) -> PathBuf {
        match name {
            Some(name) => self.fifo_profile(name),
            None => {
                let dir = self.home.path().join(".plexi");
                std::fs::create_dir_all(&dir).unwrap();
                let config = dir.join("config.toml");
                if !config.exists() {
                    let c = std::ffi::CString::new(config.as_os_str().as_encoded_bytes()).unwrap();
                    // SAFETY: c is a valid NUL-terminated path.
                    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
                }
                dir
            }
        }
    }

    fn run(&self, binary: &Path, profile_dir: &Path, config: &str, args: &[&str]) -> Observed {
        let watched = self.watched_profiles(profile_dir, args);
        // The ambient socket and every watched profile's own endpoint: an
        // explicit `--profile` addresses that profile's host (stint 1655).
        let sockets: Vec<PathBuf> = std::iter::once(self.socket.clone())
            .chain(watched.iter().map(|dir| dir.join("notify.sock")))
            .collect();
        let listeners: Vec<UnixListener> = sockets
            .iter()
            .map(|socket| {
                let listener = UnixListener::bind(socket).unwrap();
                listener.set_nonblocking(true).unwrap();
                listener
            })
            .collect();
        let done = Arc::new(AtomicBool::new(false));
        let opens = Arc::new(AtomicUsize::new(0));
        let accepts = Arc::new(AtomicUsize::new(0));
        let fifos: Vec<PathBuf> = watched
            .iter()
            .map(|dir| dir.join("config.toml"))
            .filter(|config| {
                use std::os::unix::fs::FileTypeExt;
                assert!(config.exists(), "no probe file at {}", config.display());
                std::fs::metadata(config).is_ok_and(|m| m.file_type().is_fifo())
            })
            .collect();
        // A blocking open for writing returns once the binary opens a FIFO to
        // read it: that is the observed read. The probe ends by opening each
        // read side itself, after `done`, which is not counted.
        let fifo_watchers: Vec<_> = fifos
            .iter()
            .map(|fifo| {
                let (done, opens, fifo) = (done.clone(), opens.clone(), fifo.clone());
                let config = config.to_string();
                std::thread::spawn(move || {
                    use std::io::Write;
                    while let Ok(mut writer) = std::fs::OpenOptions::new().write(true).open(&fifo) {
                        if done.load(Ordering::SeqCst) {
                            break;
                        }
                        opens.fetch_add(1, Ordering::SeqCst);
                        let _ = writer.write_all(config.as_bytes());
                    }
                })
            })
            .collect();
        // Accepts until `done`, then drains whatever is still queued.
        let socket_watcher = {
            let (done, accepts) = (done.clone(), accepts.clone());
            std::thread::spawn(move || loop {
                let mut accepted = false;
                for listener in &listeners {
                    if let Ok((stream, _)) = listener.accept() {
                        accepts.fetch_add(1, Ordering::SeqCst);
                        accepted = true;
                        drop(stream);
                    }
                }
                if accepted {
                    continue;
                }
                if done.load(Ordering::SeqCst) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(2));
            })
        };
        let stop = move || {
            done.store(true, Ordering::SeqCst);
            for (fifo, watcher) in fifos.iter().zip(fifo_watchers) {
                use std::os::unix::fs::OpenOptionsExt;
                let _unblock = std::fs::OpenOptions::new()
                    .read(true)
                    .custom_flags(libc::O_NONBLOCK)
                    .open(fifo);
                watcher.join().unwrap();
            }
            socket_watcher.join().unwrap();
        };
        let mut command = Command::new(binary);
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("PLEXI_") {
                command.env_remove(key);
            }
        }
        let child = command
            .args(args)
            .current_dir(self.home.path())
            .env("HOME", self.home.path())
            .env("PLEXI_SOCKET", &self.socket)
            .env("NO_COLOR", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        // Collect output on another thread so a large help text cannot fill
        // the pipe while the bound is being enforced here.
        let pid = child.id();
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || sender.send(child.wait_with_output()));
        let output = match receiver.recv_timeout(Duration::from_secs(60)) {
            Ok(output) => output.unwrap(),
            Err(_) => {
                // SAFETY: pid is our own child, not yet reaped.
                unsafe { libc::kill(pid as i32, libc::SIGKILL) };
                stop();
                panic!("{args:?} did not exit within the bound");
            }
        };
        stop();
        for socket in &sockets {
            std::fs::remove_file(socket).unwrap();
        }
        Observed {
            output,
            config_opens: opens.load(Ordering::SeqCst),
            accepts: accepts.load(Ordering::SeqCst),
        }
    }
}

// `config_version` keeps config migration from rewriting the FIFO;
// `core_only` keeps the positive control's profile initialization offline.
const EXPERIMENTS_OFF: &str = "config_version = 1\ncore_only = true\n[experiments]\n";
const ASSISTANT_ON: &str =
    "config_version = 1\ncore_only = true\n[experiments]\nassistant = true\n";

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn assert_contactless(observed: &Observed, args: &[&str]) {
    assert_eq!(observed.config_opens, 0, "{args:?} opened config.toml");
    assert_eq!(observed.accepts, 0, "{args:?} contacted the socket");
}

fn assert_usage_error(observed: &Observed, args: &[&str], needle: &str) {
    let stderr = text(&observed.output.stderr);
    assert_eq!(observed.output.status.code(), Some(2), "{args:?}: {stderr}");
    assert!(observed.output.stdout.is_empty(), "{args:?} wrote stdout");
    assert!(stderr.contains(needle), "{args:?}: {stderr}");
    assert_contactless(observed, args);
}

fn assert_display(observed: &Observed, args: &[&str]) -> String {
    let stdout = text(&observed.output.stdout);
    assert_eq!(
        observed.output.status.code(),
        Some(0),
        "{args:?}: {}",
        text(&observed.output.stderr)
    );
    assert!(!stdout.is_empty(), "{args:?} printed nothing");
    assert_contactless(observed, args);
    stdout
}

#[test]
fn usage_errors_exit_2_with_no_profile_read_and_no_contact() {
    let probe = Probe::new();
    let profile = probe.fifo_profile(PROFILE);
    let binary = PathBuf::from(env!("CARGO_BIN_EXE_plexi"));
    let cases: &[(&[&str], &str)] = &[
        (
            &["--version", "--profile", "one", "host", "--profile", "two"],
            "--profile",
        ),
        (
            &["--profile", "one", "host", "--help", "--profile", "two"],
            "--profile",
        ),
        (&["--profile", "one", "--version", "--profile"], "--profile"),
        (
            &[
                "--profile",
                "one",
                "assistant",
                "permission",
                "list",
                "--profile",
                "two",
            ],
            "--profile",
        ),
        (
            &[
                "--profile",
                "one",
                "assistant",
                "permission",
                "list",
                "--profile",
                "one",
            ],
            "--profile",
        ),
        (
            &["--profile", "Default", "host", "status"],
            ".plexi-Default",
        ),
        (&["host", "stop", "--profile"], "--profile"),
        (&["--render", "--profile"], "--render"),
        (&["pane", "send", "--help", "--bogus"], "--bogus"),
        (&["--version", "host", "--json=1"], "--json"),
        (&["help", "no-such-command"], "no-such-command"),
        (&["agent"], "Usage:"),
        (&["agent", "--json"], "Usage:"),
        (&["pnae"], "Did you mean `pane`?"),
    ];
    for config in [EXPERIMENTS_OFF, ASSISTANT_ON] {
        for (args, needle) in cases {
            let mut argv = vec!["--profile", PROFILE];
            if args.contains(&"--profile") {
                argv.clear();
            }
            argv.extend(args.iter());
            let observed = probe.run(&binary, &profile, config, &argv);
            assert_usage_error(&observed, &argv, needle);
        }
    }
}

#[test]
fn display_exits_0_with_no_profile_read_and_no_contact() {
    let probe = Probe::new();
    let profile = probe.fifo_profile(PROFILE);
    let binary = PathBuf::from(env!("CARGO_BIN_EXE_plexi"));
    let run =
        |args: &[&str]| assert_display(&probe.run(&binary, &profile, ASSISTANT_ON, args), args);

    assert_eq!(
        run(&["help", "host", "--profile", PROFILE]),
        run(&["--profile", PROFILE, "help", "host"])
    );
    assert_eq!(
        run(&["--version", "host"]),
        run(&["--version", "host", "--json"])
    );
    assert_eq!(
        run(&["--profile", PROFILE, "pane", "--help", "send"]),
        run(&["--profile", PROFILE, "pane", "--help"])
    );
    for args in [
        &["--profile", PROFILE, "--version"][..],
        &["--profile", PROFILE, "--help"],
        &["--profile", PROFILE, "-h"],
        &["--profile", PROFILE, "help"],
        &["--profile", PROFILE, "agent", "--help"],
        &["--profile", PROFILE, "assistant", "--help"],
        &["--profile", PROFILE, "completions", "zsh"],
    ] {
        run(args);
    }
}

/// The positive control: a valid command that an experiment owns reads the
/// selected profile, and with the experiment on it reaches the socket.
#[test]
fn a_valid_experiment_command_reads_the_profile() {
    let probe = Probe::new();
    let profile = probe.fifo_profile(PROFILE);
    let binary = PathBuf::from(env!("CARGO_BIN_EXE_plexi"));
    let args = ["--profile", PROFILE, "assistant", "permission", "list"];
    let off = probe.run(&binary, &profile, EXPERIMENTS_OFF, &args);
    assert_eq!(
        off.output.status.code(),
        Some(1),
        "{}",
        text(&off.output.stderr)
    );
    assert!(text(&off.output.stderr).contains("plexi experiments enable assistant"));
    assert!(off.config_opens > 0, "the control never read config.toml");
    // The socket control reads an ordinary config file: a FIFO serves one
    // open per writer pass, and repeated reads can race that pass.
    let enabled = probe.home.path().join(".plexi-t1663on");
    std::fs::create_dir_all(&enabled).unwrap();
    std::fs::write(enabled.join("config.toml"), ASSISTANT_ON).unwrap();
    let args = ["--profile", "t1663on", "assistant", "permission", "list"];
    let on = probe.run(&binary, &enabled, "", &args);
    assert!(
        on.accepts > 0,
        "the enabled control never reached the socket: exit {:?} {}",
        on.output.status.code(),
        text(&on.output.stderr)
    );
}

/// A channel-named binary names itself in every usage line, and public output
/// does not depend on what the profile enables.
#[test]
fn a_channel_named_binary_renders_its_own_name_and_the_public_surface() {
    let probe = Probe::new();
    let binary = probe.channel_binary("plexi-zz1663");
    let profile = probe.fifo_profile("zz1663");
    let agent = probe.run(&binary, &profile, EXPERIMENTS_OFF, &["agent"]);
    let agent_json = probe.run(&binary, &profile, EXPERIMENTS_OFF, &["agent", "--json"]);
    let agent_help = probe.run(&binary, &profile, EXPERIMENTS_OFF, &["agent", "--help"]);
    assert_usage_error(&agent, &["agent"], "Usage: plexi-zz1663 agent");
    assert_usage_error(
        &agent_json,
        &["agent", "--json"],
        "Usage: plexi-zz1663 agent",
    );
    let help = assert_display(&agent_help, &["agent", "--help"]);
    for output in [
        text(&agent.output.stderr),
        text(&agent_json.output.stderr),
        help,
    ] {
        for hidden in ["team", "fleet"] {
            assert!(
                !output
                    .lines()
                    .any(|line| line.trim_start().starts_with(hidden)),
                "advertised {hidden}:\n{output}"
            );
        }
    }
    let host = probe.run(&binary, &profile, EXPERIMENTS_OFF, &["host", "--json"]);
    assert_usage_error(&host, &["host", "--json"], "Usage: plexi-zz1663 host");
    let root = probe.run(&binary, &profile, EXPERIMENTS_OFF, &["--help"]);
    assert!(assert_display(&root, &["--help"]).contains("plexi-zz1663 [OPTIONS] [COMMAND]"));
    let version = probe.run(&binary, &profile, EXPERIMENTS_OFF, &["--version"]);
    assert!(assert_display(&version, &["--version"]).starts_with("plexi-zz1663 "));

    // Identical public output whatever the profile enables: a readable
    // config with every experiment on changes no byte.
    let all_on = probe.home.path().join(".plexi-zz1664");
    std::fs::create_dir_all(&all_on).unwrap();
    std::fs::write(
        all_on.join("config.toml"),
        "config_version = 1\ncore_only = true\n[experiments]\nassistant = true\nagent_mesh = true\nledger = true\n",
    )
    .unwrap();
    let on_binary = probe.channel_binary("plexi-zz1664");
    for args in [&["--help"][..], &["agent", "--help"], &["agent"]] {
        let off = probe.run(&binary, &profile, EXPERIMENTS_OFF, args);
        let on = probe.run(&on_binary, &all_on, "", args);
        let normalize = |bytes: &[u8]| text(bytes).replace("plexi-zz1664", "plexi-zz1663");
        assert_eq!(
            normalize(&on.output.stdout),
            normalize(&off.output.stdout),
            "{args:?}"
        );
        assert_eq!(
            normalize(&on.output.stderr),
            normalize(&off.output.stderr),
            "{args:?}"
        );
    }
}

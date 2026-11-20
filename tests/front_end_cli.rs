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

    fn run(&self, binary: &Path, profile_dir: &Path, config: &str, args: &[&str]) -> Observed {
        let fifo = profile_dir.join("config.toml");
        let listener = UnixListener::bind(&self.socket).unwrap();
        listener.set_nonblocking(true).unwrap();
        let done = Arc::new(AtomicBool::new(false));
        let opens = Arc::new(AtomicUsize::new(0));
        let accepts = Arc::new(AtomicUsize::new(0));
        let is_fifo = {
            use std::os::unix::fs::FileTypeExt;
            std::fs::metadata(&fifo).is_ok_and(|m| m.file_type().is_fifo())
        };
        // A blocking open for writing returns once the binary opens the FIFO
        // to read it: that is the observed read. The probe ends by opening the
        // read side itself, after `done`, which is not counted.
        assert!(
            is_fifo || fifo.is_file(),
            "no probe file at {}",
            fifo.display()
        );
        let fifo_watcher = is_fifo.then(|| {
            let (done, opens, fifo) = (done.clone(), opens.clone(), fifo.clone());
            let config = config.to_string();
            std::thread::spawn(move || {
                use std::io::Write;
                loop {
                    let opened = std::fs::OpenOptions::new().write(true).open(&fifo);
                    let Ok(mut writer) = opened else { break };
                    if done.load(Ordering::SeqCst) {
                        break;
                    }
                    opens.fetch_add(1, Ordering::SeqCst);
                    let _ = writer.write_all(config.as_bytes());
                }
            })
        });
        let socket_watcher = {
            let (done, accepts) = (done.clone(), accepts.clone());
            std::thread::spawn(move || {
                while !done.load(Ordering::SeqCst) {
                    if let Ok((stream, _)) = listener.accept() {
                        accepts.fetch_add(1, Ordering::SeqCst);
                        drop(stream);
                    }
                    std::thread::sleep(Duration::from_millis(2));
                }
            })
        };
        let stop = move || {
            done.store(true, Ordering::SeqCst);
            if let Some(watcher) = fifo_watcher {
                use std::os::unix::fs::OpenOptionsExt;
                let _unblock = std::fs::OpenOptions::new()
                    .read(true)
                    .custom_flags(libc::O_NONBLOCK)
                    .open(&fifo);
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
        std::fs::remove_file(&self.socket).unwrap();
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
    let on = probe.run(&binary, &profile, ASSISTANT_ON, &args);
    assert!(on.config_opens > 0, "the control never read config.toml");
    assert!(
        on.accepts > 0,
        "the enabled control never reached the socket"
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

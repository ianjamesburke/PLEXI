//! Host-side self-validation for `scripts/cargo-with-lease.sh` — gutting the
//! host-arbitrated build lease. The wrapper used to ask a running Plexi
//! host for a named lease over IPC (`plexi lock acquire/release/status`);
//! ownership was keyed to the PANE, not the process, so a client that died
//! (Ctrl-C, SIGKILL, timeout) left the pane registered as a permanent
//! holder and wedged every later build in that pane. The rewrite replaces
//! that with a plain kernel `flock`, which self-releases on any process
//! death including SIGKILL.
//!
//! These tests invoke the real script with `std::process::Command`, exactly
//! as every justfile call site does, pointed at a temp `PLEXI_CARGO_LOCK`
//! so runs stay isolated from the machine's real lock file and from each
//! other under parallel test execution.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn script_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/cargo-with-lease.sh")
}

/// A fresh temp lock path, not yet created — the wrapper creates it.
fn temp_lock_path(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "plexi-cargo-lease-test-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir for lock");
    dir.join("cargo-build.lock")
}

// ─── Exit code propagation ─────────────────────────────────────────────────

/// The wrapper must run the wrapped command and exit with its exact code,
/// both for success and for a nonzero failure.
#[test]
fn cargo_lease_propagates_exit_code() {
    let lock_path = temp_lock_path("exit-code");

    let ok = Command::new("bash")
        .arg(script_path())
        .arg("true")
        .env("PLEXI_CARGO_LOCK", &lock_path)
        .status()
        .expect("run wrapper with true");
    assert!(ok.success(), "wrapper must succeed when the command succeeds");

    let fail = Command::new("bash")
        .arg(script_path())
        .arg("false")
        .env("PLEXI_CARGO_LOCK", &lock_path)
        .status()
        .expect("run wrapper with false");
    assert_eq!(
        fail.code(),
        Some(1),
        "wrapper must propagate the wrapped command's nonzero exit code"
    );
}

// ─── Serialization ──────────────────────────────────────────────────────────

/// Runs the wrapper around a shell snippet that appends a start marker,
/// sleeps, then appends an end marker to `marker_path` — used to prove two
/// concurrent wrapper invocations never overlap.
fn run_marked(script: &Path, lock_path: &Path, marker_path: &Path, label: &str) {
    let inline = format!(
        "echo start-{label} >> {marker}; sleep 0.3; echo end-{label} >> {marker}",
        label = label,
        marker = marker_path.display()
    );
    let status = Command::new("bash")
        .arg(script)
        .arg("bash")
        .arg("-c")
        .arg(inline)
        .env("PLEXI_CARGO_LOCK", lock_path)
        .status()
        .expect("run marked wrapper invocation");
    assert!(status.success(), "marked invocation {label} must succeed");
}

/// Two concurrent wrapper invocations around a shared lock must never have
/// their start/end intervals interleave — the whole point of the lock as a
/// build throttle.
#[test]
fn cargo_lease_serializes_concurrent_invocations() {
    let lock_path = temp_lock_path("serialize");
    let marker_path = temp_lock_path("serialize-marker"); // reused as a plain temp file path

    let script_a = script_path();
    let lock_a = lock_path.clone();
    let marker_a = marker_path.clone();
    let handle_a =
        std::thread::spawn(move || run_marked(&script_a, &lock_a, &marker_a, "a"));

    // Give the first invocation a head start so ordering is deterministic
    // enough to reason about, though the assertion below holds regardless
    // of which one wins the lock first.
    std::thread::sleep(Duration::from_millis(50));

    let script_b = script_path();
    let lock_b = lock_path.clone();
    let marker_b = marker_path.clone();
    let handle_b =
        std::thread::spawn(move || run_marked(&script_b, &lock_b, &marker_b, "b"));

    handle_a.join().expect("thread a");
    handle_b.join().expect("thread b");

    let contents = std::fs::read_to_string(&marker_path).expect("read marker file");
    let mut open_label: Option<&str> = None;
    for line in contents.lines() {
        if let Some(label) = line.strip_prefix("start-") {
            assert!(
                open_label.is_none(),
                "invocation {label} started while {:?} was still running; \
                 intervals interleaved:\n{contents}",
                open_label
            );
            open_label = Some(label);
        } else if let Some(label) = line.strip_prefix("end-") {
            assert_eq!(
                open_label,
                Some(label),
                "invocation {label} ended without a matching open start; \
                 intervals interleaved:\n{contents}"
            );
            open_label = None;
        } else {
            panic!("unexpected marker line: {line:?}");
        }
    }
    assert!(open_label.is_none(), "an invocation never closed: {contents}");
}

// ─── SIGKILL self-release ───────────────────────────────────────────────────

/// A SIGKILLed holder must not wedge the lock: killing the process that
/// holds the flock must release it immediately (kernel-level, not via a
/// bash trap — traps never fire on SIGKILL), so a fresh invocation acquires
/// promptly instead of waiting out the full timeout.
#[test]
fn cargo_lease_sigkilled_holder_does_not_wedge_the_lock() {
    let lock_path = temp_lock_path("sigkill");

    // `exec python3 ...` at the top of the script replaces the bash process
    // image in place, so this Child's pid is the actual lock-holding
    // process for its whole lifetime — killing it directly exercises "the
    // holder dies by SIGKILL", not just "the wrapped child dies".
    let mut holder = Command::new("bash")
        .arg(script_path())
        .arg("sleep")
        .arg("30")
        .env("PLEXI_CARGO_LOCK", &lock_path)
        .spawn()
        .expect("spawn long-running holder");

    // Give it time to acquire the lock before killing it.
    std::thread::sleep(Duration::from_millis(500));

    holder.kill().expect("SIGKILL the holder");
    let _ = holder.wait();

    let started = Instant::now();
    let status = Command::new("bash")
        .arg(script_path())
        .arg("true")
        .env("PLEXI_CARGO_LOCK", &lock_path)
        .env("PLEXI_CARGO_LEASE_TIMEOUT_SECS", "10")
        .status()
        .expect("run fresh invocation after killing the holder");
    let elapsed = started.elapsed();

    assert!(status.success(), "fresh invocation must succeed");
    assert!(
        elapsed < Duration::from_secs(5),
        "fresh invocation took {elapsed:?} — the lock appears wedged after \
         SIGKILLing its holder instead of self-releasing"
    );
}

// ─── Nesting guard ───────────────────────────────────────────────────────────

/// When `PLEXI_CARGO_LOCK_HELD` is already set (a nested wrapper call, e.g.
/// `just pr-install` invoking a script that itself calls this wrapper), the
/// inner invocation must run the command directly without trying to
/// re-acquire the lock its own ancestor holds — otherwise it deadlocks.
#[test]
fn cargo_lease_nested_invocation_skips_the_lock() {
    let lock_path = temp_lock_path("nesting");
    let mut ready_marker = temp_lock_path("nesting-ready");
    ready_marker.set_file_name("ready-marker");
    let _ = std::fs::remove_file(&ready_marker);

    // Hold the real lock in the background: this outer invocation writes a
    // ready marker once it has (almost certainly) acquired the lock, then
    // sleeps well past the nested call's own duration.
    let inline = format!(
        "touch {ready}; sleep 5",
        ready = ready_marker.display()
    );
    let mut outer_holder = Command::new("bash")
        .arg(script_path())
        .arg("bash")
        .arg("-c")
        .arg(inline)
        .env("PLEXI_CARGO_LOCK", &lock_path)
        .spawn()
        .expect("spawn outer holder");

    let wait_started = Instant::now();
    while !ready_marker.exists() {
        assert!(
            wait_started.elapsed() < Duration::from_secs(5),
            "outer holder never acquired the lock"
        );
        std::thread::sleep(Duration::from_millis(20));
    }

    // A nested call with the marker set must return immediately, proving it
    // never contended for the still-held outer lock.
    let started = Instant::now();
    let nested = Command::new("bash")
        .arg(script_path())
        .arg("true")
        .env("PLEXI_CARGO_LOCK", &lock_path)
        .env("PLEXI_CARGO_LOCK_HELD", "1")
        .status()
        .expect("run nested invocation");
    let elapsed = started.elapsed();

    assert!(nested.success(), "nested invocation must succeed");
    assert!(
        elapsed < Duration::from_secs(1),
        "nested invocation took {elapsed:?} — it appears to have waited on \
         the lock instead of skipping it via PLEXI_CARGO_LOCK_HELD"
    );

    let _ = outer_holder.kill();
    let _ = outer_holder.wait();
}

// ─── Stdin passthrough ───────────────────────────────────────────────────────

/// The wrapped command must inherit the caller's real stdin, not the
/// wrapper's internal python heredoc. Regression guard for a defect where
/// `exec python3 - "$@" <<'PY'` made python's stdin the heredoc itself, so
/// `subprocess.run` handed an already-consumed pipe to the wrapped command —
/// anything reading stdin (a codesign/keychain prompt, a cargo prompt, an
/// interactive command) would silently see EOF.
#[test]
fn cargo_lease_wrapped_command_inherits_real_stdin() {
    let lock_path = temp_lock_path("stdin");

    let mut child = Command::new("bash")
        .arg(script_path())
        .arg("cat")
        .env("PLEXI_CARGO_LOCK", &lock_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn wrapper around cat");

    let payload = b"hello from the real caller stdin\n";
    child
        .stdin
        .take()
        .expect("child stdin")
        .write_all(payload)
        .expect("write to wrapper stdin");

    let mut output = child.wait_with_output().expect("wait for wrapper");
    assert!(output.status.success(), "wrapper around cat must succeed");
    // Normalize in case the pipe delivers in more than one read (not
    // expected here, but wait_with_output already collects everything).
    let mut got = Vec::new();
    got.append(&mut output.stdout);
    assert_eq!(
        got.as_slice(),
        payload.as_slice(),
        "wrapped `cat` must echo back exactly what was piped into the wrapper's stdin"
    );
}

// ─── Signal exit-code mapping ────────────────────────────────────────────────

/// `subprocess.run` reports death-by-signal as a NEGATIVE returncode (Python
/// convention: -9 for SIGKILL); the wrapper must translate that to the shell
/// convention (128 + signum) so callers see the same exit code a bare
/// `exec "$@"` would have produced. Kills the WRAPPED command from within
/// itself (distinct from `cargo_lease_sigkilled_holder_does_not_wedge_the_lock`,
/// which kills the lock-holding wrapper process).
#[test]
fn cargo_lease_sigkilled_wrapped_command_reports_137() {
    let lock_path = temp_lock_path("signal-exit-code");

    let status = Command::new("bash")
        .arg(script_path())
        .arg("bash")
        .arg("-c")
        .arg("kill -9 $$")
        .env("PLEXI_CARGO_LOCK", &lock_path)
        .status()
        .expect("run wrapper around a self-SIGKILLing command");

    assert_eq!(
        status.code(),
        Some(137),
        "a SIGKILLed wrapped command must report exit code 128+9=137, got {status:?}"
    );
}

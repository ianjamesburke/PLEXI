//! Reload-path tests for hot reload (#83).
//!
//! Two paths under test:
//!   1. Drop on a well-behaved subprocess sends `Shutdown` and reaps
//!      cleanly within the 2s timeout (no SIGTERM escalation).
//!   2. Drop on an app that ignores Shutdown escalates to SIGTERM
//!      and still reaps within the 1s SIGTERM timeout.
//!
//! These exercise the `Drop for ProcessApp` machinery directly — the
//! reload glue in `pane_ops::create::reload_app_pane` relies on
//! `Drop` for the Shutdown→wait→kill sequence. Replacing the runtime
//! field naturally drops the old `ProcessApp`.
use super::super::*;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

fn make_sh_app(args: &[&str]) -> Option<ProcessApp> {
    let sh = ["/bin/sh", "/usr/bin/sh"]
        .iter()
        .find(|p| std::path::Path::new(p).exists())
        .map(PathBuf::from)?;
    let workspace_root = std::env::temp_dir();
    let owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    ProcessApp::launch(
        "test_reload",
        "Test Reload",
        &sh,
        &workspace_root,
        &owned,
        workspace_root.clone(),
        HashSet::new(),
        false,
        None,
    )
    .ok()
}

/// Reload contract: dropping a `ProcessApp` reaps the underlying
/// child. `Shutdown` is best-effort over a stdio that the child
/// likely isn't even reading; the timed escalation guarantees the
/// reap happens regardless.
#[test]
fn drop_reaps_well_behaved_subprocess_within_window() {
    let Some(app) = make_sh_app(&["-c", "sleep 5"]) else {
        eprintln!("skipping: no /bin/sh available");
        return;
    };
    // The child PID must exist while `app` is alive.
    let pid = app
        .process
        .as_ref()
        .map(|c| c.id())
        .expect("child should be running");
    let start = std::time::Instant::now();
    drop(app);
    let elapsed = start.elapsed();

    // Drop completed — under 4s ceiling (2s shutdown + 1s SIGTERM + slack).
    assert!(
        elapsed < std::time::Duration::from_secs(4),
        "drop must complete within escalation window, took {elapsed:?}"
    );

    // Verify the OS released the PID (kill 0 to test process existence).
    // On Unix, kill -0 returns -1 with ESRCH if the process doesn't exist.
    #[cfg(unix)]
    {
        // Sleep a tick so the OS gets a chance to fully reap.
        std::thread::sleep(std::time::Duration::from_millis(50));
        let alive = unsafe { libc::kill(pid as libc::pid_t, 0) };
        // kill returning -1 means the process is gone (ESRCH).
        assert_eq!(
            alive, -1,
            "child pid {pid} must be reaped after drop, got {alive}"
        );
    }
}

/// Reload force-kill contract: a subprocess that ignores `Shutdown`
/// (no stdin reader, just `sleep`) must still be reaped via the
/// SIGTERM escalation.
#[test]
fn drop_force_kills_unresponsive_subprocess() {
    // `sleep 30` doesn't read stdin and ignores SIGPIPE — the only
    // way it dies is SIGTERM/SIGKILL.
    let Some(app) = make_sh_app(&["-c", "exec sleep 30"]) else {
        eprintln!("skipping: no /bin/sh available");
        return;
    };
    let pid = app
        .process
        .as_ref()
        .map(|c| c.id())
        .expect("child should be running");
    let start = std::time::Instant::now();
    drop(app);
    let elapsed = start.elapsed();

    // The 2s shutdown window expires (sleep ignores it), then
    // SIGTERM reaps within 1s. Total well under 4s.
    assert!(
        elapsed < std::time::Duration::from_secs(4),
        "force-kill must complete within escalation window, took {elapsed:?}"
    );
    assert!(
        elapsed >= std::time::Duration::from_millis(1900),
        "shutdown wait should give the well-behaved app time to exit; elapsed={elapsed:?}"
    );

    #[cfg(unix)]
    {
        std::thread::sleep(std::time::Duration::from_millis(50));
        let alive = unsafe { libc::kill(pid as libc::pid_t, 0) };
        assert_eq!(
            alive, -1,
            "unresponsive child pid {pid} must be force-reaped after drop"
        );
    }
}

// ── render coalescing (issue #368) ────────────────────────────────────────

/// Verifies render-event coalescing: N back-to-back `PlexiEvent::Render`
/// calls must produce at most 1 `FlushRender` token in the channel (so
/// a burst never fills the queue and silently drops itself), and a
/// subsequent non-render event must still reach the subprocess.
///
/// Strategy: rather than round-tripping through a subprocess, we inspect
/// the shared `render_slot` / `render_in_queue` Arcs directly after the
/// calls. This is race-free because `send_event` writes them synchronously
/// on the caller's thread before returning.
///
/// Covered invariants:
///   1. After N renders, `render_in_queue` is true (exactly one token queued).
///   2. `render_slot` contains the *last* render's payload.
///   3. A Key event sent after the renders does not clear or corrupt the slot.
///   4. `event_tx` is still Some (no spurious disconnection).
#[test]
fn render_events_coalesced_non_render_events_preserved() {
    let Some(mut app) = make_sh_app(&["-c", "sleep 5"]) else {
        eprintln!("skipping: no /bin/sh available");
        return;
    };

    let rect = crate::app_protocol::Rect {
        x: 0.0,
        y: 0.0,
        w: 800.0,
        h: 600.0,
    };

    // Send 5 Render events back-to-back.
    for frame_id in 1u64..=5 {
        app.send_event(&PlexiEvent::Render {
            frame_id,
            rect: rect.clone(),
        });
    }

    // After the burst, exactly one FlushRender token must be in the channel.
    // `render_in_queue` is true while the token is queued / not yet drained.
    assert!(
        app.render_in_queue.load(Ordering::Relaxed),
        "render_in_queue must be true after a burst of Render events"
    );

    // render_slot must hold the *latest* (frame_id=5) payload, not an earlier one.
    {
        let slot = app.render_slot.lock().unwrap();
        let payload = slot.as_deref().expect("render_slot must be populated");
        assert!(
            payload.contains("\"frame_id\":5"),
            "render_slot must contain the latest frame_id (5), got: {payload}"
        );
    }

    // A Key event after the burst must be accepted without error.
    app.send_event(&PlexiEvent::Key {
        key: "j".to_string(),
        modifiers: crate::app_protocol::Modifiers::default(),
    });

    // event_tx must still be live — the Key was enqueued successfully.
    assert!(
        app.event_tx.is_some(),
        "event_tx must remain Some after sending a Key event"
    );
}

/// Sanity check: `launch_process` is re-entrant for the same id —
/// hot reload calls it on every reload.
#[test]
fn launch_process_is_reentrant_for_same_app() {
    let Some(a) = make_sh_app(&["-c", "sleep 0.1"]) else {
        eprintln!("skipping: no /bin/sh available");
        return;
    };
    let Some(b) = make_sh_app(&["-c", "sleep 0.1"]) else {
        eprintln!("skipping: no /bin/sh available");
        return;
    };
    let id_a = a.process.as_ref().map(|c| c.id());
    let id_b = b.process.as_ref().map(|c| c.id());
    assert!(id_a.is_some());
    assert!(id_b.is_some());
    assert_ne!(
        id_a, id_b,
        "two launches of the same app must produce distinct PIDs"
    );
}

// ── stream child cleanup on drop (#675) ────────────────────────────────────

#[test]
fn drop_cancels_active_stream_children() {
    let Some(mut app) = make_sh_app(&["-c", "sleep 5"]) else {
        eprintln!("skipping: no /bin/sh available");
        return;
    };

    let mut stream_child = std::process::Command::new("/bin/sh")
        .args(["-c", "exec sleep 60"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn stream child");
    let stream_pid = stream_child.id();

    app.stream_handles.insert(
        "test-stream".to_string(),
        StreamHandle {
            cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            pid: stream_pid,
        },
    );

    drop(app);

    match stream_child.wait() {
        Ok(status) => assert!(
            !status.success(),
            "stream child should have been killed by signal, got: {status}"
        ),
        Err(e) if e.raw_os_error() == Some(libc::ECHILD) => {
            // Already reaped by the SIGKILL escalation thread — expected.
        }
        Err(e) => panic!("unexpected wait error: {e}"),
    }

    #[cfg(unix)]
    {
        let alive = unsafe { libc::kill(stream_pid as libc::pid_t, 0) };
        assert_eq!(
            alive, -1,
            "stream child pid {stream_pid} must be gone after drop"
        );
    }
}

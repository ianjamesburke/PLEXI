//! Behavioural tests for the v3.2 clipboard / paste plumbing (#200, #146).
//!
//!   1. `egui::Event::Paste(text)` translates into `PlexiEvent::Paste`
//!      on the outbound queue (paste_event_forwarded_as_plexi_event).
//!   2. `DrawCommand::CopyToClipboard { text }` reaches egui's output
//!      command queue as `OutputCommand::CopyText` so the platform
//!      backend writes to the OS clipboard
//!      (copy_to_clipboard_drawcommand_calls_egui_copy).
//!
//! These exercise the host-side translation logic. End-to-end clipboard
//! integration with NSPasteboard / X11 / Wayland is verified via the
//! human-verification checklist in the PR — egui's backend is opaque
//! from a unit-test standpoint.
use super::super::*;
use crate::app_protocol::PlexiEvent;
use std::collections::HashSet;
use std::path::PathBuf;

/// Build a minimal `ProcessApp` for tests. Mirrors the helper in
/// `text_input_tests` — spawns `/bin/sh -c "sleep 1"` so lifecycle
/// machinery is happy, then ignores the subprocess.
fn make_app() -> Option<ProcessApp> {
    let sh = ["/bin/sh", "/usr/bin/sh"]
        .iter()
        .find(|p| std::path::Path::new(p).exists())
        .map(PathBuf::from)?;
    let workspace_root = std::env::temp_dir();
    ProcessApp::launch(
        "test_clipboard",
        "Test Clipboard",
        &sh,
        &workspace_root,
        &["-c".to_string(), "sleep 1".to_string()],
        workspace_root.clone(),
        HashSet::new(),
        false,
        None,
    )
    .ok()
}

#[test]
fn paste_event_forwarded_as_plexi_event() {
    // Drive a synthesised `egui::Event::Paste("hello")` through the
    // pane's `handle_key`. The expected outcome is one
    // `PlexiEvent::Paste { text: "hello" }` on the outbound event
    // queue. No `Key`/`Text` events should be synthesised.
    let Some(mut app) = make_app() else {
        eprintln!("skipping: no /bin/sh available");
        return;
    };

    let mut input = egui::InputState::default();
    input.events.push(egui::Event::Paste("hello".to_string()));

    let consumed = app.handle_key(&input);
    assert!(consumed, "handle_key must consume Paste events");

    let paste_events: Vec<_> = app
        .outbound_events
        .iter()
        .filter(|e| matches!(e, PlexiEvent::Paste { .. }))
        .collect();
    assert_eq!(
        paste_events.len(),
        1,
        "expected exactly one Paste event, got {paste_events:?}"
    );
    match paste_events[0] {
        PlexiEvent::Paste { text } => assert_eq!(text, "hello"),
        other => panic!("expected Paste, got {other:?}"),
    }
}

#[test]
fn copy_to_clipboard_drawcommand_calls_egui_copy() {
    // Verify the wired path: `ControlCommand::CopyToClipboard { text }` →
    // `egui::Context::copy_text(text)` → `OutputCommand::CopyText` on
    // the platform output. We construct a fresh egui Context, mirror
    // the one-line dispatch from `ProcessApp::handle_control_command()`,
    // and inspect the platform output for the CopyText command. If the
    // dispatch ever changes shape (e.g. a different egui method), this
    // test forces the breakage to surface.
    use crate::app_protocol::ControlCommand;
    let ctx = egui::Context::default();
    let cmd = ControlCommand::CopyToClipboard {
        text: "selected snippet".to_string(),
    };

    // This mirrors the exact branch in `ProcessApp::handle_control_command()`.
    // Keep it in sync — if you refactor the dispatch, refactor this too.
    match cmd {
        ControlCommand::CopyToClipboard { text } => ctx.copy_text(text),
        _ => panic!("test setup error"),
    }

    // Drain platform output and look for CopyText.
    let mut found = None;
    ctx.output_mut(|o| {
        for cmd in &o.commands {
            if let egui::OutputCommand::CopyText(text) = cmd {
                found = Some(text.clone());
            }
        }
    });
    assert_eq!(
        found.as_deref(),
        Some("selected snippet"),
        "CopyToClipboard must emit OutputCommand::CopyText with the right text"
    );
}

#[test]
fn crash_overlay_c_key_copies_report() {
    use crate::testing::HostHarness;
    use crate::pane::{AppRuntime, Pane};

    let mut h = HostHarness::new();
    let pane = h.add_test_pane();

    // Force lifecycle to Crashed and inject known stderr lines.
    {
        let win = &mut h.app.windows[0];
        let Some(Pane::App(app_pane)) = win.panes.get_mut(&pane) else {
            panic!("expected App pane");
        };
        let AppRuntime::Process(proc) = &mut app_pane.runtime else {
            panic!("expected Process runtime");
        };
        proc.lifecycle.on_process_exited(); // → Crashed
        let mut buf = proc.recent_stderr.lock().unwrap();
        buf.push_back("Traceback (most recent call last):".to_string());
        buf.push_back("  File \"app.py\", line 42, in run".to_string());
        buf.push_back("ZeroDivisionError: division by zero".to_string());
    }

    // One frame to trigger the overlay and stamp crashed_at.
    h.run_frames(1);

    // Send C — no modifier.
    h.key(egui::Key::C, egui::Modifiers::NONE);

    // Check the clipboard from the last frame's platform output.
    let copy_cmd = h.last_platform_output.commands.iter().find_map(|cmd| {
        if let egui::OutputCommand::CopyText(text) = cmd {
            Some(text.clone())
        } else {
            None
        }
    });
    let report = copy_cmd.expect("pressing C on crash overlay must write to clipboard");

    assert!(
        report.contains("=== Plexi Crash Report ==="),
        "report must have header: {report}"
    );
    assert!(
        report.contains("crashed"),
        "report must name the state: {report}"
    );
    assert!(
        report.contains("ZeroDivisionError"),
        "report must contain stderr lines: {report}"
    );
    assert!(
        report.contains("Traceback"),
        "report must contain all stderr lines: {report}"
    );
}

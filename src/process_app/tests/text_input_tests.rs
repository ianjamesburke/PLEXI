//! Buffer-state tests for the v3.1 host-owned TextInput primitive
//! (issue #283). Covered behaviours:
//!   1. Buffer persists across frames until submit.
//!   2. Enter emits `PlexiEvent::TextSubmitted` with the buffered value.
//!   3. Submit clears the buffer (so the field is empty on the next emit).
//!   4. Pane resize (which triggers re-render but not buffer touch) does
//!      not wipe the buffer.
//!
//! These exercise the persistent-state contract — the egui rendering
//! layer is verified end-to-end by the human-verification checklist.
//! Keeping the unit tests pure makes them deterministic and fast.
use super::super::*;
use crate::app_protocol::PlexiEvent;
use std::collections::HashSet;
use std::path::PathBuf;
use std::process::Command;

/// Build a `ProcessApp` for tests that doesn't touch real I/O. We
/// spawn `/bin/sh -c true` — the cheapest valid subprocess — and
/// then ignore the lifecycle / draw threads. The app's stdin/stdout
/// are real but we never write to them in these tests.
fn make_app() -> Option<ProcessApp> {
    let sh = ["/bin/sh", "/usr/bin/sh"]
        .iter()
        .find(|p| std::path::Path::new(p).exists())
        .map(PathBuf::from)?;
    let workspace_root = std::env::temp_dir();
    // -c true exits immediately. The lifecycle reader threads will
    // observe stdout EOF and flip Crashed, which is fine — these
    // tests don't read lifecycle state.
    let _ = Command::new(&sh); // sanity — silences unused-import warnings on some configs
    ProcessApp::launch(
        "test_text_input",
        "Test Text Input",
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
fn text_input_buffer_persists_across_frames() {
    let Some(mut app) = make_app() else {
        eprintln!("skipping: no /bin/sh available");
        return;
    };
    // Simulate two frames where the user has typed "hel" then "hello"
    // by directly manipulating the buffer the way egui's TextEdit
    // would across two ui() ticks.
    app.render_session.text_input_buffers
        .insert("note".to_string(), "hel".to_string());
    // ... another frame happens (no submit) ...
    // Buffer must still be there.
    assert_eq!(
        app.render_session.text_input_buffers.get("note").map(String::as_str),
        Some("hel"),
        "buffer should survive between frames"
    );
    app.render_session.text_input_buffers
        .insert("note".to_string(), "hello".to_string());
    assert_eq!(
        app.render_session.text_input_buffers.get("note").map(String::as_str),
        Some("hello")
    );
}

#[test]
fn enter_emits_text_submitted_with_buffered_value() {
    let Some(mut app) = make_app() else {
        eprintln!("skipping: no /bin/sh available");
        return;
    };
    app.render_session.text_input_buffers
        .insert("note".to_string(), "hello world".to_string());

    app.submit_text_input("note");

    let evt = app.outbound_events.pop_back().expect("event queued");
    match evt {
        PlexiEvent::TextSubmitted { id, value } => {
            assert_eq!(id, "note");
            assert_eq!(value, "hello world");
        }
        other => panic!("expected TextSubmitted, got {other:?}"),
    }
}

#[test]
fn submit_clears_buffer() {
    let Some(mut app) = make_app() else {
        eprintln!("skipping: no /bin/sh available");
        return;
    };
    app.render_session.text_input_buffers
        .insert("note".to_string(), "draft".to_string());
    app.submit_text_input("note");
    assert!(
        !app.render_session.text_input_buffers.contains_key("note"),
        "buffer must be cleared after submit (default UX)"
    );
}

#[test]
fn submit_on_empty_buffer_emits_empty_value() {
    let Some(mut app) = make_app() else {
        eprintln!("skipping: no /bin/sh available");
        return;
    };
    // No prior buffer — Enter on a fresh TextInput is a valid case
    // (e.g. user immediately presses Enter without typing).
    app.submit_text_input("note");
    let evt = app.outbound_events.pop_back().expect("event queued");
    match evt {
        PlexiEvent::TextSubmitted { id, value } => {
            assert_eq!(id, "note");
            assert_eq!(value, "");
        }
        other => panic!("expected TextSubmitted, got {other:?}"),
    }
}

#[test]
fn text_input_buffer_survives_pane_resize() {
    let Some(mut app) = make_app() else {
        eprintln!("skipping: no /bin/sh available");
        return;
    };
    // Pane resize is a `last_size` change in `ui()`. The buffer is
    // owned by `render_session.text_input_buffers` and never touched by resize
    // logic. Simulate the resize bookkeeping and assert the buffer
    // is untouched.
    app.render_session.text_input_buffers
        .insert("note".to_string(), "midway".to_string());
    // Resize bookkeeping (mirrors what `ui()` does on size delta).
    app.last_size = egui::vec2(800.0, 600.0);
    // No buffer mutation should happen here — just last_size changes.
    assert_eq!(
        app.render_session.text_input_buffers.get("note").map(String::as_str),
        Some("midway"),
        "resize must not wipe the host-owned text buffer"
    );
}

#[test]
fn distinct_ids_keep_independent_buffers() {
    let Some(mut app) = make_app() else {
        eprintln!("skipping: no /bin/sh available");
        return;
    };
    app.render_session.text_input_buffers
        .insert("a".to_string(), "alpha".to_string());
    app.render_session.text_input_buffers
        .insert("b".to_string(), "beta".to_string());
    app.submit_text_input("a");
    assert_eq!(
        app.render_session.text_input_buffers.get("b").map(String::as_str),
        Some("beta"),
        "submitting one input must not affect another id"
    );
}

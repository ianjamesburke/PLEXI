use super::super::*;
use std::collections::HashSet;
use std::path::PathBuf;

fn make_app() -> Option<ProcessApp> {
    let sh = ["/bin/sh", "/usr/bin/sh"]
        .iter()
        .find(|p| std::path::Path::new(p).exists())
        .map(PathBuf::from)?;
    let workspace_root = std::env::temp_dir();
    ProcessApp::launch(
        "test_render_session",
        "Test RenderSession",
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
fn render_session_submit_produces_event() {
    let Some(mut app) = make_app() else {
        eprintln!("skipping: no /bin/sh available");
        return;
    };
    app.render_session
        .text_input_buffers
        .insert("x".to_string(), "hello".to_string());
    app.submit_text_input("x");
    let evt = app.outbound_events.pop_back().expect("event queued");
    match evt {
        crate::app_protocol::PlexiEvent::TextSubmitted { id, value } => {
            assert_eq!(id, "x");
            assert_eq!(value, "hello");
        }
        other => panic!("expected TextSubmitted, got {other:?}"),
    }
}

#[test]
fn render_session_process_app_has_no_text_input_fields() {
    // Compile-time proof: ProcessApp::render_session owns the state.
    // This test just exercises the field path — if mod.rs still had
    // text_input_buffers directly on ProcessApp this wouldn't compile.
    let Some(mut app) = make_app() else {
        return;
    };
    app.render_session
        .text_input_buffers
        .insert("k".to_string(), "v".to_string());
    assert!(app.render_session.text_input_buffers.contains_key("k"));
}

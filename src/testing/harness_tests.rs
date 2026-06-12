use super::*;

// -- DrawCommand routing --------------------------------------------------

/// Regression guard for PR #536: `AiQuery` was silently dropped into
/// `pending_frame` (visual buffer) instead of being routed through
/// `route_command`. Confirmed by the fact that routing state stays
/// unchanged — the command was never dispatched.
///
/// Uses a pane with no `ai.query` capability (withheld / first-run).
/// With the consent-auto-trigger fix, the withheld path defers the query
/// and queues a consent prompt instead of emitting an immediate AiResponse.
/// Either path proves the routing regression was not reintroduced: if
/// AiQuery were dropped into `pending_frame`, neither the deferred queue
/// nor the pending prompt would be populated.
#[test]
fn ai_query_reaches_route_command_not_pending_frame() {
    let mut h = HostHarness::new();
    // No capabilities (withheld) → AiQuery defers and queues consent prompt.
    let pane = h.add_test_pane_with_permissions(AppPermissions::from_capability_strings(&[]));

    h.inject(pane, ai_query("req-1"));

    // Drive routing directly via background_tick — no egui frames needed.
    {
        let win = &mut h.app.windows[0];
        let Some(Pane::App(app_pane)) = win.panes.get_mut(&pane) else {
            panic!("expected App pane");
        };
        let AppRuntime::Process(proc) = &mut app_pane.runtime else {
            panic!("expected Process runtime");
        };
        proc.background_tick();
    }

    // Withheld path: query is deferred + consent prompt queued.
    let (has_deferred, has_prompt) = {
        let win = &mut h.app.windows[0];
        let Some(Pane::App(app_pane)) = win.panes.get_mut(&pane) else {
            panic!("expected App pane");
        };
        let AppRuntime::Process(proc) = &mut app_pane.runtime else {
            panic!("expected Process runtime");
        };
        let has_deferred = !proc.deferred_ai_queries.is_empty();
        let has_prompt = proc.pending_prompts.iter().any(|p| {
            matches!(p, crate::process_app::PendingPrompt::Capability { capability, .. }
                if capability == "ai.query")
        });
        (has_deferred, has_prompt)
    };
    assert!(
        has_deferred,
        "AiQuery must be deferred — got none. \
         This means it was silently dropped instead of being routed."
    );
    assert!(
        has_prompt,
        "ai.query consent prompt must be queued after deferred AiQuery"
    );
}

// -- State snapshot -------------------------------------------------------

#[test]
fn add_test_pane_appears_in_snapshot() {
    let mut h = HostHarness::new();
    assert!(h.state().open_panes.is_empty());

    let pane = h.add_test_pane();
    let snap = h.state();
    assert!(snap.open_panes.contains(&pane));
    assert_eq!(
        snap.pane_titles.get(&pane).map(|s| s.as_str()),
        Some("Test App")
    );
}

/// Regression guard for the per-frame `ViewportCommand::Title` repaint loop:
/// `egui::Context::send_viewport_cmd` unconditionally requests an immediate
/// repaint, so any host code that sends a viewport command every frame pins
/// the render loop at the display refresh rate (uncapped when occluded).
/// An idle, settled frame must not request an immediate repaint.
#[test]
fn idle_frame_requests_no_immediate_repaint() {
    let mut h = HostHarness::new();
    let pane = h.add_test_pane();

    // Settle: first frames legitimately repaint (initial title send, app
    // render kick-off). Commit the pane's first render so it goes idle —
    // a render left in flight polls at frame rate by design.
    h.run_frames(1);
    h.render_payload_take(pane);
    h.inject(
        pane,
        DrawCommand::Control(crate::app_protocol::ControlCommand::FrameDone { frame_id: 1 }),
    );
    h.run_frames(5);

    h.run_frames(1);
    assert!(
        !h.app.ctx.requested_repaint_last_pass(),
        "an idle settled frame must not request an immediate repaint; \
         some per-frame code path (e.g. an unconditional send_viewport_cmd) \
         is forcing a continuous render loop"
    );
}

/// Issue #2208: a PGAP app that never sends FrameDone must not pin the host
/// at full frame rate. Two guarantees:
///
/// 1. While a render is legitimately in flight, the poll is a *bounded
///    delay* — never an immediate repaint. The old 16ms poll saturated to
///    zero inside egui's `request_repaint_after` (it subtracts the predicted
///    frame time, ~16.7ms at 60Hz), which meant 60fps polling.
/// 2. After `RENDER_IN_FLIGHT_TIMEOUT` with no FrameDone, the host abandons
///    the render transaction, marks the app hung (existing lifecycle path),
///    and stops requesting repaints entirely.
#[test]
fn hung_render_stops_repaint_polling_after_timeout() {
    use crate::process_app::{LifecycleState, RENDER_IN_FLIGHT_TIMEOUT};

    let mut h = HostHarness::new();
    let pane = h.add_test_pane();

    // Frame 1 sends Render(1); the app never answers with FrameDone.
    h.run_frames(1);
    h.render_payload_take(pane)
        .expect("first visible frame must request an app Render");
    h.run_frames(3);

    // Pre-timeout: the in-flight poll must survive egui's predicted-frame-
    // time subtraction — a bounded delayed repaint, not an immediate one.
    assert!(
        !h.app.ctx.requested_repaint_last_pass(),
        "render-in-flight polling must be a bounded delay, not an immediate \
         repaint; a poll interval at or below one frame time saturates to \
         zero and pins the host at full frame rate"
    );
    assert!(
        h.process_app_mut(pane).render_in_flight_for_test(),
        "render must still be in flight before the timeout"
    );

    // Simulate the render having been stuck for longer than the timeout.
    h.process_app_mut(pane)
        .backdate_in_flight_render(RENDER_IN_FLIGHT_TIMEOUT + std::time::Duration::from_secs(1));
    h.run_frames(2);

    let app = h.process_app_mut(pane);
    assert!(
        !app.render_in_flight_for_test(),
        "render-in-flight state must be cleared once the timeout expires"
    );
    assert_eq!(
        app.lifecycle.state(),
        LifecycleState::Hung,
        "a render timeout must surface through the existing hung status path"
    );
    assert!(
        !h.app.ctx.requested_repaint_last_pass(),
        "after the render timeout the host must stop requesting repaints \
         for the hung app instead of polling forever"
    );
}

#[test]
fn visible_idle_process_app_does_not_emit_recurring_render_events() {
    let mut h = HostHarness::new();
    let pane = h.add_test_pane();

    h.run_frames(1);
    let first = h
        .render_payload_take(pane)
        .expect("first visible frame must request an app Render");
    assert!(
        first.contains("\"frame_id\":1"),
        "first Render should use frame_id=1, got {first}"
    );

    h.inject(
        pane,
        DrawCommand::Control(crate::app_protocol::ControlCommand::FrameDone { frame_id: 1 }),
    );
    h.run_frames(1);
    let second = h.render_payload_take(pane);
    assert_eq!(
        second, None,
        "an idle visible app with a committed frame must not receive another Render just because the host repainted"
    );

    h.run_frames(2);
    assert_eq!(
        h.render_payload_take(pane),
        None,
        "idle host repaint cycles must not restart visible app rendering"
    );
}

#[test]
fn in_flight_process_app_render_is_not_duplicated_while_host_polls() {
    let mut h = HostHarness::new();
    let pane = h.add_test_pane();

    h.run_frames(1);
    assert!(
        h.render_payload_take(pane).is_some(),
        "first visible frame must request an app Render"
    );

    h.run_frames(3);
    assert_eq!(
        h.render_payload_take(pane),
        None,
        "host wake polling while awaiting FrameDone must not send duplicate Render events"
    );
}

#[test]
fn background_frame_done_clears_in_flight_render() {
    let mut h = HostHarness::new();
    let pane = h.add_test_pane();

    h.run_frames(1);
    assert!(
        h.render_payload_take(pane).is_some(),
        "first visible frame must request an app Render"
    );

    h.inject(
        pane,
        DrawCommand::Control(crate::app_protocol::ControlCommand::FrameDone { frame_id: 1 }),
    );
    {
        let win = &mut h.app.windows[0];
        let Some(Pane::App(app_pane)) = win.panes.get_mut(&pane) else {
            panic!("expected App pane");
        };
        let AppRuntime::Process(proc) = &mut app_pane.runtime else {
            panic!("expected Process runtime");
        };
        proc.background_tick();
    }

    h.inject(
        pane,
        DrawCommand::Control(crate::app_protocol::ControlCommand::ScheduleRender { after_ms: 0 }),
    );
    h.run_frames(2);
    let scheduled = h
        .render_payload_take(pane)
        .expect("a background-drained FrameDone must allow the next Render");
    assert!(
        scheduled.contains("\"frame_id\":2"),
        "next Render should use frame_id=2 after background FrameDone, got {scheduled}"
    );
}

#[test]
fn schedule_render_requests_next_process_app_render() {
    let mut h = HostHarness::new();
    let pane = h.add_test_pane();

    h.run_frames(1);
    assert!(
        h.render_payload_take(pane).is_some(),
        "first visible frame must request an app Render"
    );
    h.inject(
        pane,
        DrawCommand::Control(crate::app_protocol::ControlCommand::FrameDone { frame_id: 1 }),
    );
    h.run_frames(1);
    assert_eq!(
        h.render_payload_take(pane),
        None,
        "stable idle frame should not render before ScheduleRender"
    );

    h.inject(
        pane,
        DrawCommand::Control(crate::app_protocol::ControlCommand::ScheduleRender { after_ms: 0 }),
    );
    h.run_frames(1);
    let scheduled = h
        .render_payload_take(pane)
        .expect("a due ScheduleRender drained this pass must send the Render in the same pass");
    assert!(
        scheduled.contains("\"frame_id\":2"),
        "scheduled Render should use the next frame_id, got {scheduled}"
    );
}

#[test]
fn pending_async_completion_keeps_host_wake_without_idle_render() {
    let mut h = HostHarness::new();
    let pane = h.add_test_pane();

    h.run_frames(1);
    assert!(
        h.render_payload_take(pane).is_some(),
        "first visible frame must request an app Render"
    );
    h.inject(
        pane,
        DrawCommand::Control(crate::app_protocol::ControlCommand::FrameDone { frame_id: 1 }),
    );
    h.run_frames(1);
    assert_eq!(
        h.render_payload_take(pane),
        None,
        "stable app must not render before async work starts"
    );

    h.inject(
        pane,
        DrawCommand::Host(AppRequest::HttpRequest {
            request_id: "http-1".to_string(),
            url: "https://example.com/".to_string(),
            method: "GET".to_string(),
            headers: std::collections::HashMap::new(),
            body: None,
        }),
    );
    h.run_frames(1);
    assert_eq!(
        h.render_payload_take(pane),
        None,
        "arming async work must wake the host without sending an app Render"
    );

    let mut scheduled_render = None;
    for _ in 0..20 {
        std::thread::sleep(std::time::Duration::from_millis(5));
        h.run_frames(1);
        if let Some(payload) = h.render_payload_take(pane) {
            scheduled_render = Some(payload);
            break;
        }
    }

    let scheduled = scheduled_render.expect("async completion must trigger the next app Render");
    assert!(
        scheduled.contains("\"frame_id\":2"),
        "async-triggered Render should use the next frame_id, got {scheduled}"
    );
}

#[test]
fn two_test_panes_have_distinct_ids() {
    let mut h = HostHarness::new();
    let p1 = h.add_test_pane();
    let p2 = h.add_test_pane();
    assert_ne!(p1, p2);
    let snap = h.state();
    assert_eq!(snap.open_panes.len(), 2);
}

fn temp_response(dir: &std::path::Path, name: &str) -> String {
    dir.join(format!("{name}-{}.json", uuid::Uuid::new_v4()))
        .to_string_lossy()
        .into_owned()
}

fn read_json_response(path: &str) -> serde_json::Value {
    let content = std::fs::read_to_string(path).expect("response file must exist");
    serde_json::from_str(&content).expect("response must be valid JSON")
}

// -- Pane file slots ------------------------------------------------------

#[test]
fn pane_slots_write_read_list_delete() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut h = HostHarness::new();
    h.app.set_active_context_root(tmp.path().to_path_buf());
    let pane = h.add_test_pane();

    let write_file = temp_response(tmp.path(), "slot-write");
    h.inject_ipc(AppRequest::SlotWrite {
        pane_id: pane,
        slot_name: "artifact".to_string(),
        content: b"hello".to_vec(),
        append: false,
        replace: false,
        response_file: write_file.clone(),
    });
    h.run_frames(1);
    let write = read_json_response(&write_file);
    assert_eq!(write["ok"].as_bool(), Some(true));

    let append_file = temp_response(tmp.path(), "slot-append");
    h.inject_ipc(AppRequest::SlotWrite {
        pane_id: pane,
        slot_name: "artifact".to_string(),
        content: b" world".to_vec(),
        append: true,
        replace: false,
        response_file: append_file.clone(),
    });
    h.run_frames(1);
    assert_eq!(read_json_response(&append_file)["ok"].as_bool(), Some(true));

    let read_file = temp_response(tmp.path(), "slot-read");
    h.inject_ipc(AppRequest::SlotRead {
        pane_id: pane,
        slot_name: "artifact".to_string(),
        response_file: read_file.clone(),
    });
    h.run_frames(1);
    assert_eq!(
        std::fs::read(&read_file).expect("read response"),
        b"hello world"
    );

    let list_file = temp_response(tmp.path(), "slot-list");
    h.inject_ipc(AppRequest::SlotList {
        pane_id: pane,
        response_file: list_file.clone(),
    });
    h.run_frames(1);
    let list = read_json_response(&list_file);
    let entries = list.as_array().expect("slot list must be an array");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["name"], "artifact");
    assert_eq!(entries[0]["size"], 11);

    let delete_file = temp_response(tmp.path(), "slot-delete");
    h.inject_ipc(AppRequest::SlotDelete {
        pane_id: pane,
        slot_name: "artifact".to_string(),
        response_file: delete_file.clone(),
    });
    h.run_frames(1);
    let deleted = read_json_response(&delete_file);
    assert_eq!(deleted["ok"].as_bool(), Some(true));
    assert_eq!(deleted["removed"].as_bool(), Some(true));
}

#[test]
fn pane_slot_read_preserves_error_like_json_content() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut h = HostHarness::new();
    h.app.set_active_context_root(tmp.path().to_path_buf());
    let pane = h.add_test_pane();
    let raw_json = br#"{"ok":false,"error":"stored artifact"}"#;

    let write_file = temp_response(tmp.path(), "slot-json-write");
    h.inject_ipc(AppRequest::SlotWrite {
        pane_id: pane,
        slot_name: "artifact".to_string(),
        content: raw_json.to_vec(),
        append: false,
        replace: false,
        response_file: write_file.clone(),
    });
    h.run_frames(1);
    assert_eq!(read_json_response(&write_file)["ok"].as_bool(), Some(true));

    let read_file = temp_response(tmp.path(), "slot-json-read");
    h.inject_ipc(AppRequest::SlotRead {
        pane_id: pane,
        slot_name: "artifact".to_string(),
        response_file: read_file.clone(),
    });
    h.run_frames(1);
    assert_eq!(std::fs::read(&read_file).expect("read response"), raw_json);
    assert!(
        !std::path::PathBuf::from(format!("{read_file}.err")).exists(),
        "successful raw reads must not write the error sidecar"
    );
}

#[test]
fn pane_slot_read_errors_use_sidecar_file() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut h = HostHarness::new();
    h.app.set_active_context_root(tmp.path().to_path_buf());
    let pane = h.add_test_pane();

    let read_file = temp_response(tmp.path(), "slot-missing-read");
    h.inject_ipc(AppRequest::SlotRead {
        pane_id: pane,
        slot_name: "missing".to_string(),
        response_file: read_file.clone(),
    });
    h.run_frames(1);

    assert!(
        !std::path::Path::new(&read_file).exists(),
        "read error must not occupy the raw response path"
    );
    let error_file = format!("{read_file}.err");
    let response = read_json_response(&error_file);
    assert_eq!(response["ok"].as_bool(), Some(false));
    assert!(response["error"]
        .as_str()
        .expect("error")
        .contains("slot 'missing' not found"));
}

#[test]
fn pane_slot_write_requires_replace_or_append_for_existing_slot() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut h = HostHarness::new();
    h.app.set_active_context_root(tmp.path().to_path_buf());
    let pane = h.add_test_pane();

    let first_file = temp_response(tmp.path(), "slot-first");
    h.inject_ipc(AppRequest::SlotWrite {
        pane_id: pane,
        slot_name: "artifact".to_string(),
        content: b"first".to_vec(),
        append: false,
        replace: false,
        response_file: first_file.clone(),
    });
    h.run_frames(1);
    assert_eq!(read_json_response(&first_file)["ok"].as_bool(), Some(true));

    let second_file = temp_response(tmp.path(), "slot-second");
    h.inject_ipc(AppRequest::SlotWrite {
        pane_id: pane,
        slot_name: "artifact".to_string(),
        content: b"second".to_vec(),
        append: false,
        replace: false,
        response_file: second_file.clone(),
    });
    h.run_frames(1);
    let second = read_json_response(&second_file);
    assert_eq!(second["ok"].as_bool(), Some(false));
    assert!(
        second["error"]
            .as_str()
            .expect("error")
            .contains("already exists"),
        "unexpected error: {second}"
    );

    let read_file = temp_response(tmp.path(), "slot-read-after-reject");
    h.inject_ipc(AppRequest::SlotRead {
        pane_id: pane,
        slot_name: "artifact".to_string(),
        response_file: read_file.clone(),
    });
    h.run_frames(1);
    assert_eq!(std::fs::read(&read_file).expect("read response"), b"first");
}

#[test]
fn pane_slot_write_rejects_content_over_10_mib() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut h = HostHarness::new();
    h.app.set_active_context_root(tmp.path().to_path_buf());
    let pane = h.add_test_pane();

    let response_file = temp_response(tmp.path(), "slot-too-large");
    let too_large = vec![b'x'; 10 * 1024 * 1024 + 1];
    h.inject_ipc(AppRequest::SlotWrite {
        pane_id: pane,
        slot_name: "big".to_string(),
        content: too_large,
        append: false,
        replace: false,
        response_file: response_file.clone(),
    });
    h.run_frames(1);
    let response = read_json_response(&response_file);
    assert_eq!(response["ok"].as_bool(), Some(false));
    let error = response["error"].as_str().expect("error");
    assert!(error.contains("slot 'big'"), "unexpected error: {error}");
    assert!(error.contains("10485761"), "unexpected error: {error}");
}

#[test]
fn pane_slot_append_rejects_final_file_over_10_mib() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut h = HostHarness::new();
    h.app.set_active_context_root(tmp.path().to_path_buf());
    let pane = h.add_test_pane();

    let first_file = temp_response(tmp.path(), "slot-first");
    h.inject_ipc(AppRequest::SlotWrite {
        pane_id: pane,
        slot_name: "artifact".to_string(),
        content: vec![b'a'; 10 * 1024 * 1024],
        append: false,
        replace: false,
        response_file: first_file.clone(),
    });
    h.run_frames(1);
    assert_eq!(read_json_response(&first_file)["ok"].as_bool(), Some(true));

    let append_file = temp_response(tmp.path(), "slot-too-large-append");
    h.inject_ipc(AppRequest::SlotWrite {
        pane_id: pane,
        slot_name: "artifact".to_string(),
        content: b"!".to_vec(),
        append: true,
        replace: false,
        response_file: append_file.clone(),
    });
    h.run_frames(1);
    let response = read_json_response(&append_file);
    assert_eq!(response["ok"].as_bool(), Some(false));
    let error = response["error"].as_str().expect("error");
    assert!(
        error.contains("slot 'artifact'"),
        "unexpected error: {error}"
    );
    assert!(error.contains("10485761"), "unexpected error: {error}");
}

#[test]
fn pane_slot_append_uses_tracked_path_after_context_root_changes() {
    let first_root = tempfile::tempdir().expect("first root");
    let second_root = tempfile::tempdir().expect("second root");
    let mut h = HostHarness::new();
    h.app
        .set_active_context_root(first_root.path().to_path_buf());
    let pane = h.add_test_pane();

    let write_file = temp_response(first_root.path(), "slot-root-write");
    h.inject_ipc(AppRequest::SlotWrite {
        pane_id: pane,
        slot_name: "artifact".to_string(),
        content: b"hello".to_vec(),
        append: false,
        replace: false,
        response_file: write_file.clone(),
    });
    h.run_frames(1);
    let original_path = read_json_response(&write_file)["path"]
        .as_str()
        .expect("slot path")
        .to_string();

    h.app
        .set_active_context_root(second_root.path().to_path_buf());
    let append_file = temp_response(second_root.path(), "slot-root-append");
    h.inject_ipc(AppRequest::SlotWrite {
        pane_id: pane,
        slot_name: "artifact".to_string(),
        content: b" world".to_vec(),
        append: true,
        replace: false,
        response_file: append_file.clone(),
    });
    h.run_frames(1);
    let append = read_json_response(&append_file);
    assert_eq!(append["ok"].as_bool(), Some(true));
    assert_eq!(append["path"].as_str(), Some(original_path.as_str()));
    assert_eq!(
        std::fs::read(&original_path).expect("original slot contents"),
        b"hello world"
    );
}

#[test]
fn pane_info_and_list_include_slots_object() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut h = HostHarness::new();
    h.app.set_active_context_root(tmp.path().to_path_buf());
    let pane = h.add_test_pane();

    let write_file = temp_response(tmp.path(), "slot-info-write");
    h.inject_ipc(AppRequest::SlotWrite {
        pane_id: pane,
        slot_name: "artifact".to_string(),
        content: b"{} ".to_vec(),
        append: false,
        replace: false,
        response_file: write_file.clone(),
    });
    h.run_frames(1);
    let slot_path = read_json_response(&write_file)["path"]
        .as_str()
        .expect("slot path")
        .to_string();

    let info_file = temp_response(tmp.path(), "slot-info");
    h.inject_ipc(AppRequest::GetPaneInfo {
        pane_id: pane,
        response_file: info_file.clone(),
    });
    h.run_frames(1);
    let info = read_json_response(&info_file);
    assert_eq!(info["slots"]["artifact"].as_str(), Some(slot_path.as_str()));

    let list_file = temp_response(tmp.path(), "slot-pane-list");
    h.inject_ipc(AppRequest::ListPanes {
        response_file: list_file.clone(),
        context_id: None,
    });
    h.run_frames(1);
    let list = read_json_response(&list_file);
    let pane_entry = list
        .as_array()
        .expect("pane list array")
        .iter()
        .find(|entry| entry["id"].as_u64() == Some(pane))
        .expect("pane entry");
    assert_eq!(
        pane_entry["slots"]["artifact"].as_str(),
        Some(slot_path.as_str())
    );
}

#[test]
fn workspace_clean_slots_lists_and_removes_dead_pane_slot_dirs() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut h = HostHarness::new();
    h.app.set_active_context_root(tmp.path().to_path_buf());
    let pane = h.add_test_pane();

    let write_file = temp_response(tmp.path(), "slot-clean-write");
    h.inject_ipc(AppRequest::SlotWrite {
        pane_id: pane,
        slot_name: "artifact".to_string(),
        content: b"done".to_vec(),
        append: false,
        replace: false,
        response_file: write_file.clone(),
    });
    h.run_frames(1);
    assert_eq!(read_json_response(&write_file)["ok"].as_bool(), Some(true));

    let slot_dir = tmp
        .path()
        .join(crate::config::workspace_channel_dir())
        .join("slots")
        .join(pane.to_string());
    assert!(slot_dir.exists(), "slot dir should exist before clean");

    h.app.windows[0].panes.remove(&pane);

    let dry_run_file = temp_response(tmp.path(), "slot-clean-dry-run");
    h.inject_ipc(AppRequest::WorkspaceCleanSlots {
        dry_run: true,
        response_file: dry_run_file.clone(),
    });
    h.run_frames(1);
    let dry_run = read_json_response(&dry_run_file);
    assert_eq!(dry_run["ok"].as_bool(), Some(true));
    let dry_run_paths = dry_run["paths"].as_array().expect("paths array");
    assert!(dry_run_paths.iter().any(|path| {
        path.as_str()
            .is_some_and(|path| path == slot_dir.to_string_lossy())
    }));
    assert!(slot_dir.exists(), "dry-run must not remove slot dir");

    let clean_file = temp_response(tmp.path(), "slot-clean");
    h.inject_ipc(AppRequest::WorkspaceCleanSlots {
        dry_run: false,
        response_file: clean_file.clone(),
    });
    h.run_frames(1);
    assert_eq!(read_json_response(&clean_file)["ok"].as_bool(), Some(true));
    assert!(!slot_dir.exists(), "clean should remove dead pane slot dir");
}

#[test]
fn workspace_clean_slots_removes_unregistered_files_for_reused_pane_ids() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut h = HostHarness::new();
    h.app.set_active_context_root(tmp.path().to_path_buf());
    let pane = h.add_test_pane();

    let slot_dir = tmp
        .path()
        .join(crate::config::workspace_channel_dir())
        .join("slots")
        .join(pane.to_string());
    std::fs::create_dir_all(&slot_dir).expect("create stale slot dir");
    let stale_slot = slot_dir.join("artifact");
    std::fs::write(&stale_slot, b"stale").expect("write stale slot");

    let clean_file = temp_response(tmp.path(), "slot-clean-reused-pane");
    h.inject_ipc(AppRequest::WorkspaceCleanSlots {
        dry_run: false,
        response_file: clean_file.clone(),
    });
    h.run_frames(1);
    assert_eq!(read_json_response(&clean_file)["ok"].as_bool(), Some(true));
    assert!(
        !stale_slot.exists(),
        "unregistered stale slot file should be removed for live pane id"
    );

    let write_file = temp_response(tmp.path(), "slot-reused-pane-write");
    h.inject_ipc(AppRequest::SlotWrite {
        pane_id: pane,
        slot_name: "artifact".to_string(),
        content: b"fresh".to_vec(),
        append: false,
        replace: false,
        response_file: write_file.clone(),
    });
    h.run_frames(1);
    assert_eq!(read_json_response(&write_file)["ok"].as_bool(), Some(true));
}

// -- Nav stack ------------------------------------------------------------

/// Regression guard for PR #392: `push_nav` and `pop_nav` commands must be
/// processed so the host nav stack tracks depth correctly.
#[test]
fn push_nav_increments_nav_stack_depth() {
    let mut h = HostHarness::new();
    let pane = h.add_test_pane();

    h.inject(
        pane,
        DrawCommand::Host(AppRequest::PushNav {
            view_id: "detail".to_string(),
            title: "Detail".to_string(),
        }),
    );
    h.run_frames(2);

    let win = &h.app.windows[0];
    let Pane::App(app_pane) = win.panes.get(&pane).unwrap() else {
        panic!("expected App pane");
    };
    let AppRuntime::Process(proc) = &app_pane.runtime else {
        panic!("expected Process runtime");
    };
    assert_eq!(
        proc.nav_stack_depth(),
        1,
        "push_nav should add one entry to the nav stack"
    );
}

#[test]
fn push_pop_nav_returns_to_zero() {
    let mut h = HostHarness::new();
    let pane = h.add_test_pane();

    h.inject(
        pane,
        DrawCommand::Host(AppRequest::PushNav {
            view_id: "detail".to_string(),
            title: "Detail".to_string(),
        }),
    );
    h.run_frames(1);

    h.inject(pane, DrawCommand::Host(AppRequest::PopNav {}));
    h.run_frames(1);

    let win = &h.app.windows[0];
    let Pane::App(app_pane) = win.panes.get(&pane).unwrap() else {
        panic!("expected App pane");
    };
    let AppRuntime::Process(proc) = &app_pane.runtime else {
        panic!("expected Process runtime");
    };
    assert_eq!(
        proc.nav_stack_depth(),
        0,
        "pop_nav should empty the nav stack"
    );
}

// -- Status summary ───────────────────────────────────────────────────────

/// Verifies `DrawCommand::StatusSummary` is routed and stored on the pane,
/// not discarded or dumped into the visual frame buffer.
#[test]
fn status_summary_stored_on_process_app() {
    let mut h = HostHarness::new();
    let pane = h.add_test_pane();

    h.inject(
        pane,
        DrawCommand::Host(AppRequest::StatusSummary {
            text: "Working…".to_string(),
        }),
    );
    h.run_frames(1);

    let win = &h.app.windows[0];
    let Pane::App(app_pane) = win.panes.get(&pane).unwrap() else {
        panic!("expected App pane");
    };
    let AppRuntime::Process(proc) = &app_pane.runtime else {
        panic!("expected Process runtime");
    };
    assert_eq!(
        proc.status_summary.as_deref(),
        Some("Working…"),
        "StatusSummary command must be routed to process_app.status_summary"
    );
}

#[test]
fn set_pane_title_unknown_pane_id_does_not_panic() {
    // Injects SetPaneTitle for a pane_id that doesn't exist.
    // Must run without panicking and log a warn — verifies the drain path is wired.
    let mut h = HostHarness::new();
    h.ipc_tx
        .send(AppRequest::SetPaneTitle {
            pane_id: 9999,
            name: "ghost".into(),
        })
        .unwrap();
    h.run_frames(1); // must not panic; logs warn "not found"
}

#[test]
fn osc_title_updates_unlocked_terminal_pane_name_by_default() {
    let mut h = HostHarness::new();
    let app_pane = h.add_test_pane();
    h.focus_pane(app_pane);
    h.run_frames(1);

    h.app.split_focused(true, None, false, false, None);
    let terminal_pane_id = h.app.windows[0]
        .panes
        .iter()
        .find_map(|(id, pane)| pane.as_terminal().map(|_| *id))
        .expect("split should create a terminal pane");

    h.app
        .pty_event_tx
        .send((
            terminal_pane_id,
            egui_term::PtyEvent::Title("/tmp/claude-work".to_string()),
        ))
        .unwrap();
    h.run_frames(1);

    let terminal = h.app.windows[0]
        .panes
        .get(&terminal_pane_id)
        .and_then(|pane| pane.as_terminal())
        .expect("terminal pane should still exist");
    assert_eq!(terminal.pty_title.as_deref(), Some("/tmp/claude-work"));
    assert_eq!(terminal.name.as_deref(), Some("/tmp/claude-work"));
}

#[test]
fn osc_title_tracks_but_does_not_overwrite_locked_terminal_pane_name() {
    let mut h = HostHarness::new();
    let app_pane = h.add_test_pane();
    h.focus_pane(app_pane);
    h.run_frames(1);

    h.app.split_focused(true, None, false, false, None);
    let terminal_pane_id = h.app.windows[0]
        .panes
        .iter()
        .find_map(|(id, pane)| pane.as_terminal().map(|_| *id))
        .expect("split should create a terminal pane");

    h.inject_ipc(AppRequest::SetPaneTitle {
        pane_id: terminal_pane_id,
        name: "Pinned".to_string(),
    });
    h.run_frames(1);

    h.app
        .pty_event_tx
        .send((
            terminal_pane_id,
            egui_term::PtyEvent::Title("/tmp/claude-work".to_string()),
        ))
        .unwrap();
    h.run_frames(1);

    let terminal = h.app.windows[0]
        .panes
        .get(&terminal_pane_id)
        .and_then(|pane| pane.as_terminal())
        .expect("terminal pane should still exist");
    assert_eq!(terminal.pty_title.as_deref(), Some("/tmp/claude-work"));
    assert_eq!(terminal.name.as_deref(), Some("Pinned"));
}

/// Regression guard for issue #1018: dismissing the command palette must
/// surrender egui focus from palette_search so AccessKit holds no stale
/// focused node ID after the widget is gone. Without the fix, the next
/// pane close triggers AccessKit's internal consistency check and panics.
#[test]
fn palette_close_surrenders_focus_before_pane_close() {
    let mut h = HostHarness::new();
    h.add_test_pane();

    // Open the palette — sync_command_palette_focus pushes the focus layer
    // and the per-frame code requests focus on palette_search.
    h.app.show_command_palette = true;
    h.run_frames(3);

    // Dismiss the palette — sync_command_palette_focus must pop the layer
    // AND surrender palette_search focus so AccessKit has no stale node.
    h.app.show_command_palette = false;
    h.run_frames(2);

    // Close a pane — triggers the AccessKit consistency check that panicked
    // when palette_search focus was not surrendered. Passes if no panic.
    h.app.execute_close_pane();
    h.run_frames(1);
}

// -- Shell execution security (issue #1177) ───────────────────────────────

/// Regression guard: an app without `terminal.bindings` must never reach
/// the `sh -c` spawn in `StreamProcess`. The host must return
/// `StreamEnd { exit_code: 1 }` immediately and never spawn a subprocess.
///
/// This is the only app-reachable `sh -c` path in the codebase. Any future
/// app-reachable shell execution path must add a matching denial test here.
/// See `docs/security/shell-execution-inventory.md` for the full audit.
#[test]
fn stream_process_denied_without_terminal_bindings() {
    use crate::app_protocol::{AppRequest, DrawCommand, PlexiEvent, StreamChannel};

    let mut h = HostHarness::new();
    let pane = h.add_test_pane_with_permissions(AppPermissions::from_capability_strings(&[]));

    h.inject(
        pane,
        DrawCommand::Host(AppRequest::StreamProcess {
            correlation_id: "sec-test-1".to_string(),
            terminal_pane_id: 0,
            command: "echo SHOULD_NOT_RUN".to_string(),
            channel: StreamChannel::Stdout,
        }),
    );

    {
        let win = &mut h.app.windows[0];
        let Some(Pane::App(app_pane)) = win.panes.get_mut(&pane) else {
            panic!("expected App pane");
        };
        let AppRuntime::Process(proc) = &mut app_pane.runtime else {
            panic!("expected Process runtime");
        };
        proc.background_tick();
    }

    let effects = h.effects_drain(pane);
    assert!(
        !effects.is_empty(),
        "StreamProcess must produce an outbound event — got none. \
         This means the denial path was not reached."
    );
    let stream_end = effects.iter().find(|e| {
        matches!(e, PlexiEvent::StreamEnd { correlation_id, exit_code: 1 }
            if correlation_id == "sec-test-1")
    });
    assert!(
        stream_end.is_some(),
        "expected StreamEnd {{ exit_code: 1 }} for denied StreamProcess, got: {:?}",
        effects
    );
}

// -- Handle-key chain (issue #1764) ──────────────────────────────────────

/// Notification modal handle_key always returns Consumed so poll_actions and
/// dispatch_app_key_events are skipped while the modal is open.
#[test]
fn notification_modal_handle_key_returns_consumed() {
    use crate::app::app_trait::KeyDisposition;
    use crate::app::FocusLayer;
    let mut h = HostHarness::new();
    h.app.push_focus_layer(FocusLayer::NotificationModal);
    let ctx = h.app.ctx.clone();
    let disposition = h.app.notification_modal_handle_key(&ctx);
    assert_eq!(
        disposition,
        KeyDisposition::Consumed,
        "notification_modal_handle_key must return Consumed"
    );
}

// -- Context root CWD resolution ------------------------------------------

/// Regression guard for #1534: `reset_active_context` (Cmd+N from welcome
/// screen) must use `cwd_for_welcome_tab()` — which checks the context root
/// — rather than hardcoding `home_dir()`.
#[test]
fn cwd_for_welcome_tab_returns_context_root_when_set() {
    let root = std::path::PathBuf::from("/tmp");
    let mut h = HostHarness::new();
    h.app.set_active_context_root(root.clone());
    assert_eq!(
        h.app.cwd_for_welcome_tab(),
        root,
        "cwd_for_welcome_tab must return the context root when one is set"
    );
}

/// Regression guard for #1534: without a context root, `cwd_for_welcome_tab`
/// must fall back to home dir, not panic or return an arbitrary dir.
#[test]
fn cwd_for_welcome_tab_falls_back_to_window_path_when_no_root() {
    let h = HostHarness::new();
    // No root set — fallback chain: None → home_dir()
    assert_eq!(
        h.app.cwd_for_welcome_tab(),
        dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("/")),
        "cwd_for_welcome_tab must fall back to home dir when no context root is set"
    );
}

// -- Capability modal focus layer -----------------------------------------

/// Regression guard for #1596: Escape must reach the capability consent
/// modal and fire deny_once. Before the fix, `dispatch_app_key_events` ran
/// in step 3 of `update()`, the focused app consumed any key (returning
/// `consumed = true`), which caused the host to steal Escape from the
/// input state — the modal in step 4 saw nothing. After the fix the modal
/// renders in step 2 with exclusive keyboard ownership.
///
/// Observable invariants checked (all follow from a successful deny_once):
/// 1. `CapabilityModal` is on the focus stack while a prompt is queued.
/// 2. After Escape, `pending_prompts` is empty (prompt was popped).
/// 3. After Escape, `CapabilityModal` is removed from the focus stack
///    (sync_capability_modal_focus pops it when the queue drains).
#[test]
fn capability_modal_escape_fires_deny_once() {
    use crate::app::FocusLayer;
    use crate::host::pane::AppRuntime;
    use crate::process_app::PendingPrompt;

    let mut h = HostHarness::new();
    let pane = h.add_test_pane();

    // Set the pane as the focused pane so sync_capability_modal_focus finds it.
    let tile_id = {
        let win = &h.app.windows[0];
        win.tree
            .tiles
            .iter()
            .find_map(|(id, tile)| match tile {
                egui_tiles::Tile::Pane(p) if *p == pane => Some(id),
                _ => None,
            })
            .expect("test pane must have a tile")
    };
    h.app.windows[0].focused_pane = Some(*tile_id);

    // Queue a capability prompt directly on the pane's ProcessApp.
    {
        let win = &mut h.app.windows[0];
        let Some(Pane::App(app_pane)) = win.panes.get_mut(&pane) else {
            panic!("expected App pane");
        };
        let AppRuntime::Process(ref mut proc) = app_pane.runtime else {
            panic!("expected Process runtime");
        };
        proc.pending_prompts.push_back(PendingPrompt::Capability {
            request_id: "req-cap-1".to_string(),
            capability: "net.http".to_string(),
        });
    }

    // One idle frame: sync_capability_modal_focus should push the layer.
    h.run_frames(1);
    assert!(
        h.app.focus_stack.contains(&FocusLayer::CapabilityModal),
        "CapabilityModal must be on the focus stack when the focused pane has pending prompts"
    );

    // Send Escape. The modal must consume it and pop the prompt (deny_once).
    h.key(egui::Key::Escape, egui::Modifiers::NONE);

    // pending_prompts must be empty — Escape triggered deny_once, which pops
    // the front of the queue. If it's still non-empty, the modal didn't see Escape.
    let prompts_empty = {
        let win = &h.app.windows[0];
        let Some(Pane::App(app_pane)) = win.panes.get(&pane) else {
            panic!("expected App pane");
        };
        let AppRuntime::Process(ref proc) = app_pane.runtime else {
            panic!("expected Process runtime");
        };
        proc.pending_prompts.is_empty()
    };
    assert!(
        prompts_empty,
        "pending_prompts must be empty after Escape — deny_once must have consumed the prompt. \
         If non-empty, dispatch_app_key_events stole Escape before the modal could read it."
    );

    // CapabilityModal must have been popped from the focus stack since
    // sync_capability_modal_focus removes it when pending_prompts drains.
    assert!(
        !h.app.focus_stack.contains(&FocusLayer::CapabilityModal),
        "CapabilityModal must be removed from the focus stack after deny_once drains the queue"
    );
}

/// A buried sync-managed layer must be removed even when another layer sits on top.
///
/// Regression guard for the `pop_focus_layer` → `retain` fix: `pop_focus_layer` only
/// removes the top entry, so a buried entry would survive until it accidentally
/// became top again — causing a closed overlay to regain keyboard ownership.
#[test]
fn buried_stale_focus_layer_is_removed_by_sync() {
    use crate::app::FocusLayer;

    let mut h = HostHarness::new();

    // Push ConfirmClose by activating its source state and running sync.
    h.app.pending_close = true;
    h.app.sync_confirm_close_focus();
    assert!(
        h.app.focus_stack.contains(&FocusLayer::ConfirmClose),
        "ConfirmClose must be pushed when pending_close is true"
    );

    // Push CommandPalette on top — now ConfirmClose is buried.
    h.app.show_command_palette = true;
    h.app.sync_command_palette_focus();
    assert_eq!(
        h.app.focus_stack.last(),
        Some(&FocusLayer::CommandPalette),
        "CommandPalette must be at the top after its source state becomes true"
    );
    assert!(
        h.app.focus_stack.contains(&FocusLayer::ConfirmClose),
        "ConfirmClose must still be in the stack (buried beneath CommandPalette)"
    );

    // Clear ConfirmClose source state — sync must remove the buried entry.
    h.app.pending_close = false;
    h.app.sync_confirm_close_focus();

    assert!(
        !h.app.focus_stack.contains(&FocusLayer::ConfirmClose),
        "ConfirmClose must be removed from the stack even though CommandPalette was on top. \
         If this fails, sync_confirm_close_focus used pop_focus_layer (top-only) instead of retain."
    );

    // The layer that was on top must still be present — we only removed ConfirmClose.
    assert!(
        h.app.focus_stack.contains(&FocusLayer::CommandPalette),
        "CommandPalette must remain in the stack after removing the buried ConfirmClose layer"
    );
}

// -- QuickNote paste (#1637) -----------------------------------------------

/// Regression guard for #1637: Cmd+V (paste) while QuickNote is open must
/// land in `quick_note_text`, not fall through to the terminal pane behind
/// the overlay.
///
/// The fix: `draw_quick_note_modal` explicitly consumes `egui::Event::Paste`
/// via `ctx.input_mut` and inserts the text at the cursor position, rather
/// than relying on the TextEdit's internal event processing (which depends on
/// focus being fully settled by the time the TextEdit renders).
#[test]
fn quick_note_paste_inserts_into_note_text() {
    let mut h = HostHarness::new();

    // Open QuickNote.
    h.app.push_focus_layer(crate::app::FocusLayer::QuickNote);

    // Run two idle frames so the overlay and focus fully settle.
    h.run_frames(2);

    // Inject a paste event in the next frame.
    h.frame(egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1280.0, 800.0),
        )),
        events: vec![egui::Event::Paste("hello world".into())],
        ..Default::default()
    });

    assert_eq!(
        h.app.quick_note_text, "hello world",
        "Pasted text must appear in quick_note_text after a Paste event with QuickNote open"
    );
}

/// After a paste event with QuickNote open, `quick_note_text` must be
/// updated AND the `Paste` event must not be present in the queue for
/// downstream readers (terminal backends, poll_actions).
#[test]
fn quick_note_paste_consumed_from_queue() {
    let mut h = HostHarness::new();

    h.app.push_focus_layer(crate::app::FocusLayer::QuickNote);
    h.run_frames(2);

    // Inject paste + a non-input event that must survive.
    h.app.ctx.input_mut(|i| {
        i.events.push(egui::Event::Paste("test".into()));
        i.events
            .push(egui::Event::PointerMoved(egui::pos2(50.0, 50.0)));
    });

    // Run the overlay draw directly — draw_quick_note_modal consumes key events
    // via ctx.input_mut before the terminal backends run.
    let ctx = h.app.ctx.clone();
    h.app.draw_quick_note_modal(&ctx);

    // Paste event must be gone.
    h.app.ctx.input(|i| {
        assert!(
            !i.events.iter().any(|e| matches!(e, egui::Event::Paste(_))),
            "Paste event must be consumed before terminal backends see it"
        );
        // Non-input event must survive.
        assert!(
            i.events
                .iter()
                .any(|e| matches!(e, egui::Event::PointerMoved(_))),
            "Non-input events must not be drained"
        );
    });
}

// -- Layer 2 post-CentralPanel focus guard (#1601) -------------------------

/// Helper: simulate a CentralPanel pane widget stealing egui focus between two frames.
/// Returns the egui Id of the "stealing" widget so callers can assert against it.
fn steal_focus(h: &HostHarness) -> egui::Id {
    let steal_id = egui::Id::new("fake_pane_text_input_steal");
    h.ctx.memory_mut(|m| m.request_focus(steal_id));
    steal_id
}

/// Regression guard for #2124: migrated host TextField overlays must register
/// their focus intent with the UI-kit registry, and the post-CentralPanel drain
/// must re-claim focus after a pane TextInput steals it.
#[test]
fn command_palette_text_field_registry_wins_after_central_panel_steal() {
    let mut h = HostHarness::new();
    h.app.show_command_palette = true;
    h.app.sync_command_palette_focus();

    h.run_frames(1);
    steal_focus(&h);
    h.run_frames(1);

    assert_eq!(
        h.ctx.memory(|m| m.focused()),
        Some(egui::Id::new("palette_search")),
        "palette_search must win focus back through the host TextField registry"
    );
}

/// Regression guard for #1601: rename-pane TextEdit must retain egui focus
/// after CentralPanel renders. The between-frame steal simulates a pane TextInput
/// calling request_focus during CentralPanel — the post-CentralPanel block must
/// re-claim focus for `rename_pane_input` before the frame ends.
#[test]
fn rename_pane_overlay_focus_wins_after_central_panel_steal() {
    use crate::app::FocusLayer;
    let mut h = HostHarness::new();
    let pane = h.add_test_pane();
    h.app.renaming_pane = Some(pane);
    h.app.rename_buffer = "test name".to_string();
    h.app.push_focus_layer(FocusLayer::RenamePane);
    // Frame 1: one-shot fires, focus = rename_pane_input.
    h.run_frames(1);
    // Simulate CentralPanel pane widget stealing focus.
    steal_focus(&h);
    // Frame 2: post-CentralPanel block must reclaim rename_pane_input.
    h.run_frames(1);
    assert_eq!(
        h.ctx.memory(|m| m.focused()),
        Some(egui::Id::new("rename_pane_input")),
        "rename_pane_input must win focus back after a CentralPanel pane widget steals it"
    );
}

/// Regression guard for #1601: context-rename TextEdit (modal path, sidebar hidden)
/// must retain egui focus after CentralPanel renders.
#[test]
fn context_rename_overlay_focus_wins_after_central_panel_steal() {
    use crate::app::FocusLayer;
    let mut h = HostHarness::new();
    h.app.renaming_window = Some(0);
    h.app.rename_buffer = "new context".to_string();
    h.app.sidebar_visible = false;
    h.app.push_focus_layer(FocusLayer::ContextRename);
    h.run_frames(1);
    steal_focus(&h);
    h.run_frames(1);
    assert_eq!(
        h.ctx.memory(|m| m.focused()),
        Some(egui::Id::new("rename_context_input")),
        "rename_context_input must win focus back after a CentralPanel pane widget steals it"
    );
}

/// Regression guard for #1601: edit-description TextEdit must retain egui focus
/// after CentralPanel renders.
#[test]
fn edit_description_overlay_focus_wins_after_central_panel_steal() {
    use crate::app::FocusLayer;
    let mut h = HostHarness::new();
    h.app.editing_description = Some(0);
    h.app.description_buffer = "my description".to_string();
    h.app.push_focus_layer(FocusLayer::ContextDescription);
    h.run_frames(1);
    steal_focus(&h);
    h.run_frames(1);
    assert_eq!(
        h.ctx.memory(|m| m.focused()),
        Some(egui::Id::new("edit_description_input")),
        "edit_description_input must win focus back after a CentralPanel pane widget steals it"
    );
}

/// Regression guard for #1601: generic text-input overlay field must retain
/// egui focus after CentralPanel renders.
#[test]
fn text_input_overlay_focus_wins_after_central_panel_steal() {
    use crate::app::{FocusLayer, OverlayTarget, TextInputOverlay};
    let mut h = HostHarness::new();
    h.app.text_overlay = Some((
        TextInputOverlay {
            label: "Root directory".to_string(),
            hint: "/path/to/project".to_string(),
            buffer: String::new(),
            focus_requested: false,
        },
        OverlayTarget::ContextRoot(0),
    ));
    h.app.push_focus_layer(FocusLayer::TextInput);
    h.run_frames(1);
    steal_focus(&h);
    h.run_frames(1);
    assert_eq!(
        h.ctx.memory(|m| m.focused()),
        Some(egui::Id::new("text_input_overlay_field")),
        "text_input_overlay_field must win focus back after a CentralPanel pane widget steals it"
    );
}

/// Regression guard for #1601: capability/secret prompt TextEdit must retain
/// egui focus after CentralPanel renders. The secret TextEdit now carries a
/// stable `capability_secret_input` Id so the post-CentralPanel block can
/// target it by name.
#[test]
fn capability_secret_overlay_focus_wins_after_central_panel_steal() {
    use crate::app::FocusLayer;
    use crate::host::pane::AppRuntime;
    use crate::process_app::PendingPrompt;

    let mut h = HostHarness::new();
    let pane = h.add_test_pane();

    // Focus the pane so sync_capability_modal_focus can find it.
    let tile_id = {
        let win = &h.app.windows[0];
        win.tree
            .tiles
            .iter()
            .find_map(|(id, tile)| match tile {
                egui_tiles::Tile::Pane(p) if *p == pane => Some(id),
                _ => None,
            })
            .expect("test pane must have a tile")
    };
    h.app.windows[0].focused_pane = Some(*tile_id);

    // Queue a Secret prompt on the pane.
    {
        let win = &mut h.app.windows[0];
        let Some(Pane::App(app_pane)) = win.panes.get_mut(&pane) else {
            panic!("expected App pane");
        };
        let AppRuntime::Process(ref mut proc) = app_pane.runtime else {
            panic!("expected Process runtime");
        };
        proc.pending_prompts.push_back(PendingPrompt::Secret {
            key: "API_KEY".to_string(),
        });
    }

    // Frame 1: sync pushes CapabilityModal layer; modal renders; post-CentralPanel
    // requests focus on capability_secret_input.
    h.run_frames(1);
    assert!(
        h.app.focus_stack.contains(&FocusLayer::CapabilityModal),
        "CapabilityModal must be on the focus stack when the focused pane has a pending Secret prompt"
    );

    // Simulate CentralPanel pane widget stealing focus.
    steal_focus(&h);

    // Frame 2: post-CentralPanel block must reclaim capability_secret_input.
    h.run_frames(1);
    assert_eq!(
        h.ctx.memory(|m| m.focused()),
        Some(egui::Id::new("capability_secret_input")),
        "capability_secret_input must win focus back after a CentralPanel pane widget steals it"
    );
}

// -- Pane read/control over PGAP (stint 0013/0014) --------------------------

/// App WITHOUT `panes.read` (yellow/unset, NOT blocked) sends ListPanes via
/// PGAP: with yellow-state routing (stint 0017) the request is deferred and a
/// consent prompt is queued — no denial JSON is written and no forward happens
/// until the user decides.
#[test]
fn pgap_list_panes_yellow_queues_prompt_without_panes_read() {
    use crate::host::pane::AppRuntime;
    use crate::process_app::PendingPrompt;

    let mut h = HostHarness::new();
    let pane = h.add_test_pane_with_permissions(AppPermissions::from_capability_strings(&[]));

    let tmp = tempfile::tempdir().expect("tempdir");
    let response_file = tmp.path().join("list-panes.json");
    h.inject(
        pane,
        DrawCommand::Host(AppRequest::ListPanes {
            response_file: response_file.to_string_lossy().into_owned(),
            context_id: None,
        }),
    );
    h.run_frames(2);

    assert!(
        !response_file.exists(),
        "yellow-state request must not write a denial or a forward result — it waits on consent"
    );
    let win = &h.app.windows[0];
    let Some(Pane::App(app_pane)) = win.panes.get(&pane) else {
        panic!("expected App pane");
    };
    let AppRuntime::Process(ref proc) = app_pane.runtime else {
        panic!("expected Process runtime");
    };
    assert!(
        proc.pending_prompts.iter().any(|p| {
            matches!(p, PendingPrompt::Capability { capability, .. } if capability == "panes.read")
        }),
        "a consent prompt for panes.read must be queued"
    );
    assert_eq!(
        proc.deferred_gated_requests.len(),
        1,
        "the request must be deferred pending consent"
    );
}

/// App WITH `panes.read` sends ListPanes via PGAP: the request is forwarded
/// into the host pane-IPC handler, which writes the pane list JSON array.
#[test]
fn pgap_list_panes_forwarded_with_panes_read() {
    let mut h = HostHarness::new();
    let pane = h.add_test_pane_with_permissions(AppPermissions::from_capability_strings(&[
        "panes.read".to_string(),
    ]));

    let tmp = tempfile::tempdir().expect("tempdir");
    let response_file = tmp.path().join("list-panes.json");
    h.inject(
        pane,
        DrawCommand::Host(AppRequest::ListPanes {
            response_file: response_file.to_string_lossy().into_owned(),
            context_id: None,
        }),
    );
    h.run_frames(2);

    let content =
        std::fs::read_to_string(&response_file).expect("grant must write the response file");
    let v: serde_json::Value = serde_json::from_str(&content).expect("response must be JSON");
    let panes = v
        .as_array()
        .expect("ListPanes response must be a JSON array");
    assert!(
        panes.iter().any(|p| p["id"].as_u64() == Some(pane)),
        "pane list must contain the requesting pane: {content}"
    );
}

/// App with `panes.control` BLOCKED (stored Red) sends SendToPane via PGAP:
/// the gate must write the capability-denied error object instead of
/// forwarding — and must not queue a consent prompt (only the yellow/unset
/// middle state prompts; Red stays flat-denied).
#[test]
fn pgap_send_to_pane_denied_without_panes_control() {
    let mut h = HostHarness::new();
    let mut perms = AppPermissions::from_capability_strings(&[
        "panes.read".to_string(), // read alone must NOT grant control
    ]);
    perms
        .blocked
        .insert(crate::app::permissions::Capability::PanesControl);
    let pane = h.add_test_pane_with_permissions(perms);

    let tmp = tempfile::tempdir().expect("tempdir");
    let response_file = tmp.path().join("send-to-pane.json");
    h.inject(
        pane,
        DrawCommand::Host(AppRequest::SendToPane {
            pane_id: pane,
            text: "echo hi".to_string(),
            response_file: Some(response_file.to_string_lossy().into_owned()),
        }),
    );
    h.run_frames(2);

    let content =
        std::fs::read_to_string(&response_file).expect("denial must write the response file");
    let v: serde_json::Value = serde_json::from_str(&content).expect("response must be JSON");
    assert_eq!(
        v["capability"], "panes.control",
        "denial object must name the missing capability: {content}"
    );
}

/// App WITH `panes.control` sends SendToPane via PGAP: the request reaches the
/// host pane-IPC handler. The target is the app's own (non-terminal) pane, so
/// the handler — not the capability gate — answers with its pane-type error,
/// proving the forward happened.
#[test]
fn pgap_send_to_pane_forwarded_with_panes_control() {
    let mut h = HostHarness::new();
    let pane = h.add_test_pane_with_permissions(AppPermissions::from_capability_strings(&[
        "panes.control".to_string(),
    ]));

    let tmp = tempfile::tempdir().expect("tempdir");
    let response_file = tmp.path().join("send-to-pane.json");
    h.inject(
        pane,
        DrawCommand::Host(AppRequest::SendToPane {
            pane_id: pane,
            text: "echo hi".to_string(),
            response_file: Some(response_file.to_string_lossy().into_owned()),
        }),
    );
    h.run_frames(2);

    let content =
        std::fs::read_to_string(&response_file).expect("forward must write the response file");
    let v: serde_json::Value = serde_json::from_str(&content).expect("response must be JSON");
    assert!(
        v.get("capability").is_none(),
        "granted request must not produce a capability denial: {content}"
    );
    assert!(
        v["error"]
            .as_str()
            .is_some_and(|e| e.contains("not a terminal pane")),
        "host handler must answer with its own pane-type error: {content}"
    );
}

// -- Permission management + yellow-state routing (stint 0017) ---------------

/// App WITHOUT `permissions.manage` (yellow/unset) sends ListPermissions via
/// PGAP: yellow-state routing queues a consent prompt and defers the request —
/// no denial JSON, no forward.
#[test]
fn list_permissions_denied_without_manage() {
    use crate::host::pane::AppRuntime;
    use crate::process_app::PendingPrompt;

    let mut h = HostHarness::new();
    let pane = h.add_test_pane_with_permissions(AppPermissions::from_capability_strings(&[]));

    let tmp = tempfile::tempdir().expect("tempdir");
    let response_file = tmp.path().join("list-permissions.json");
    h.inject(
        pane,
        DrawCommand::Host(AppRequest::ListPermissions {
            response_file: response_file.to_string_lossy().into_owned(),
        }),
    );
    h.run_frames(2);

    assert!(
        !response_file.exists(),
        "ungranted+unblocked permissions.manage must defer, not deny or forward"
    );
    let win = &h.app.windows[0];
    let Some(Pane::App(app_pane)) = win.panes.get(&pane) else {
        panic!("expected App pane");
    };
    let AppRuntime::Process(ref proc) = app_pane.runtime else {
        panic!("expected Process runtime");
    };
    assert!(
        proc.pending_prompts.iter().any(|p| {
            matches!(p, PendingPrompt::Capability { capability, .. }
                if capability == "permissions.manage")
        }),
        "a consent prompt for permissions.manage must be queued"
    );
    assert_eq!(proc.deferred_gated_requests.len(), 1);
}

/// App WITH `permissions.manage` sends ListPermissions via PGAP: the request
/// is forwarded into the host handler, which writes the permission inventory —
/// stored rows from permissions.toml plus live rows for running apps.
#[test]
fn list_permissions_granted() {
    use crate::app::permissions::{Capability, PermissionState, PermissionStore};

    let mut h = HostHarness::new();
    let pane = h.add_test_pane_with_permissions(AppPermissions::from_capability_strings(&[
        "permissions.manage".to_string(),
    ]));

    // Seed a stored decision for an (unrelated) app.
    let store_dir = h.app.permission_store_dir.clone();
    {
        let mut store = PermissionStore::load_or_default(&store_dir);
        store.set(
            "other-app",
            std::path::Path::new("/test/project"),
            Capability::NetHttp,
            PermissionState::Red,
        );
        store.save();
    }

    let tmp = tempfile::tempdir().expect("tempdir");
    let response_file = tmp.path().join("list-permissions.json");
    h.inject(
        pane,
        DrawCommand::Host(AppRequest::ListPermissions {
            response_file: response_file.to_string_lossy().into_owned(),
        }),
    );
    h.run_frames(2);

    let content =
        std::fs::read_to_string(&response_file).expect("grant must write the response file");
    let v: serde_json::Value = serde_json::from_str(&content).expect("response must be JSON");
    let rows = v["permissions"]
        .as_array()
        .expect("permissions must be an array");
    let stored_row = rows
        .iter()
        .find(|r| r["app_id"] == "other-app")
        .unwrap_or_else(|| panic!("stored entry must appear: {content}"));
    assert_eq!(stored_row["capability"], "net.http");
    assert_eq!(stored_row["state"], "red");
    assert_eq!(stored_row["stored"], true);
    assert_eq!(stored_row["sensitive"], true);

    // The requesting app itself is running with a live (unstored) grant.
    let live_row = rows
        .iter()
        .find(|r| r["app_id"] == "test" && r["capability"] == "permissions.manage")
        .unwrap_or_else(|| panic!("live capability of running app must appear: {content}"));
    assert_eq!(live_row["state"], "green");
    assert_eq!(live_row["stored"], false);

    let running = v["running"].as_array().expect("running must be an array");
    assert!(
        running.iter().any(|a| a == "test"),
        "running list must contain the requesting app: {content}"
    );
}

/// SetPermission to red must persist to the store AND live-update the running
/// app's AppPermissions so the revocation takes effect on its next request.
#[test]
fn set_permission_revokes_running_app() {
    use crate::app::permissions::{Capability, PermissionState, PermissionStore};
    use crate::host::pane::AppRuntime;

    let mut h = HostHarness::new();
    let manager = h.add_test_pane_with_permissions(AppPermissions::from_capability_strings(&[
        "permissions.manage".to_string(),
    ]));
    let target = h.add_test_pane_with_permissions(AppPermissions::from_capability_strings(&[
        "panes.read".to_string(),
    ]));

    let workspace = std::env::temp_dir();
    let tmp = tempfile::tempdir().expect("tempdir");
    let response_file = tmp.path().join("set-permission.json");
    h.inject(
        manager,
        DrawCommand::Host(AppRequest::SetPermission {
            app_id: "test".to_string(),
            workspace: Some(workspace.to_string_lossy().into_owned()),
            capability: "panes.read".to_string(),
            state: "red".to_string(),
            response_file: response_file.to_string_lossy().into_owned(),
        }),
    );
    h.run_frames(2);

    let content =
        std::fs::read_to_string(&response_file).expect("SetPermission must write a reply");
    let v: serde_json::Value = serde_json::from_str(&content).expect("reply must be JSON");
    assert_eq!(v["ok"], true, "reply must be ok: {content}");

    // Live AppPermissions of the running target must reflect the revocation.
    let win = &h.app.windows[0];
    let Some(Pane::App(app_pane)) = win.panes.get(&target) else {
        panic!("expected App pane");
    };
    let AppRuntime::Process(ref proc) = app_pane.runtime else {
        panic!("expected Process runtime");
    };
    assert!(
        proc.permissions.blocked.contains(&Capability::PanesRead),
        "live permissions must have panes.read blocked"
    );
    assert!(
        !proc
            .permissions
            .capabilities
            .contains(&Capability::PanesRead),
        "live permissions must no longer grant panes.read"
    );

    // The store must have persisted the Red decision.
    let store = PermissionStore::load_or_default(&h.app.permission_store_dir);
    assert_eq!(
        store.get("test", &workspace, Capability::PanesRead),
        Some(PermissionState::Red),
        "permissions.toml must persist the red decision"
    );
}

/// SetPermission with an unknown capability or state must fail closed with an
/// error reply and persist nothing.
#[test]
fn set_permission_unknown_capability_fails_closed() {
    let mut h = HostHarness::new();
    let manager = h.add_test_pane_with_permissions(AppPermissions::from_capability_strings(&[
        "permissions.manage".to_string(),
    ]));

    let tmp = tempfile::tempdir().expect("tempdir");
    let response_file = tmp.path().join("set-permission.json");
    h.inject(
        manager,
        DrawCommand::Host(AppRequest::SetPermission {
            app_id: "test".to_string(),
            workspace: Some(std::env::temp_dir().to_string_lossy().into_owned()),
            capability: "not.a.capability".to_string(),
            state: "red".to_string(),
            response_file: response_file.to_string_lossy().into_owned(),
        }),
    );
    h.run_frames(2);

    let content = std::fs::read_to_string(&response_file).expect("error reply must be written");
    let v: serde_json::Value = serde_json::from_str(&content).expect("reply must be JSON");
    assert!(
        v["error"]
            .as_str()
            .is_some_and(|e| e.contains("not.a.capability")),
        "error must name the unknown capability: {content}"
    );
}

/// Yellow-state routing end-to-end: a gated request with no stored decision
/// queues the consent prompt; granting via the modal forwards the deferred
/// request into the host handler, which writes the real response.
#[test]
fn yellow_capability_queues_prompt_then_grant_forwards() {
    use crate::app::FocusLayer;
    use crate::host::pane::AppRuntime;
    use crate::process_app::PendingPrompt;

    let mut h = HostHarness::new();
    let pane = h.add_test_pane_with_permissions(AppPermissions::from_capability_strings(&[]));

    let tmp = tempfile::tempdir().expect("tempdir");
    let response_file = tmp.path().join("list-panes.json");
    h.inject(
        pane,
        DrawCommand::Host(AppRequest::ListPanes {
            response_file: response_file.to_string_lossy().into_owned(),
            context_id: None,
        }),
    );
    h.focus_pane(pane);
    h.run_frames(2);

    // Prompt queued, request deferred, nothing written yet.
    {
        let win = &h.app.windows[0];
        let Some(Pane::App(app_pane)) = win.panes.get(&pane) else {
            panic!("expected App pane");
        };
        let AppRuntime::Process(ref proc) = app_pane.runtime else {
            panic!("expected Process runtime");
        };
        assert!(
            proc.pending_prompts.iter().any(|p| {
                matches!(p, PendingPrompt::Capability { capability, .. }
                    if capability == "panes.read")
            }),
            "consent prompt for panes.read must be queued"
        );
        assert!(!response_file.exists());
    }
    assert!(
        h.app.focus_stack.contains(&FocusLayer::CapabilityModal),
        "CapabilityModal must be on the focus stack while the prompt is pending"
    );

    // Grant once via the modal (plain Enter).
    h.key(egui::Key::Enter, egui::Modifiers::NONE);
    h.run_frames(2);

    let content = std::fs::read_to_string(&response_file)
        .expect("granted deferred request must be forwarded and answered");
    let v: serde_json::Value = serde_json::from_str(&content).expect("response must be JSON");
    let panes = v
        .as_array()
        .expect("ListPanes response must be a JSON array");
    assert!(
        panes.iter().any(|p| p["id"].as_u64() == Some(pane)),
        "pane list must contain the requesting pane: {content}"
    );
}

/// Yellow-state routing deny path: denying the consent prompt must answer each
/// deferred request's response_file with the standard denial JSON.
#[test]
fn yellow_capability_deny_writes_denial() {
    let mut h = HostHarness::new();
    let pane = h.add_test_pane_with_permissions(AppPermissions::from_capability_strings(&[]));

    let tmp = tempfile::tempdir().expect("tempdir");
    let response_file = tmp.path().join("list-panes.json");
    h.inject(
        pane,
        DrawCommand::Host(AppRequest::ListPanes {
            response_file: response_file.to_string_lossy().into_owned(),
            context_id: None,
        }),
    );
    h.focus_pane(pane);
    h.run_frames(2);

    // Deny once via the modal (Escape).
    h.key(egui::Key::Escape, egui::Modifiers::NONE);
    h.run_frames(1);

    let content = std::fs::read_to_string(&response_file).expect("deny must write the denial JSON");
    let v: serde_json::Value = serde_json::from_str(&content).expect("response must be JSON");
    assert_eq!(
        v["capability"], "panes.read",
        "denial must name the capability: {content}"
    );
    assert!(
        v["error"]
            .as_str()
            .is_some_and(|e| e.contains("capability denied")),
        "denial must carry the standard error message: {content}"
    );
}

/// A blocked (Red) capability stays flat-denied with no consent prompt — only
/// the yellow/unset middle state routes through the modal.
#[test]
fn red_capability_flat_denies_without_prompt() {
    use crate::app::permissions::Capability;
    use crate::host::pane::AppRuntime;

    let mut h = HostHarness::new();
    let mut perms = AppPermissions::from_capability_strings(&[]);
    perms.blocked.insert(Capability::PanesRead);
    let pane = h.add_test_pane_with_permissions(perms);

    let tmp = tempfile::tempdir().expect("tempdir");
    let response_file = tmp.path().join("list-panes.json");
    h.inject(
        pane,
        DrawCommand::Host(AppRequest::ListPanes {
            response_file: response_file.to_string_lossy().into_owned(),
            context_id: None,
        }),
    );
    h.run_frames(2);

    let content =
        std::fs::read_to_string(&response_file).expect("blocked cap must write denial JSON");
    let v: serde_json::Value = serde_json::from_str(&content).expect("response must be JSON");
    assert_eq!(v["capability"], "panes.read");

    let win = &h.app.windows[0];
    let Some(Pane::App(app_pane)) = win.panes.get(&pane) else {
        panic!("expected App pane");
    };
    let AppRuntime::Process(ref proc) = app_pane.runtime else {
        panic!("expected Process runtime");
    };
    assert!(
        proc.pending_prompts.is_empty(),
        "a blocked capability must not queue a consent prompt"
    );
    assert!(
        proc.deferred_gated_requests.is_empty(),
        "a blocked capability must not defer the request"
    );
}

// -- Unified permissions broker (docs/prm/permissions-broker.md, Phase A) ---

/// A persisted broker allow record auto-grants a CapabilityRequest without
/// showing the consent modal — the PGAP capability path resolves through the
/// unified broker.
#[test]
fn capability_request_auto_granted_from_broker_record() {
    use crate::app_protocol::PlexiEvent;
    use crate::broker::{Decision, GrantRecord, GrantStore};

    let mut h = HostHarness::new();
    let pane = h.add_test_pane_with_permissions(AppPermissions::from_capability_strings(&[]));

    // Seed the pane's broker store with an allow grant for this app+workspace.
    let ws = std::env::temp_dir();
    {
        let win = &mut h.app.windows[0];
        let Some(Pane::App(app_pane)) = win.panes.get_mut(&pane) else {
            panic!("expected App pane");
        };
        let AppRuntime::Process(proc) = &mut app_pane.runtime else {
            panic!("expected Process runtime");
        };
        let mut store = GrantStore::default();
        store.record(GrantRecord::app_capability(
            "test",
            &ws,
            crate::app::permissions::Capability::PanesRead,
            Decision::Allow,
        ));
        proc.grant_store = store;
    }

    h.inject(
        pane,
        DrawCommand::Host(AppRequest::CapabilityRequest {
            request_id: "cap-1".to_string(),
            capability: "panes.read".to_string(),
        }),
    );
    // Drive routing directly via background_tick — frames would flush the
    // outbound events toward the (absent) subprocess before we can assert.
    {
        let win = &mut h.app.windows[0];
        let Some(Pane::App(app_pane)) = win.panes.get_mut(&pane) else {
            panic!("expected App pane");
        };
        let AppRuntime::Process(proc) = &mut app_pane.runtime else {
            panic!("expected Process runtime");
        };
        proc.background_tick();
    }

    let events = h.effects_drain(pane);
    assert!(
        events.iter().any(|e| matches!(
            e,
            PlexiEvent::CapabilityDecision { request_id, granted: true }
                if request_id == "cap-1"
        )),
        "broker allow record must auto-grant without a modal: {events:?}"
    );
    let win = &h.app.windows[0];
    let Some(Pane::App(app_pane)) = win.panes.get(&pane) else {
        panic!("expected App pane");
    };
    let AppRuntime::Process(ref proc) = app_pane.runtime else {
        panic!("expected Process runtime");
    };
    assert!(
        proc.pending_prompts.is_empty(),
        "no consent prompt must be queued for a broker-allowed capability"
    );
    assert!(
        proc.permissions
            .capabilities
            .contains(&crate::app::permissions::Capability::PanesRead),
        "broker allow must populate the in-memory capability set"
    );
}

/// A persisted broker deny record auto-denies a CapabilityRequest without
/// a modal — equivalent of the legacy Red state through the unified broker.
#[test]
fn capability_request_auto_denied_from_broker_record() {
    use crate::app_protocol::PlexiEvent;
    use crate::broker::{Decision, GrantRecord, GrantStore};

    let mut h = HostHarness::new();
    let pane = h.add_test_pane_with_permissions(AppPermissions::from_capability_strings(&[]));

    let ws = std::env::temp_dir();
    {
        let win = &mut h.app.windows[0];
        let Some(Pane::App(app_pane)) = win.panes.get_mut(&pane) else {
            panic!("expected App pane");
        };
        let AppRuntime::Process(proc) = &mut app_pane.runtime else {
            panic!("expected Process runtime");
        };
        let mut store = GrantStore::default();
        store.record(GrantRecord::app_capability(
            "test",
            &ws,
            crate::app::permissions::Capability::PanesControl,
            Decision::Deny,
        ));
        proc.grant_store = store;
    }

    h.inject(
        pane,
        DrawCommand::Host(AppRequest::CapabilityRequest {
            request_id: "cap-2".to_string(),
            capability: "panes.control".to_string(),
        }),
    );
    // Drive routing directly via background_tick — frames would flush the
    // outbound events toward the (absent) subprocess before we can assert.
    {
        let win = &mut h.app.windows[0];
        let Some(Pane::App(app_pane)) = win.panes.get_mut(&pane) else {
            panic!("expected App pane");
        };
        let AppRuntime::Process(proc) = &mut app_pane.runtime else {
            panic!("expected Process runtime");
        };
        proc.background_tick();
    }

    let events = h.effects_drain(pane);
    assert!(
        events.iter().any(|e| matches!(
            e,
            PlexiEvent::CapabilityDecision { request_id, granted: false }
                if request_id == "cap-2"
        )),
        "broker deny record must auto-deny without a modal: {events:?}"
    );
    let win = &h.app.windows[0];
    let Some(Pane::App(app_pane)) = win.panes.get(&pane) else {
        panic!("expected App pane");
    };
    let AppRuntime::Process(ref proc) = app_pane.runtime else {
        panic!("expected Process runtime");
    };
    assert!(
        proc.pending_prompts.is_empty(),
        "no consent prompt must be queued for a broker-denied capability"
    );
}

/// With no broker record and no legacy state, a CapabilityRequest still asks —
/// the broker's default fallback is Ask, preserving the consent-modal flow.
#[test]
fn capability_request_without_grant_still_prompts() {
    use crate::process_app::PendingPrompt;

    let mut h = HostHarness::new();
    let pane = h.add_test_pane_with_permissions(AppPermissions::from_capability_strings(&[]));

    h.inject(
        pane,
        DrawCommand::Host(AppRequest::CapabilityRequest {
            request_id: "cap-3".to_string(),
            capability: "fs.write".to_string(),
        }),
    );
    h.run_frames(2);

    let win = &h.app.windows[0];
    let Some(Pane::App(app_pane)) = win.panes.get(&pane) else {
        panic!("expected App pane");
    };
    let AppRuntime::Process(ref proc) = app_pane.runtime else {
        panic!("expected Process runtime");
    };
    assert!(
        proc.pending_prompts.iter().any(|p| {
            matches!(p, PendingPrompt::Capability { capability, .. } if capability == "fs.write")
        }),
        "broker Ask fallback must queue the consent modal"
    );
}

/// A configured posture deny list auto-denies a CapabilityRequest with no
/// persisted grant — settings provide actor defaults in the evaluation order.
#[test]
fn capability_request_denied_by_posture() {
    use crate::app_protocol::PlexiEvent;
    use crate::broker::{Decision, PermissionPosture};

    let mut h = HostHarness::new();
    let pane = h.add_test_pane_with_permissions(AppPermissions::from_capability_strings(&[]));
    {
        let win = &mut h.app.windows[0];
        let Some(Pane::App(app_pane)) = win.panes.get_mut(&pane) else {
            panic!("expected App pane");
        };
        let AppRuntime::Process(proc) = &mut app_pane.runtime else {
            panic!("expected Process runtime");
        };
        proc.posture = Some(PermissionPosture {
            default_posture: Decision::Ask,
            allow: vec![],
            ask: vec![],
            deny: vec!["net.http".to_string()],
        });
    }

    h.inject(
        pane,
        DrawCommand::Host(AppRequest::CapabilityRequest {
            request_id: "cap-4".to_string(),
            capability: "net.http".to_string(),
        }),
    );
    // Drive routing directly via background_tick — frames would flush the
    // outbound events toward the (absent) subprocess before we can assert.
    {
        let win = &mut h.app.windows[0];
        let Some(Pane::App(app_pane)) = win.panes.get_mut(&pane) else {
            panic!("expected App pane");
        };
        let AppRuntime::Process(proc) = &mut app_pane.runtime else {
            panic!("expected Process runtime");
        };
        proc.background_tick();
    }

    let events = h.effects_drain(pane);
    assert!(
        events.iter().any(|e| matches!(
            e,
            PlexiEvent::CapabilityDecision { request_id, granted: false }
                if request_id == "cap-4"
        )),
        "posture deny must auto-deny without a modal: {events:?}"
    );
}

// -- App events + undo (docs/prm/undo-and-app-events.md, Phase B) -----------

/// Build a minimal valid EmitEvent host command for the declared
/// `move.played` stream, optionally carrying rollback metadata.
fn emit_event_cmd(rollback_token: Option<&str>) -> DrawCommand {
    DrawCommand::Host(AppRequest::EmitEvent {
        event: "move.played".to_string(),
        actor: crate::app_protocol::AppEventActor::User,
        actor_id: None,
        caused_by: None,
        summary: "White played e4".to_string(),
        resource_id: "game-abc".to_string(),
        resource_scope: Some("game".to_string()),
        revision_after: "rev-13".to_string(),
        payload: Some(serde_json::json!({"san": "e4"})),
        state_ref: Some("chess://game/abc/rev/13".to_string()),
        revision_before: Some("rev-12".to_string()),
        rollback_token: rollback_token.map(str::to_string),
        changed_resources: vec![],
        suggested_trigger: None,
    })
}

fn declare_streams_cmd() -> DrawCommand {
    DrawCommand::Host(AppRequest::DeclareEventStreams {
        streams: vec![crate::app_protocol::EventStreamDecl {
            name: "move.played".to_string(),
            schema: serde_json::json!({"type": "object"}),
            description: Some("a chess move".to_string()),
        }],
    })
}

/// A declared + emitted app event enters the host timeline; without rollback
/// metadata no undo checkpoint is created.
#[test]
fn emit_event_enters_host_timeline() {
    let mut h = HostHarness::new();
    let pane = h.add_test_pane();
    h.inject(pane, declare_streams_cmd());
    h.inject(pane, emit_event_cmd(None));
    let proc = h.process_app_mut(pane);
    proc.background_tick();

    let timeline = proc.app_timeline.lock().unwrap();
    assert_eq!(timeline.events().len(), 1, "accepted event must be recorded");
    let rec = &timeline.events()[0];
    assert_eq!(rec.app_id, "test");
    assert_eq!(rec.event, "move.played");
    assert_eq!(rec.summary, "White played e4");
    assert_eq!(rec.resource_id, "game-abc");
    assert_eq!(rec.revision_after, "rev-13");
    assert!(timeline.checkpoints().is_empty(), "no rollback metadata = no checkpoint");
}

/// EmitEvent with a rollback token also creates an undo checkpoint carrying
/// the spec's metadata fields.
#[test]
fn emit_event_with_rollback_token_creates_checkpoint() {
    let mut h = HostHarness::new();
    let pane = h.add_test_pane();
    h.inject(pane, declare_streams_cmd());
    h.inject(pane, emit_event_cmd(Some("undo-abc")));
    let proc = h.process_app_mut(pane);
    proc.background_tick();

    let timeline = proc.app_timeline.lock().unwrap();
    assert_eq!(timeline.checkpoints().len(), 1);
    let ckpt = &timeline.checkpoints()[0];
    assert_eq!(ckpt.app_id, "test");
    assert_eq!(ckpt.rollback_token, "undo-abc");
    assert_eq!(ckpt.revision_before.as_deref(), Some("rev-12"));
    assert_eq!(ckpt.revision_after, "rev-13");
    assert_eq!(ckpt.resource_id, "game-abc");
    assert_eq!(ckpt.summary, "White played e4");
}

/// Malformed events (undeclared stream, empty required field) are rejected
/// and never enter the timeline.
#[test]
fn malformed_emit_event_is_rejected() {
    let mut h = HostHarness::new();
    let pane = h.add_test_pane();

    // Undeclared stream — no DeclareEventStreams sent.
    h.inject(pane, emit_event_cmd(None));
    // Declared, but empty summary.
    h.inject(pane, declare_streams_cmd());
    h.inject(
        pane,
        DrawCommand::Host(AppRequest::EmitEvent {
            event: "move.played".to_string(),
            actor: crate::app_protocol::AppEventActor::App,
            actor_id: None,
            caused_by: None,
            summary: "   ".to_string(),
            resource_id: "game-abc".to_string(),
            resource_scope: None,
            revision_after: "rev-2".to_string(),
            payload: None,
            state_ref: None,
            revision_before: None,
            rollback_token: None,
            changed_resources: vec![],
            suggested_trigger: None,
        }),
    );
    let proc = h.process_app_mut(pane);
    proc.background_tick();
    let timeline = proc.app_timeline.lock().unwrap();
    assert!(
        timeline.events().is_empty(),
        "malformed events must never enter the timeline"
    );
}

/// Full rollback round-trip: broker-allowed request → RollbackVerify to the
/// app → matching RollbackVerifyResult → RollbackApply, checkpoint marked
/// rolled back.
#[test]
fn rollback_round_trip_applies_on_revision_match() {
    use crate::app_protocol::PlexiEvent;
    use crate::broker::{
        ActorScope, ActorType, Decision, GrantDuration, GrantRecord, GrantSource, ResourceScope,
        TargetType,
    };

    let mut h = HostHarness::new();
    let pane = h.add_test_pane();
    h.inject(pane, declare_streams_cmd());
    h.inject(pane, emit_event_cmd(Some("undo-abc")));
    let proc = h.process_app_mut(pane);
    proc.background_tick();
    let ckpt_id = proc.app_timeline.lock().unwrap().checkpoints()[0]
        .checkpoint_id
        .clone();

    // Gate: allow grant for rollback on this app.
    proc.grant_store.record(GrantRecord {
        actor_type: ActorType::App,
        actor_id: "test".to_string(),
        actor_scope: ActorScope::User,
        workspace_root: None,
        target_type: TargetType::UndoCheckpoint,
        target_id: "test".to_string(),
        resource_scope: ResourceScope::Game,
        resource_id: None,
        decision: Decision::Allow,
        duration: GrantDuration::Session,
        source: GrantSource::User,
        created_at: 0,
        expires_at: None,
    });

    h.inject(
        pane,
        DrawCommand::Host(AppRequest::RequestRollback {
            checkpoint_id: ckpt_id.clone(),
        }),
    );
    let proc = h.process_app_mut(pane);
    proc.background_tick();
    let events = h.effects_drain(pane);
    assert!(
        events.iter().any(|e| matches!(
            e,
            PlexiEvent::RollbackVerify { checkpoint_id, expected_revision, .. }
                if *checkpoint_id == ckpt_id && expected_revision == "rev-13"
        )),
        "allowed rollback must dispatch a revision verification: {events:?}"
    );

    // App confirms it is still at rev-13 → host issues the apply.
    h.inject(
        pane,
        DrawCommand::Host(AppRequest::RollbackVerifyResult {
            checkpoint_id: ckpt_id.clone(),
            current_revision: "rev-13".to_string(),
        }),
    );
    let proc = h.process_app_mut(pane);
    proc.background_tick();
    let events = h.effects_drain(pane);
    assert!(
        events.iter().any(|e| matches!(
            e,
            PlexiEvent::RollbackApply { checkpoint_id, rollback_token, .. }
                if *checkpoint_id == ckpt_id && rollback_token == "undo-abc"
        )),
        "revision match must issue RollbackApply: {events:?}"
    );
    let proc = h.process_app_mut(pane);
    assert_eq!(
        proc.app_timeline.lock().unwrap().checkpoints()[0].status,
        crate::host::app_timeline::CheckpointStatus::RolledBack
    );
}

/// Revision mismatch blocks the rollback: no RollbackApply, checkpoint
/// marked conflict, and further rollback attempts are refused.
#[test]
fn rollback_blocked_on_revision_mismatch() {
    use crate::app_protocol::PlexiEvent;
    use crate::broker::{
        ActorScope, ActorType, Decision, GrantDuration, GrantRecord, GrantSource, ResourceScope,
        TargetType,
    };

    let mut h = HostHarness::new();
    let pane = h.add_test_pane();
    h.inject(pane, declare_streams_cmd());
    h.inject(pane, emit_event_cmd(Some("undo-abc")));
    let proc = h.process_app_mut(pane);
    proc.background_tick();
    let ckpt_id = proc.app_timeline.lock().unwrap().checkpoints()[0]
        .checkpoint_id
        .clone();
    proc.grant_store.record(GrantRecord {
        actor_type: ActorType::App,
        actor_id: "test".to_string(),
        actor_scope: ActorScope::User,
        workspace_root: None,
        target_type: TargetType::UndoCheckpoint,
        target_id: "test".to_string(),
        resource_scope: ResourceScope::Game,
        resource_id: None,
        decision: Decision::Allow,
        duration: GrantDuration::Session,
        source: GrantSource::User,
        created_at: 0,
        expires_at: None,
    });

    h.inject(
        pane,
        DrawCommand::Host(AppRequest::RequestRollback {
            checkpoint_id: ckpt_id.clone(),
        }),
    );
    // App has moved on to rev-99 — verification must fail.
    h.inject(
        pane,
        DrawCommand::Host(AppRequest::RollbackVerifyResult {
            checkpoint_id: ckpt_id,
            current_revision: "rev-99".to_string(),
        }),
    );
    let proc = h.process_app_mut(pane);
    proc.background_tick();
    let events = h.effects_drain(pane);
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, PlexiEvent::RollbackApply { .. })),
        "revision mismatch must never apply: {events:?}"
    );
    let proc = h.process_app_mut(pane);
    assert_eq!(
        proc.app_timeline.lock().unwrap().checkpoints()[0].status,
        crate::host::app_timeline::CheckpointStatus::Conflict
    );
}

/// Without a broker allow grant, RequestRollback is blocked (default Ask)
/// and no verification is dispatched.
#[test]
fn rollback_without_grant_is_blocked() {
    use crate::app_protocol::PlexiEvent;

    let mut h = HostHarness::new();
    let pane = h.add_test_pane();
    h.inject(pane, declare_streams_cmd());
    h.inject(pane, emit_event_cmd(Some("undo-abc")));
    let proc = h.process_app_mut(pane);
    proc.background_tick();
    let ckpt_id = proc.app_timeline.lock().unwrap().checkpoints()[0]
        .checkpoint_id
        .clone();

    h.inject(
        pane,
        DrawCommand::Host(AppRequest::RequestRollback {
            checkpoint_id: ckpt_id,
        }),
    );
    let proc = h.process_app_mut(pane);
    proc.background_tick();
    let events = h.effects_drain(pane);
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, PlexiEvent::RollbackVerify { .. })),
        "ungated rollback must not dispatch verification: {events:?}"
    );
    let proc = h.process_app_mut(pane);
    assert_eq!(
        proc.app_timeline.lock().unwrap().checkpoints()[0].status,
        crate::host::app_timeline::CheckpointStatus::Active,
        "blocked rollback must leave the checkpoint untouched"
    );
}

/// ListUndoCheckpoints returns the app's checkpoints, newest first.
#[test]
fn list_undo_checkpoints_responds_with_serialized_records() {
    use crate::app_protocol::PlexiEvent;

    let mut h = HostHarness::new();
    let pane = h.add_test_pane();
    h.inject(pane, declare_streams_cmd());
    h.inject(pane, emit_event_cmd(Some("undo-abc")));
    h.inject(
        pane,
        DrawCommand::Host(AppRequest::ListUndoCheckpoints {
            request_id: "lc-1".to_string(),
            app_id: None,
        }),
    );
    let proc = h.process_app_mut(pane);
    proc.background_tick();
    let events = h.effects_drain(pane);
    let listed = events.iter().find_map(|e| match e {
        PlexiEvent::UndoCheckpoints { request_id, checkpoints } if request_id == "lc-1" => {
            Some(checkpoints.clone())
        }
        _ => None,
    });
    let listed = listed.expect("ListUndoCheckpoints must answer with UndoCheckpoints");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0]["rollback_token"], "undo-abc");
    assert_eq!(listed[0]["revision_after"], "rev-13");
    assert_eq!(listed[0]["status"], "active");
}

/// Wire subscriptions are broker-gated: without an allow grant the request
/// is refused; with one, events route to the subscriber pane as
/// PlexiEvent::AppEvent shaped by the payload mode.
#[test]
fn subscribe_app_events_gated_and_delivers() {
    use crate::app_protocol::{PayloadMode, PlexiEvent, TriggerMode};
    use crate::broker::{
        ActorScope, ActorType, Decision, GrantDuration, GrantRecord, GrantSource, ResourceScope,
        TargetType,
    };

    let mut h = HostHarness::new();
    let publisher = h.add_test_pane();
    let subscriber = h.add_test_pane();

    // Share one timeline between the two panes and give the subscriber its
    // own identity.
    let shared = h.process_app_mut(publisher).app_timeline.clone();
    {
        let proc = h.process_app_mut(subscriber);
        proc.app_timeline = shared;
        proc.type_id = "agent-app".to_string();
    }

    h.inject(publisher, declare_streams_cmd());
    h.process_app_mut(publisher).background_tick();

    let subscribe_cmd = |request_id: &str| {
        DrawCommand::Host(AppRequest::SubscribeAppEvents {
            request_id: request_id.to_string(),
            app_id: "test".to_string(),
            event_names: vec!["move.played".to_string()],
            payload_mode: PayloadMode::Summary,
            trigger_mode: TriggerMode::Conversation,
            resource_id: None,
        })
    };

    // No grant → broker default Ask → refused.
    h.inject(subscriber, subscribe_cmd("sub-req-1"));
    h.process_app_mut(subscriber).background_tick();
    let events = h.effects_drain(subscriber);
    assert!(
        events.iter().any(|e| matches!(
            e,
            PlexiEvent::AppEventsSubscribed { request_id, subscription_id: None, error: Some(_) }
                if request_id == "sub-req-1"
        )),
        "ungranted subscription must be refused: {events:?}"
    );

    // Allow grant for this subscriber on the publisher's stream.
    h.process_app_mut(subscriber).grant_store.record(GrantRecord {
        actor_type: ActorType::App,
        actor_id: "agent-app".to_string(),
        actor_scope: ActorScope::User,
        workspace_root: None,
        target_type: TargetType::AppEventStream,
        target_id: "test::move.played".to_string(),
        resource_scope: ResourceScope::Game,
        resource_id: None,
        decision: Decision::Allow,
        duration: GrantDuration::Session,
        source: GrantSource::User,
        created_at: 0,
        expires_at: None,
    });
    h.inject(subscriber, subscribe_cmd("sub-req-2"));
    h.process_app_mut(subscriber).background_tick();
    let events = h.effects_drain(subscriber);
    assert!(
        events.iter().any(|e| matches!(
            e,
            PlexiEvent::AppEventsSubscribed { request_id, subscription_id: Some(_), error: None }
                if request_id == "sub-req-2"
        )),
        "granted subscription must succeed: {events:?}"
    );

    // Publisher emits — subscriber receives an AppEvent with summary only.
    h.inject(publisher, emit_event_cmd(None));
    h.process_app_mut(publisher).background_tick();
    let proc = h.process_app_mut(subscriber);
    proc.deliver_subscribed_events();
    let events = h.effects_drain(subscriber);
    let delivered = events.iter().find_map(|e| match e {
        PlexiEvent::AppEvent {
            app_id,
            event,
            trigger_mode,
            summary,
            payload,
            state_ref,
            ..
        } => Some((
            app_id.clone(),
            event.clone(),
            *trigger_mode,
            summary.clone(),
            payload.clone(),
            state_ref.clone(),
        )),
        _ => None,
    });
    let (app_id, event, trigger, summary, payload, state_ref) =
        delivered.expect("subscribed event must be delivered to the subscriber pane");
    assert_eq!(app_id, "test");
    assert_eq!(event, "move.played");
    assert_eq!(trigger, TriggerMode::Conversation);
    assert_eq!(summary.as_deref(), Some("White played e4"));
    assert!(payload.is_none(), "summary mode must not deliver the payload");
    assert!(state_ref.is_none(), "summary mode must not deliver the state ref");
}

/// Subscribing to an undeclared stream name is refused before the broker is
/// even consulted.
#[test]
fn subscribe_undeclared_stream_is_refused() {
    use crate::app_protocol::{PayloadMode, PlexiEvent, TriggerMode};

    let mut h = HostHarness::new();
    let pane = h.add_test_pane();
    h.inject(
        pane,
        DrawCommand::Host(AppRequest::SubscribeAppEvents {
            request_id: "sub-req-x".to_string(),
            app_id: "test".to_string(),
            event_names: vec!["never.declared".to_string()],
            payload_mode: PayloadMode::Full,
            trigger_mode: TriggerMode::Ambient,
            resource_id: None,
        }),
    );
    let proc = h.process_app_mut(pane);
    proc.background_tick();
    let events = h.effects_drain(pane);
    assert!(
        events.iter().any(|e| matches!(
            e,
            PlexiEvent::AppEventsSubscribed { request_id, subscription_id: None, error: Some(err) }
                if request_id == "sub-req-x" && err.contains("never.declared")
        )),
        "undeclared stream subscription must be refused: {events:?}"
    );
}

// -- Phase C: host agent runtime (docs/prm/chess-agent-poc.md) ---------------

mod agent_runtime {
    use super::*;
    use crate::agent::{AgentDefinition, AgentHost};
    use crate::app_protocol::{AiTool, PlexiEvent};
    use crate::broker::{
        ActorScope, ActorType, Decision, GrantDuration, GrantRecord, GrantSource, ResourceScope,
        TargetType,
    };
    use crate::host::app_timeline::{AppTimeline, EmittedEvent};
    use crate::plexi_ai::broker::{AiBroker, AiBrokerRequest, AiBrokerResponse};
    use crate::plexi_ai::tool_dispatch;
    use std::sync::{Arc, Mutex};

    const CHESS_PANE: u64 = 7001;

    const AGENT_SETTINGS: &str = r#"
[agent]
id = "chess-opponent"
display_name = "Chess Opponent"
default_tier = "low"

[permissions]
default_posture = "review"
allow = ["app.chess.current_state", "app.chess.legal_moves"]
ask = ["app.chess.make_move", "app.chess.undo_move"]

[[subscriptions]]
app = "chess"
events = ["turn.ready", "move.played"]
payload = "full"
trigger = "conversation"
default = "ask"
"#;

    fn chess_def() -> AgentDefinition {
        AgentDefinition::parse("You are a chess opponent.", AGENT_SETTINGS).unwrap()
    }

    fn chess_timeline() -> Arc<Mutex<AppTimeline>> {
        let mut t = AppTimeline::default();
        t.declare_streams(
            "chess",
            ["turn.ready", "move.played"]
                .into_iter()
                .map(|name| crate::app_protocol::EventStreamDecl {
                    name: name.to_string(),
                    schema: serde_json::json!({"type": "object"}),
                    description: None,
                })
                .collect(),
        )
        .unwrap();
        Arc::new(Mutex::new(t))
    }

    fn turn_ready() -> EmittedEvent {
        EmittedEvent {
            event: "turn.ready".to_string(),
            actor: crate::app_protocol::AppEventActor::App,
            actor_id: None,
            caused_by: None,
            summary: "black to move in game-1".to_string(),
            resource_id: "game-1".to_string(),
            resource_scope: Some("game".to_string()),
            revision_after: "rev-1".to_string(),
            payload: Some(serde_json::json!({"legal_moves": ["Nf6", "e5"]})),
            state_ref: None,
            revision_before: None,
            rollback_token: None,
            changed_resources: vec![],
            suggested_trigger: None,
        }
    }

    fn subscription_grant(event: &str) -> GrantRecord {
        GrantRecord {
            actor_type: ActorType::Agent,
            actor_id: "chess-opponent".to_string(),
            actor_scope: ActorScope::Workspace,
            workspace_root: None,
            target_type: TargetType::AppEventStream,
            target_id: format!("chess::{event}"),
            resource_scope: ResourceScope::Game,
            resource_id: None,
            decision: Decision::Allow,
            duration: GrantDuration::Session,
            source: GrantSource::User,
            created_at: 0,
            expires_at: None,
        }
    }

    fn make_move_grant() -> GrantRecord {
        GrantRecord {
            actor_type: ActorType::Agent,
            actor_id: "chess-opponent".to_string(),
            actor_scope: ActorScope::Workspace,
            workspace_root: None,
            target_type: TargetType::AppConnector,
            target_id: "app.chess.make_move".to_string(),
            resource_scope: ResourceScope::Game,
            resource_id: Some("game-1".to_string()),
            decision: Decision::Allow,
            duration: GrantDuration::Session,
            source: GrantSource::User,
            created_at: 0,
            expires_at: None,
        }
    }

    fn chess_tools() -> Vec<AiTool> {
        ["chess.current_state", "chess.legal_moves", "chess.make_move"]
            .into_iter()
            .map(|name| AiTool {
                name: name.to_string(),
                description: format!("test {name}"),
                input_schema: serde_json::json!({"type": "object", "properties": {}}),
                timeout_ms: Some(2_000),
            })
            .collect()
    }

    /// Register the chess tools and spawn a thread acting as the chess app:
    /// it answers `ToolCall`s for `chess.make_move` with the spec's output
    /// shape and records the resulting reversible `move.played` event on the
    /// shared timeline (what the real app's emit_event would do).
    fn register_chess_app(timeline: Arc<Mutex<AppTimeline>>, workspace: std::path::PathBuf) {
        let (tx, rx) = std::sync::mpsc::channel::<crate::process_app::StdinItem>();
        tool_dispatch::register(
            CHESS_PANE,
            chess_tools(),
            tool_dispatch::AppEventSender { tx },
            workspace,
        );
        std::thread::spawn(move || {
            while let Ok(item) = rx.recv() {
                let crate::process_app::StdinItem::Event(json) = item else {
                    continue;
                };
                let Ok(ev) = serde_json::from_str::<PlexiEvent>(&json) else {
                    continue;
                };
                if let PlexiEvent::ToolCall { call_id, name, .. } = ev {
                    assert_eq!(name, "chess.make_move");
                    // The app validates and applies the move…
                    let mut move_played = turn_ready();
                    move_played.event = "move.played".to_string();
                    move_played.actor = crate::app_protocol::AppEventActor::Agent;
                    move_played.summary = "Black played Nf6".to_string();
                    move_played.revision_before = Some("rev-1".to_string());
                    move_played.revision_after = "rev-2".to_string();
                    move_played.rollback_token = Some("move-2".to_string());
                    timeline
                        .lock()
                        .unwrap()
                        .record_event("chess", CHESS_PANE, move_played)
                        .expect("app's move.played must be accepted");
                    // …and answers the tool call with the spec output shape.
                    tool_dispatch::resolve_pending(
                        &call_id,
                        tool_dispatch::ToolCallResult {
                            output_json: Some(
                                serde_json::json!({
                                    "ok": true,
                                    "summary": "Black played Nf6",
                                    "revision_before": "rev-1",
                                    "revision_after": "rev-2",
                                    "rollback_token": "move-2",
                                    "changed_resources": ["game-1"],
                                })
                                .to_string(),
                            ),
                            error: None,
                        },
                    );
                }
            }
        });
    }

    /// Mock model: calls `chess.make_move` through the gated dispatcher when
    /// it is visible, otherwise replies with commentary only.
    struct MoveMakingBroker;

    impl AiBroker for MoveMakingBroker {
        fn dispatch(
            &self,
            request: AiBrokerRequest,
            _on_delta: &mut dyn FnMut(crate::plexi_ai::turn_loop::TurnDelta<'_>),
        ) -> AiBrokerResponse {
            let dispatcher = request
                .tool_dispatcher
                .as_ref()
                .expect("agent turns must carry a tool dispatcher");
            let visible: Vec<String> =
                dispatcher.all_tools().into_iter().map(|t| t.name).collect();
            if visible.iter().any(|n| n == "chess.make_move") {
                let result = dispatcher.dispatch_call(
                    "call-1".to_string(),
                    "chess.make_move",
                    serde_json::json!({"game_id": "game-1", "move": "Nf6", "notation": "san"})
                        .to_string(),
                );
                assert!(
                    result.error.is_none(),
                    "granted make_move must dispatch: {:?}",
                    result.error
                );
                AiBrokerResponse::ok("I play Nf6.".to_string(), 1, 1)
            } else {
                // Denied: prove invocation is blocked too, not just hidden.
                let result = dispatcher.dispatch_call(
                    "call-x".to_string(),
                    "chess.make_move",
                    "{}".to_string(),
                );
                assert!(
                    result
                        .error
                        .as_deref()
                        .unwrap_or_default()
                        .contains("tool_not_found"),
                    "denied tool must be uninvocable: {result:?}"
                );
                AiBrokerResponse::ok(
                    "A fine position. I would play Nf6, but I am not allowed to move."
                        .to_string(),
                    1,
                    1,
                )
            }
        }
    }

    fn wait_for_agent_reply(host: &mut AgentHost) -> Vec<crate::agent::TranscriptEntry> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            host.tick();
            let agent = &host.agents[0];
            if agent.transcript.iter().any(|e| e.role == "agent" || e.role == "error") {
                return agent.transcript.clone();
            }
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for agent reply; transcript: {:?}",
                agent.transcript
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    /// End-to-end happy path: turn.ready delivery → conversation turn →
    /// mocked model calls chess.make_move → broker allows → app applies and
    /// emits move.played → checkpoint exists → reply lands in the transcript.
    #[test]
    fn agent_turn_makes_granted_move_and_checkpoints() {
        // Each test gets its own workspace dir — the tool registry is a
        // process-global filtered by workspace, and tests run in parallel.
        let ws = tempfile::tempdir().unwrap();
        let timeline = chess_timeline();
        register_chess_app(Arc::clone(&timeline), ws.path().to_path_buf());
        let mut host = AgentHost::new_for_test(
            Arc::clone(&timeline),
            Arc::new(MoveMakingBroker),
            ws.path().to_path_buf(),
        );
        host.grant_store.record(subscription_grant("turn.ready"));
        host.grant_store.record(subscription_grant("move.played"));
        host.grant_store.record(make_move_grant());
        host.attach(chess_def());
        assert_eq!(
            host.agents[0].subscription_ids.len(),
            1,
            "granted subscription must be created"
        );

        // The chess app emits turn.ready → queued for the agent.
        let outcome = timeline
            .lock()
            .unwrap()
            .record_event("chess", CHESS_PANE, turn_ready())
            .unwrap();
        assert_eq!(outcome.deliveries_queued, 1);

        let transcript = wait_for_agent_reply(&mut host);
        assert!(
            transcript
                .iter()
                .any(|e| e.role == "event" && e.text.contains("turn.ready")),
            "injected event must be in the transcript: {transcript:?}"
        );
        assert!(
            transcript
                .iter()
                .any(|e| e.role == "agent" && e.text == "I play Nf6."),
            "agent reply must land in the transcript: {transcript:?}"
        );

        let t = timeline.lock().unwrap();
        assert!(
            t.events().iter().any(|e| e.event == "move.played"),
            "the app must have applied the move"
        );
        assert_eq!(t.checkpoints().len(), 1, "reversible move must checkpoint");
        assert_eq!(t.checkpoints()[0].rollback_token, "move-2");

        tool_dispatch::unregister(CHESS_PANE);
    }

    /// Denied `chess.make_move` (posture ask, no user grant): the agent can
    /// comment but the tool is invisible and uninvocable — no move, no
    /// checkpoint.
    #[test]
    fn denied_make_move_lets_agent_comment_but_not_move() {
        const PANE: u64 = 7002;
        let ws = tempfile::tempdir().unwrap();
        let timeline = chess_timeline();
        // Register tools on a different pane id with no app thread — a
        // dispatch attempt would time out, but the broker gate must prevent
        // it from ever being sent.
        let (tx, _rx) = std::sync::mpsc::channel::<crate::process_app::StdinItem>();
        tool_dispatch::register(
            PANE,
            chess_tools(),
            tool_dispatch::AppEventSender { tx },
            ws.path().to_path_buf(),
        );

        let mut host = AgentHost::new_for_test(
            Arc::clone(&timeline),
            Arc::new(MoveMakingBroker),
            ws.path().to_path_buf(),
        );
        host.grant_store.record(subscription_grant("turn.ready"));
        host.grant_store.record(subscription_grant("move.played"));
        // No make_move grant — the settings put it in `ask`.
        host.attach(chess_def());

        timeline
            .lock()
            .unwrap()
            .record_event("chess", PANE, turn_ready())
            .unwrap();

        let transcript = wait_for_agent_reply(&mut host);
        assert!(
            transcript
                .iter()
                .any(|e| e.role == "agent" && e.text.contains("not allowed to move")),
            "agent must still be able to comment: {transcript:?}"
        );
        let t = timeline.lock().unwrap();
        assert!(
            !t.events().iter().any(|e| e.event == "move.played"),
            "denied make_move must not produce a move"
        );
        assert!(t.checkpoints().is_empty());

        tool_dispatch::unregister(PANE);
    }

    /// Denied subscription: no grant records → attach creates no
    /// subscription → emitted events are never delivered to the agent.
    #[test]
    fn denied_subscription_means_agent_sees_nothing() {
        let timeline = chess_timeline();
        let mut host = AgentHost::new_for_test(
            Arc::clone(&timeline),
            Arc::new(MoveMakingBroker),
            std::env::temp_dir(),
        );
        host.attach(chess_def());
        assert!(
            host.agents[0].subscription_ids.is_empty(),
            "ungranted subscription must not be created"
        );

        let outcome = timeline
            .lock()
            .unwrap()
            .record_event("chess", CHESS_PANE, turn_ready())
            .unwrap();
        assert_eq!(outcome.deliveries_queued, 0, "no subscription = no delivery");

        host.tick();
        assert!(
            host.agents[0].transcript.is_empty(),
            "agent must see nothing without a subscription grant"
        );
        assert!(!host.turns_in_flight());
    }

    /// Cross-pane rollback (Phase B's deferred seam): another pane's
    /// RequestRollback for a chess checkpoint routes RollbackVerify to the
    /// chess pane through the tool registry sender.
    #[test]
    fn cross_pane_rollback_routes_verify_to_owning_pane() {
        const PANE: u64 = 7003;
        let mut h = HostHarness::new();
        let requester = h.add_test_pane();
        let timeline = h.process_app_mut(requester).app_timeline.clone();

        // Chess app: declared stream + reversible event recorded from PANE,
        // reachable through the registry.
        timeline
            .lock()
            .unwrap()
            .declare_streams(
                "chess",
                vec![crate::app_protocol::EventStreamDecl {
                    name: "move.played".to_string(),
                    schema: serde_json::json!({"type": "object"}),
                    description: None,
                }],
            )
            .unwrap();
        let (tx, rx) = std::sync::mpsc::channel::<crate::process_app::StdinItem>();
        tool_dispatch::register(
            PANE,
            chess_tools(),
            tool_dispatch::AppEventSender { tx },
            std::env::temp_dir(),
        );
        let mut reversible = turn_ready();
        reversible.event = "move.played".to_string();
        reversible.rollback_token = Some("move-1".to_string());
        reversible.revision_before = Some("rev-0".to_string());
        let ckpt_id = timeline
            .lock()
            .unwrap()
            .record_event("chess", PANE, reversible)
            .unwrap()
            .checkpoint_id
            .unwrap();

        // Allow the requesting app to roll back chess checkpoints.
        h.process_app_mut(requester).grant_store.record(GrantRecord {
            actor_type: ActorType::App,
            actor_id: "test".to_string(),
            actor_scope: ActorScope::User,
            workspace_root: None,
            target_type: TargetType::UndoCheckpoint,
            target_id: "chess".to_string(),
            resource_scope: ResourceScope::Game,
            resource_id: None,
            decision: Decision::Allow,
            duration: GrantDuration::Session,
            source: GrantSource::User,
            created_at: 0,
            expires_at: None,
        });

        h.process_app_mut(requester)
            .request_rollback(ActorType::App, "test", &ckpt_id)
            .expect("cross-pane rollback must route through the registry");

        // The verify event must arrive on the chess pane's channel.
        let verify = rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("RollbackVerify must be delivered to the owning pane");
        let crate::process_app::StdinItem::Event(json) = verify else {
            panic!("expected an Event item");
        };
        let ev: PlexiEvent = serde_json::from_str(&json).unwrap();
        match ev {
            PlexiEvent::RollbackVerify {
                checkpoint_id,
                expected_revision,
                ..
            } => {
                assert_eq!(checkpoint_id, ckpt_id);
                assert_eq!(expected_revision, "rev-1");
            }
            other => panic!("expected RollbackVerify, got {other:?}"),
        }

        tool_dispatch::unregister(PANE);
    }
}

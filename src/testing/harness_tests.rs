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
    assert!(
        response["error"]
            .as_str()
            .expect("error")
            .contains("slot 'missing' not found")
    );
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
    assert!(error.contains("slot 'artifact'"), "unexpected error: {error}");
    assert!(error.contains("10485761"), "unexpected error: {error}");
}

#[test]
fn pane_slot_append_uses_tracked_path_after_context_root_changes() {
    let first_root = tempfile::tempdir().expect("first root");
    let second_root = tempfile::tempdir().expect("second root");
    let mut h = HostHarness::new();
    h.app.set_active_context_root(first_root.path().to_path_buf());
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
        h.app
            .focus_stack
            .iter()
            .any(|l| *l == FocusLayer::CapabilityModal),
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
        !h.app
            .focus_stack
            .iter()
            .any(|l| *l == FocusLayer::CapabilityModal),
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
        h.app
            .focus_stack
            .iter()
            .any(|l| *l == FocusLayer::ConfirmClose),
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
        h.app
            .focus_stack
            .iter()
            .any(|l| *l == FocusLayer::ConfirmClose),
        "ConfirmClose must still be in the stack (buried beneath CommandPalette)"
    );

    // Clear ConfirmClose source state — sync must remove the buried entry.
    h.app.pending_close = false;
    h.app.sync_confirm_close_focus();

    assert!(
        !h.app.focus_stack.iter().any(|l| *l == FocusLayer::ConfirmClose),
        "ConfirmClose must be removed from the stack even though CommandPalette was on top. \
         If this fails, sync_confirm_close_focus used pop_focus_layer (top-only) instead of retain."
    );

    // The layer that was on top must still be present — we only removed ConfirmClose.
    assert!(
        h.app
            .focus_stack
            .iter()
            .any(|l| *l == FocusLayer::CommandPalette),
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
        h.app.focus_stack.iter().any(|l| *l == FocusLayer::CapabilityModal),
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

/// App WITHOUT `panes.read` sends ListPanes via PGAP: the request must not be
/// forwarded — instead the capability gate writes a JSON error object to the
/// response_file so the caller doesn't hang until its poll timeout.
#[test]
fn pgap_list_panes_denied_without_panes_read() {
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

    let content =
        std::fs::read_to_string(&response_file).expect("denial must write the response file");
    let v: serde_json::Value = serde_json::from_str(&content).expect("response must be JSON");
    assert_eq!(
        v["capability"], "panes.read",
        "denial object must name the missing capability: {content}"
    );
    assert!(
        v["error"]
            .as_str()
            .is_some_and(|e| e.contains("capability denied")),
        "denial object must carry an error message: {content}"
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

/// App WITHOUT `panes.control` sends SendToPane via PGAP: the gate must write
/// the capability-denied error object instead of forwarding.
#[test]
fn pgap_send_to_pane_denied_without_panes_control() {
    let mut h = HostHarness::new();
    let pane = h.add_test_pane_with_permissions(AppPermissions::from_capability_strings(&[
        "panes.read".to_string(), // read alone must NOT grant control
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

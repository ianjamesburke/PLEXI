use super::*;

fn add_app_pane_to_window(h: &mut HostHarness, window_id: u64, manifest_id: &str) {
    let win_idx = h
        .app
        .windows
        .iter()
        .position(|window| window.window_id == window_id)
        .expect("test window must exist");
    h.app.add_app_pane_in_window(win_idx, manifest_id);
}

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

#[derive(Default)]
struct TextInputProbe {
    text: String,
    enter_handled: bool,
    enter_rendered: bool,
    consume_enter: bool,
}

impl crate::app::app_trait::App for TextInputProbe {
    fn type_id(&self) -> &'static str {
        "text-input-probe"
    }

    fn display_name(&self) -> String {
        "Text Input Probe".to_string()
    }

    fn ui(&mut self, ui: &mut egui::Ui, _ctx: &crate::app::app_trait::AppRenderContext<'_>) {
        if ui.input(|input| input.key_pressed(egui::Key::Enter)) {
            self.enter_rendered = true;
        }
        let response = ui.text_edit_singleline(&mut self.text);
        response.request_focus();
    }

    fn handle_key(
        &mut self,
        input: &crate::app::input_router::PlexiInput,
    ) -> crate::app::app_trait::KeyDisposition {
        if input.key_pressed(egui::Key::Enter) {
            self.enter_handled = true;
            if self.consume_enter {
                return crate::app::app_trait::KeyDisposition::Consumed;
            }
        }
        crate::app::app_trait::KeyDisposition::Passthrough
    }

    fn serialize_state(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "text": self.text,
            "enter_handled": self.enter_handled,
            "enter_rendered": self.enter_rendered,
        }))
    }
}

#[test]
fn send_to_app_pane_injects_text_through_focused_render_input() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut h = HostHarness::new();
    h.app.open_builtin_app_pane(
        Box::<TextInputProbe>::default(),
        AppPermissions::builtin(),
        tmp.path().to_path_buf(),
        None,
        Some("split_h"),
        None,
    );
    let pane_id = h.state().open_panes[0];
    h.run_frames(2);
    let response_file = temp_response(tmp.path(), "send-text");

    h.inject_ipc(AppRequest::SendToPane {
        pane_id,
        text: "/settings".to_string(),
        response_file: Some(response_file.clone()),
    });
    h.run_frames(1);

    assert_eq!(read_json_response(&response_file)["ok"], true);
    let state = h.app.windows[0]
        .panes
        .get(&pane_id)
        .and_then(Pane::as_app)
        .and_then(|pane| pane.runtime.serialize_state())
        .expect("probe state");
    assert_eq!(state["text"], "/settings");

    let key_response = temp_response(tmp.path(), "key-enter");
    h.inject_ipc(AppRequest::KeyPane {
        pane_id,
        key: "enter".to_string(),
        response_file: Some(key_response.clone()),
    });
    h.run_frames(1);

    let response = read_json_response(&key_response);
    assert_eq!(response["ok"], true);
    assert_eq!(response["disposition"], "passthrough");
    let state = h.app.windows[0]
        .panes
        .get(&pane_id)
        .and_then(Pane::as_app)
        .and_then(|pane| pane.runtime.serialize_state())
        .expect("probe state after Enter");
    assert_eq!(state["enter_handled"], true);
    assert_eq!(state["enter_rendered"], true);
}

#[test]
fn consumed_native_key_is_not_replayed_into_render_input() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut h = HostHarness::new();
    h.app.open_builtin_app_pane(
        Box::new(TextInputProbe {
            consume_enter: true,
            ..Default::default()
        }),
        AppPermissions::builtin(),
        tmp.path().to_path_buf(),
        None,
        Some("split_h"),
        None,
    );
    let pane_id = h.state().open_panes[0];
    h.run_frames(2);
    let response_file = temp_response(tmp.path(), "consumed-enter");

    h.inject_ipc(AppRequest::KeyPane {
        pane_id,
        key: "enter".to_string(),
        response_file: Some(response_file.clone()),
    });
    h.run_frames(1);

    let response = read_json_response(&response_file);
    assert_eq!(response["disposition"], "consumed");
    let state = h.app.windows[0]
        .panes
        .get(&pane_id)
        .and_then(Pane::as_app)
        .and_then(|pane| pane.runtime.serialize_state())
        .expect("consuming probe state");
    assert_eq!(state["enter_handled"], true);
    assert_eq!(state["enter_rendered"], false);
}


/// Stint 0387: a real keyboard event injected via `RawInput` must reach the
/// focused app pane's `handle_key` through the migrated `PlexiInput`
/// ownership-transfer router in `dispatch_app_key_events` — not just the
/// synthesized `plexi pane key` IPC path. Proves the app-dispatch migration is
/// wired into the live frame, reading the frame's owned buffer.
#[test]
fn raw_key_event_reaches_focused_app_via_router() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut h = HostHarness::new();
    h.app.open_builtin_app_pane(
        Box::<TextInputProbe>::default(),
        AppPermissions::builtin(),
        tmp.path().to_path_buf(),
        None,
        Some("split_h"),
        None,
    );
    let pane_id = h.state().open_panes[0];
    h.focus_pane(pane_id);
    h.run_frames(2);

    // Drive a genuine Enter key event through a real frame's RawInput.
    h.frame(egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1280.0, 800.0),
        )),
        events: vec![egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
        ..Default::default()
    });

    let state = h.app.windows[0]
        .panes
        .get(&pane_id)
        .and_then(Pane::as_app)
        .and_then(|pane| pane.runtime.serialize_state())
        .expect("probe state after Enter");
    assert_eq!(
        state["enter_handled"], true,
        "RawInput Enter must reach handle_key through the ownership router"
    );
}

#[test]
fn pane_slots_write_read_list_delete() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut h = HostHarness::new();
    h.app.set_context_root(tmp.path().to_path_buf(), None);
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
    h.app.set_context_root(tmp.path().to_path_buf(), None);
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
    h.app.set_context_root(tmp.path().to_path_buf(), None);
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
    h.app.set_context_root(tmp.path().to_path_buf(), None);
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
    h.app.set_context_root(tmp.path().to_path_buf(), None);
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
    h.app.set_context_root(tmp.path().to_path_buf(), None);
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
        .set_context_root(first_root.path().to_path_buf(), None);
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
        .set_context_root(second_root.path().to_path_buf(), None);
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
    h.app.set_context_root(tmp.path().to_path_buf(), None);
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
    h.app.set_context_root(tmp.path().to_path_buf(), None);
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
    h.app.set_context_root(tmp.path().to_path_buf(), None);
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
fn notification_modal_handle_key_returns_consumed() {
    use crate::app::app_trait::KeyDisposition;
    use crate::app::FocusKind;
    let mut h = HostHarness::new();
    h.app.push_focus_layer(FocusKind::NotificationModal);
    let ctx = h.app.ctx.clone();
    let mut input = crate::app::input_router::PlexiInput::take_from(&ctx);
    let disposition = h.app.notification_modal_handle_key(&mut input);
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
    h.app.set_context_root(root.clone(), None);
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
fn buried_stale_focus_layer_is_removed_by_sync() {
    use crate::app::FocusKind;

    let mut h = HostHarness::new();

    // Push ConfirmClose by activating its source state and running sync.
    h.app.pending_close = true;
    h.app.sync_confirm_close_focus();
    assert!(
        h.app.focus_stack.contains(&FocusKind::ConfirmClose),
        "ConfirmClose must be pushed when pending_close is true"
    );

    // Push CommandPalette on top — now ConfirmClose is buried.
    h.app.show_command_palette = true;
    h.app.sync_command_palette_focus();
    assert_eq!(
        h.app.focus_stack.last(),
        Some(&FocusKind::CommandPalette),
        "CommandPalette must be at the top after its source state becomes true"
    );
    assert!(
        h.app.focus_stack.contains(&FocusKind::ConfirmClose),
        "ConfirmClose must still be in the stack (buried beneath CommandPalette)"
    );

    // Clear ConfirmClose source state — sync must remove the buried entry.
    h.app.pending_close = false;
    h.app.sync_confirm_close_focus();

    assert!(
        !h.app.focus_stack.contains(&FocusKind::ConfirmClose),
        "ConfirmClose must be removed from the stack even though CommandPalette was on top. \
         If this fails, sync_confirm_close_focus used pop_focus_layer (top-only) instead of retain."
    );

    // The layer that was on top must still be present — we only removed ConfirmClose.
    assert!(
        h.app.focus_stack.contains(&FocusKind::CommandPalette),
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
    h.app.push_focus_layer(crate::app::FocusKind::QuickNote);

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

    h.app.push_focus_layer(crate::app::FocusKind::QuickNote);
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
    use crate::app::FocusKind;
    let mut h = HostHarness::new();
    let pane = h.add_test_pane();
    h.app.renaming_pane = Some(pane);
    h.app.rename_buffer = "test name".to_string();
    h.app.push_focus_layer(FocusKind::RenamePane);
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
    use crate::app::FocusKind;
    let mut h = HostHarness::new();
    h.app.renaming_window = Some(0);
    h.app.rename_buffer = "new context".to_string();
    h.app.sidebar_visible = false;
    h.app.push_focus_layer(FocusKind::ContextRename);
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
    use crate::app::FocusKind;
    let mut h = HostHarness::new();
    h.app.editing_description = Some(0);
    h.app.description_buffer = "my description".to_string();
    h.app.push_focus_layer(FocusKind::ContextDescription);
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
    use crate::app::{FocusKind, OverlayTarget, TextInputOverlay};
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
    h.app.push_focus_layer(FocusKind::TextInput);
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
fn portal_context_state_refreshes_when_child_context_changes() {
    let mut h = HostHarness::new();
    let _root_pane = h.add_test_pane();
    let root_ctx_id = h.app.router.active().context_id;

    // Child context with its own window and one pane.
    let child_ctx_id = 77001u64;
    let child_win_id = 77002u64;
    h.app.router.push(crate::host::context::Context {
        name: "child".to_string(),
        path: std::path::PathBuf::from("/tmp/harness_2023_child"),
        root: None,
        description: None,
        context_id: child_ctx_id,
        parent_id: Some(root_ctx_id),
        depth: 1,
        parked: false,
    });
    h.app
        .context_active_window
        .insert(child_ctx_id, child_win_id);
    h.app.windows.push(crate::host::context::Window {
        name: String::new(),
        path: std::path::PathBuf::from("/tmp/harness_2023_child"),
        tree: egui_tiles::Tree::empty("harness_2023_child"),
        panes: HashMap::new(),
        focused_pane: None,
        zoomed_pane: None,
        grid_x: 0,
        grid_y: 0,
        window_id: child_win_id,
        context_id: child_ctx_id,
    });
    add_app_pane_to_window(&mut h, child_win_id, "portal-child-one");

    // Portal pane in the active (root) window pointing at the child context.
    let portal_pane_id = 77200u64;
    {
        let win = &mut h.app.windows[0];
        let _ = win.tree.tiles.insert_pane(portal_pane_id);
        win.panes.insert(
            portal_pane_id,
            Pane::Portal(Box::new(crate::host::pane::PortalPane {
                pane_id: portal_pane_id,
                target_context_id: child_ctx_id,
                context_state: None,
                hidden: false,
            })),
        );
    }

    let portal_state = |h: &HostHarness| {
        let win = &h.app.windows[0];
        match win.panes.get(&portal_pane_id) {
            Some(Pane::Portal(p)) => p.context_state.clone(),
            other => panic!("expected Portal pane, got {:?}", other.map(|p| p.id())),
        }
    };

    h.run_frames(1);
    let state = portal_state(&h).expect("context_state must be computed on first frame");
    assert_eq!(state.pane_count, 1, "child context starts with one pane");

    // Mutate the child context — add a second pane — and verify the portal
    // preview reflects it on the very next frame (no stale cache).
    add_app_pane_to_window(&mut h, child_win_id, "portal-child-two");
    h.run_frames(1);
    let state = portal_state(&h).expect("context_state must persist");
    assert_eq!(
        state.pane_count, 2,
        "portal preview must reflect a pane added to the child context on the next frame"
    );
}

/// The per-pane notification badge count must update on the next frame after
/// a notification arrives, and clear after it is removed.

#[test]
fn set_pip_status_drives_activity_dot_and_overrides_agent() {
    use crate::app_protocol::{AgentState, AppRequest, PaneAgentState, PipStatus};

    let mut h = HostHarness::new();
    let pane = h.add_test_pane();
    h.run_frames(1);

    // Fresh app pane: no pip reported yet.
    assert_eq!(
        h.app.windows[0].panes.get(&pane).unwrap().pip_status(),
        None
    );

    // App reports red. pane_id is 0 on the wire; the host stamps the real pane.
    h.inject_ipc(AppRequest::SetPipStatus {
        pane_id: pane,
        status: PipStatus::Red,
    });
    h.run_frames(3);
    {
        let p = h.app.windows[0].panes.get(&pane).unwrap();
        assert_eq!(
            p.pip_status(),
            Some(PipStatus::Red),
            "host must stamp the pip on the sending pane (wire pane_id was 0)"
        );
        assert_eq!(
            p.effective_activity(),
            Some(&AgentState::Blocked),
            "red pip -> Blocked (red dot)"
        );
    }

    // Pip status wins over hook agent state: set a Working agent, then report green.
    h.app.windows[0]
        .panes
        .get_mut(&pane)
        .unwrap()
        .set_agent(Some(PaneAgentState {
            pane_id: pane,
            state: AgentState::Working,
            agent: "test".to_string(),
            detail: None,
            session_id: None,
        }));
    h.inject_ipc(AppRequest::SetPipStatus {
        pane_id: pane,
        status: PipStatus::Green,
    });
    h.run_frames(3);
    let p = h.app.windows[0].panes.get(&pane).unwrap();
    assert_eq!(
        p.effective_activity(),
        Some(&AgentState::Idle),
        "green pip -> Idle (green dot), overriding the Working agent state"
    );
}

/// Stint 0398: end-to-end counterpart of stint 0397's `canvas_transform`
/// unit test (`src/host/wasm_render.rs::canvas_click_inverts_fit_transform_to_canvas_space`).
/// That test drives a raw `UiTree` through the renderer directly; this test
/// drives a REAL process-app pane through the production `AppRequest::ClickPane`
/// dispatch — the exact path `plexi pane click` uses — and confirms the app on
/// the other side of the IPC bridge receives the correctly inverted
/// canvas-space coordinate for a `fit="contain"` (scaled + letterboxed) canvas.
///
/// `apps/dev/canvas-click-probe` renders a bare `Canvas` with no chrome, so
/// its widget rect equals the pane's own rect — letting the test predict the
/// canvas-space landing point from the pane rect alone, by hand, with the
/// same `canvas_transform` formula (declared 360x440, fit=contain) stint
/// 0397's unit test used.
#[test]
fn click_pane_delivers_canvas_space_coordinate_through_fit_contain_transform() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut h = HostHarness::new();

    let app_dir =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("apps/dev/canvas-click-probe");
    h.app
        .launch_app_by_path_with_layout(&app_dir.to_string_lossy(), None, None, &[])
        .expect("launch canvas-click-probe");
    let pane_id = *h
        .state()
        .open_panes
        .last()
        .expect("a pane appears after launching canvas-click-probe");

    // Real subprocess: poll for its first committed render before doing
    // anything layout-dependent (pane rect resolution, clicking).
    let start = std::time::Instant::now();
    loop {
        h.run_frames(1);
        let rendered = h.app.windows[h.app.active_window]
            .panes
            .get(&pane_id)
            .and_then(Pane::as_app)
            .is_some_and(|pane| {
                matches!(&pane.runtime, AppRuntime::Python(p) if p.has_rendered_tree())
            });
        if rendered {
            break;
        }
        assert!(
            start.elapsed() < std::time::Duration::from_secs(30),
            "canvas-click-probe did not render its first frame in time"
        );
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    // A couple more idle frames so the tile tree's layout settles.
    h.run_frames(2);

    let (win_idx, tile_id) = h
        .app
        .find_pane_in_any_window(pane_id)
        .expect("pane must be findable in a window");
    let pane_rect = h.app.windows[win_idx]
        .tree
        .tiles
        .rect(tile_id)
        .expect("pane must have a known rect after rendering");

    // Mirrors `canvas_transform` (src/host/wasm_render.rs) by hand: the probe
    // declares a 360x440 canvas with fit="contain" and no chrome, so the
    // canvas widget rect equals `pane_rect` exactly. Target canvas-space
    // point (90, 40) — the same target stint 0397's unit test used.
    let declared_w = 360.0_f32;
    let declared_h = 440.0_f32;
    let sx = pane_rect.width() / declared_w;
    let sy = pane_rect.height() / declared_h;
    let scale = sx.min(sy);
    let content_w = declared_w * scale;
    let content_h = declared_h * scale;
    let local_origin_x = (pane_rect.width() - content_w) / 2.0;
    let local_origin_y = (pane_rect.height() - content_h) / 2.0;
    let target_canvas_x = 90.0_f32;
    let target_canvas_y = 40.0_f32;
    let click_x = local_origin_x + target_canvas_x * scale;
    let click_y = local_origin_y + target_canvas_y * scale;

    let response_file = temp_response(tmp.path(), "click-pane");
    h.inject_click(pane_id, click_x, click_y, "left", Some(response_file.clone()));
    h.run_frames(1);

    let response = read_json_response(&response_file);
    assert_eq!(response["ok"], true, "click dispatch failed: {response:?}");

    // The click's egui-level effect is synchronous, but delivery to the
    // Python subprocess and its reply round-trip the IPC pipe — poll for the
    // app's re-render to reflect it.
    let start = std::time::Instant::now();
    let mut observed: Option<(f64, f64)> = None;
    loop {
        h.run_frames(1);
        let state = h.app.windows[win_idx]
            .panes
            .get(&pane_id)
            .and_then(Pane::as_app)
            .map(|pane| pane.semantic_state());
        if let Some(state) = &state {
            for node in &state.nodes {
                for cmd in &node.canvas_commands {
                    if cmd.get("type").and_then(|v| v.as_str()) == Some("text") {
                        if let (Some(x), Some(y)) = (
                            cmd.get("x").and_then(|v| v.as_f64()),
                            cmd.get("y").and_then(|v| v.as_f64()),
                        ) {
                            observed = Some((x, y));
                        }
                    }
                }
            }
        }
        if observed.is_some() {
            break;
        }
        assert!(
            start.elapsed() < std::time::Duration::from_secs(30),
            "app did not report the click's canvas-space coordinate in time"
        );
        std::thread::sleep(std::time::Duration::from_millis(25));
    }

    let (got_x, got_y) = observed.expect("click coordinate observed");
    assert!(
        (got_x - target_canvas_x as f64).abs() < 0.5,
        "expected canvas-space x near {target_canvas_x}, got {got_x}"
    );
    assert!(
        (got_y - target_canvas_y as f64).abs() < 0.5,
        "expected canvas-space y near {target_canvas_y}, got {got_y}"
    );
}

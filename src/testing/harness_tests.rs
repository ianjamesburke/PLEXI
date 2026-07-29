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

#[test]
fn pane_status_request_routes_through_host_and_returns_composite_evidence() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut h = HostHarness::new();
    let pane_id = h.add_focused_terminal();
    h.app.windows[0]
        .panes
        .get_mut(&pane_id)
        .expect("terminal pane")
        .set_agent(Some(crate::app_protocol::PaneAgentState {
            pane_id,
            state: crate::app_protocol::AgentState::Working,
            agent: "codex".to_string(),
            detail: Some("Bash(cargo test)".to_string()),
            session_id: None,
        }));

    let response_file = temp_response(tmp.path(), "pane-status");
    h.inject_ipc(AppRequest::PaneStatus {
        pane_id,
        response_file: response_file.clone(),
    });
    h.run_frames(1);

    let response = read_json_response(&response_file);
    assert_eq!(response["verdict"], "working");
    assert_eq!(response["confidence"], "high");
    assert_eq!(response["agent_state"], "working");
    assert_eq!(response["detail"], "Bash(cargo test)");
    assert!(response.get("status_bar").is_some());
    assert!(response.get("status_bar_truncated").is_some());
    assert!(response.get("last_buffer_line").is_some());
}

#[test]
fn pane_status_request_fails_loudly_for_unknown_pane() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut h = HostHarness::new();
    let response_file = temp_response(tmp.path(), "pane-status-missing");
    h.inject_ipc(AppRequest::PaneStatus {
        pane_id: u64::MAX,
        response_file: response_file.clone(),
    });
    h.run_frames(1);
    assert_eq!(
        read_json_response(&response_file)["error"],
        format!("pane {} not found", u64::MAX)
    );
}

#[test]
fn pane_state_ipc_returns_non_empty_semantic_tree_for_live_builtin_app() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut h = HostHarness::new();
    let pane_id = h.add_test_pane();
    h.run_frames(2);

    let response_file = temp_response(tmp.path(), "pane-state-builtin");
    h.inject_ipc(AppRequest::GetPaneState {
        pane_id,
        response_file: response_file.clone(),
    });
    h.run_hidden_frames(1);

    let response = read_json_response(&response_file);
    assert_eq!(response["type"], "app");
    assert!(
        response["semantic"]["roots"]
            .as_array()
            .is_some_and(|roots| !roots.is_empty()),
        "pane state must expose at least one semantic root; response={response}"
    );
    assert!(
        response["semantic"]["nodes"]
            .as_array()
            .is_some_and(|nodes| !nodes.is_empty()),
        "pane state must expose at least one semantic node; response={response}"
    );
}

#[test]
fn notes_drop_uses_production_dispatch_and_exposes_semantic_rejection() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let note = tmp.path().join("note.md");
    std::fs::write(&note, "hello").expect("seed note");
    let image = tmp.path().join("image.png");
    std::fs::write(&image, b"png fixture").expect("seed image");
    let mut h = HostHarness::new();
    h.app.open_builtin_app_pane(
        Box::new(crate::app::text_editor_app::TextEditorApp::new_for_test_note(note.clone())),
        crate::app::permissions::AppPermissions::builtin(),
        tmp.path().to_path_buf(),
        None,
        Some("split_h"),
        None,
    );
    let pane_id = h.state().open_panes[0];
    h.run_frames(2);
    let response = temp_response(tmp.path(), "notes-drop");
    h.inject_ipc(AppRequest::DropFile {
        pane_id,
        path_or_url: image.to_string_lossy().into_owned(),
        response_file: response.clone(),
    });
    h.run_frames(2);

    let response = read_json_response(&response);
    assert!(response.get("ok").is_none());
    // 0478: drops are validated by decodable content — fake PNG bytes reject.
    assert!(response["error"]
        .as_str()
        .is_some_and(|error| error.contains("not a decodable image")));
    let app = h.app.windows[0]
        .panes
        .get(&pane_id)
        .and_then(Pane::as_app)
        .expect("notes pane");
    let state = app.runtime.semantic_details().expect("notes semantics");
    assert_eq!(state["kind"], "notes_editor");
    assert_eq!(state["source_text"], "hello");
    assert_eq!(state["last_drop_result"]["result"], "rejected");
    assert!(!tmp.path().join("assets").exists());
}

/// `plexi pane click` on a builtin (egui-widget) pane must deliver a genuine
/// pointer press/release into the production pass — nothing on the builtin
/// render path consumes `PendingPaneClick`, so queuing one there silently
/// drops the click (the pre-0474-fix failure mode: click never moved the
/// Notes caret).
#[test]
fn click_pane_moves_notes_caret_through_real_pointer_events() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let note = tmp.path().join("note.md");
    let body: String = (0..40)
        .map(|i| format!("line number {i}\n"))
        .collect();
    std::fs::write(&note, &body).expect("seed note");
    let mut h = HostHarness::new();
    h.app.open_builtin_app_pane(
        Box::new(crate::app::text_editor_app::TextEditorApp::new_for_test_note(note)),
        crate::app::permissions::AppPermissions::builtin(),
        tmp.path().to_path_buf(),
        None,
        Some("split_h"),
        None,
    );
    let pane_id = h.state().open_panes[0];
    h.run_frames(2);

    let response = temp_response(tmp.path(), "notes-click");
    h.inject_click(pane_id, 60.0, 120.0, "left", Some(response.clone()));
    h.run_frames(1);
    assert!(
        h.app.pending_pane_inputs.contains_key(&pane_id),
        "builtin pane click must queue real pointer events for the pre-pass raw-input merge"
    );
    assert!(
        !h.app.pending_pane_clicks.contains_key(&pane_id),
        "builtin panes have no PendingPaneClick consumer; queuing one drops the click"
    );
    h.run_frames(2);

    assert_eq!(read_json_response(&response)["ok"], true);
    let app = h.app.windows[0]
        .panes
        .get(&pane_id)
        .and_then(Pane::as_app)
        .expect("notes pane");
    let state = app.runtime.semantic_details().expect("notes semantics");
    let caret = state["caret"].as_u64().expect("caret offset");
    assert!(
        caret > 0,
        "a click inside the document must move the caret off offset 0; state={state}"
    );
}

#[test]
fn drop_rejects_apps_without_a_production_handler_observably() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut h = HostHarness::new();
    let pane_id = h.add_test_pane();
    h.run_frames(2);
    let response = temp_response(tmp.path(), "drop-rejected");
    h.inject_ipc(AppRequest::DropFile {
        pane_id,
        path_or_url: "https://example.com/image.png".to_string(),
        response_file: response.clone(),
    });
    h.run_frames(1);
    assert!(read_json_response(&response)["error"]
        .as_str()
        .unwrap()
        .contains("does not accept"));
}

#[derive(Default)]
struct TextInputProbe {
    text: String,
    enter_handled: bool,
    enter_rendered: bool,
    consume_enter: bool,
    focus_grabbed: bool,
    /// Distinguishes the raw TextEdit ids when a test opens several probes —
    /// auto ids collide across identically-shaped pane uis.
    id_salt: u32,
}

#[derive(Default)]
struct KeyBurstProbe {
    received: Vec<String>,
}

impl crate::app::app_trait::App for KeyBurstProbe {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn type_id(&self) -> &'static str {
        "key-burst-probe"
    }

    fn display_name(&self) -> String {
        "Key Burst Probe".to_string()
    }

    fn ui(
        &mut self,
        _ui: &mut egui::Ui,
        _ctx: &crate::app::app_trait::AppRenderContext<'_>,
        _pending_click: Option<crate::host::pane::PendingPaneClick>,
    ) {
    }

    fn handle_key(
        &mut self,
        input: &crate::app::input_router::PlexiInput,
    ) -> crate::app::app_trait::KeyDisposition {
        for event in input.events() {
            if let egui::Event::Key {
                key, pressed: true, ..
            } = event
            {
                let name = format!("{key:?}").to_ascii_lowercase();
                let name = name.strip_prefix("arrow").unwrap_or(&name).to_string();
                self.received.push(name);
            }
        }
        crate::app::app_trait::KeyDisposition::Consumed
    }

    fn serialize_state(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({ "received": self.received }))
    }
}

impl crate::app::app_trait::App for TextInputProbe {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn type_id(&self) -> &'static str {
        "text-input-probe"
    }

    fn display_name(&self) -> String {
        "Text Input Probe".to_string()
    }

    // Deliberately simulates a misbehaving pane widget grabbing raw egui
    // focus — the exact pattern the reconciler must survive (stint 0429).
    // The grab fires once, not every frame: a per-frame re-grab across two
    // probe panes flips egui focus mid-pass so that neither TextEdit ever
    // draws while focused, and typed/injected text lands in neither.
    #[allow(clippy::disallowed_methods)]
    fn ui(
        &mut self,
        ui: &mut egui::Ui,
        _ctx: &crate::app::app_trait::AppRenderContext<'_>,
        _pending_click: Option<crate::host::pane::PendingPaneClick>,
    ) {
        if ui.input(|input| input.key_pressed(egui::Key::Enter)) {
            self.enter_rendered = true;
        }
        let response = ui.add(
            egui::TextEdit::singleline(&mut self.text)
                .id(egui::Id::new(("text_input_probe_edit", self.id_salt))),
        );
        if !self.focus_grabbed {
            response.request_focus();
            self.focus_grabbed = true;
        }
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

    h.app.handle_pane_ipc_request(AppRequest::SendToPane {
        pane_id,
        text: "/settings".to_string(),
        submit: false,
        response_file: Some(response_file.clone()),
    });
    assert_eq!(
        h.app.pending_pane_inputs.get(&pane_id).map_or(0, Vec::len),
        1,
        "IPC text must remain queued until the target pane's production render"
    );
    h.run_frames(1);

    assert!(
        !h.app.pending_pane_inputs.contains_key(&pane_id),
        "target render must consume its queued IPC input exactly once"
    );

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
    h.run_frames(2);

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

#[test]
fn rapid_pane_key_burst_delivers_every_press_in_order() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut h = HostHarness::new();
    h.app.open_builtin_app_pane(
        Box::<KeyBurstProbe>::default(),
        AppPermissions::builtin(),
        tmp.path().to_path_buf(),
        None,
        Some("split_h"),
        None,
    );
    let pane_id = h.state().open_panes[0];
    h.run_frames(2);

    let intended = [
        "right", "down", "left", "up", "right", "right", "down", "left",
    ];
    for (index, key) in intended.iter().enumerate() {
        h.inject_ipc(AppRequest::KeyPane {
            pane_id,
            key: (*key).to_string(),
            response_file: Some(temp_response(tmp.path(), &format!("rapid-key-{index}"))),
        });
    }
    h.run_frames(1);

    let state = h.app.windows[0]
        .panes
        .get(&pane_id)
        .and_then(Pane::as_app)
        .and_then(|pane| pane.runtime.serialize_state())
        .expect("burst probe state");
    assert_eq!(state["received"], serde_json::json!(intended));
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

/// Seed a slot with `content`, asserting the write succeeded.
fn seed_slot(
    h: &mut HostHarness,
    tmp: &std::path::Path,
    pane: crate::spatial::tiling::PaneId,
    slot_name: &str,
    content: &[u8],
    replace: bool,
) {
    let write_file = temp_response(tmp, "slot-seed");
    h.inject_ipc(AppRequest::SlotWrite {
        pane_id: pane,
        slot_name: slot_name.to_string(),
        content: content.to_vec(),
        append: false,
        replace,
        response_file: write_file.clone(),
    });
    h.run_frames(1);
    assert_eq!(
        read_json_response(&write_file)["ok"].as_bool(),
        Some(true),
        "seed write for slot {slot_name:?} must succeed"
    );
}

#[test]
fn slot_wait_returns_immediately_when_value_already_matches() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut h = HostHarness::new();
    h.app.set_context_root(tmp.path().to_path_buf(), None);
    let pane = h.add_test_pane();
    seed_slot(&mut h, tmp.path(), pane, "status", b"done: ok", false);

    let wait_file = temp_response(tmp.path(), "slot-wait-level");
    h.inject_ipc(AppRequest::SlotWait {
        pane_id: pane,
        slot_name: "status".to_string(),
        pattern: "^done".to_string(),
        timeout_secs: 60.0,
        response_file: wait_file.clone(),
    });
    h.run_frames(1);

    assert_eq!(
        std::fs::read(&wait_file).expect("wait response"),
        b"done: ok",
        "a slot already matching at call time must answer on the registering frame"
    );
    assert!(!std::path::PathBuf::from(format!("{wait_file}.err")).exists());
}

#[test]
fn slot_wait_ignores_non_matching_write_then_completes_on_match() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut h = HostHarness::new();
    h.app.set_context_root(tmp.path().to_path_buf(), None);
    let pane = h.add_test_pane();
    seed_slot(&mut h, tmp.path(), pane, "status", b"running", false);

    let wait_file = temp_response(tmp.path(), "slot-wait-edge");
    h.inject_ipc(AppRequest::SlotWait {
        pane_id: pane,
        slot_name: "status".to_string(),
        pattern: r"^done:\s*(ok|fail)$".to_string(),
        timeout_secs: 60.0,
        response_file: wait_file.clone(),
    });
    h.run_frames(1);
    assert!(
        !std::path::Path::new(&wait_file).exists(),
        "a non-matching current value must park the waiter"
    );

    seed_slot(&mut h, tmp.path(), pane, "status", b"still running", true);
    h.run_frames(1);
    assert!(
        !std::path::Path::new(&wait_file).exists(),
        "a non-matching write must not complete the waiter"
    );

    seed_slot(&mut h, tmp.path(), pane, "status", b"done: fail", true);
    h.run_frames(1);
    assert_eq!(
        std::fs::read(&wait_file).expect("wait response"),
        b"done: fail",
        "the matching write must deliver the raw slot bytes"
    );
}

#[test]
fn slot_wait_completes_while_window_hidden() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut h = HostHarness::new();
    h.app.set_context_root(tmp.path().to_path_buf(), None);
    let pane = h.add_test_pane();

    let wait_file = temp_response(tmp.path(), "slot-wait-hidden");
    h.inject_ipc(AppRequest::SlotWait {
        pane_id: pane,
        slot_name: "status".to_string(),
        pattern: "ready".to_string(),
        timeout_secs: 60.0,
        response_file: wait_file.clone(),
    });
    h.run_hidden_frames(1);

    let write_file = temp_response(tmp.path(), "slot-wait-hidden-write");
    h.inject_ipc(AppRequest::SlotWrite {
        pane_id: pane,
        slot_name: "status".to_string(),
        content: b"ready".to_vec(),
        append: false,
        replace: false,
        response_file: write_file.clone(),
    });
    h.run_hidden_frames(1);

    assert_eq!(read_json_response(&write_file)["ok"].as_bool(), Some(true));
    assert_eq!(
        std::fs::read(&wait_file).expect("wait response"),
        b"ready",
        "slot waits must be registered and completed from App::logic, which is \
         the only pass an occluded host runs"
    );
}

#[test]
fn slot_wait_times_out_with_typed_timeout_reply() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut h = HostHarness::new();
    h.app.set_context_root(tmp.path().to_path_buf(), None);
    let pane = h.add_test_pane();

    let wait_file = temp_response(tmp.path(), "slot-wait-timeout");
    h.inject_ipc(AppRequest::SlotWait {
        pane_id: pane,
        slot_name: "status".to_string(),
        pattern: "never".to_string(),
        timeout_secs: 0.05,
        response_file: wait_file.clone(),
    });
    // Hidden frames: expiry must also fire on an occluded host, which runs
    // `App::logic` and never `App::ui`.
    h.run_hidden_frames(1);

    let error_file = format!("{wait_file}.err");
    for _ in 0..40 {
        if std::path::Path::new(&error_file).exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
        h.run_hidden_frames(1);
    }

    let response = read_json_response(&error_file);
    assert_eq!(response["timeout"].as_bool(), Some(true));
    assert_eq!(response["ok"].as_bool(), Some(false));
    assert!(
        !std::path::Path::new(&wait_file).exists(),
        "a timed-out wait must leave stdout empty"
    );
}

#[test]
fn slot_wait_invalid_regex_writes_typed_error() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut h = HostHarness::new();
    h.app.set_context_root(tmp.path().to_path_buf(), None);
    let pane = h.add_test_pane();

    let wait_file = temp_response(tmp.path(), "slot-wait-bad-regex");
    h.inject_ipc(AppRequest::SlotWait {
        pane_id: pane,
        slot_name: "status".to_string(),
        pattern: "([unclosed".to_string(),
        timeout_secs: 60.0,
        response_file: wait_file.clone(),
    });
    h.run_frames(1);

    let response = read_json_response(&format!("{wait_file}.err"));
    assert_eq!(response["ok"].as_bool(), Some(false));
    assert_eq!(response["timeout"].as_bool(), None);
    assert!(
        response["error"]
            .as_str()
            .expect("error")
            .contains("invalid --until pattern"),
        "unexpected error: {response}"
    );
}

#[test]
fn slot_wait_oversized_timeout_writes_typed_error() {
    // Finite f64 values beyond Duration's range panic in from_secs_f64; the
    // host must reject them as usage errors, never trusting the client.
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut h = HostHarness::new();
    h.app.set_context_root(tmp.path().to_path_buf(), None);
    let pane = h.add_test_pane();

    let wait_file = temp_response(tmp.path(), "slot-wait-huge-timeout");
    h.inject_ipc(AppRequest::SlotWait {
        pane_id: pane,
        slot_name: "status".to_string(),
        pattern: "ready".to_string(),
        timeout_secs: 1e308,
        response_file: wait_file.clone(),
    });
    h.run_frames(1);

    let response = read_json_response(&format!("{wait_file}.err"));
    assert_eq!(response["ok"].as_bool(), Some(false));
    assert!(
        response["error"].as_str().expect("error").contains("at most"),
        "unexpected error: {response}"
    );
}

#[test]
fn slot_wait_unknown_pane_writes_typed_error() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut h = HostHarness::new();
    h.app.set_context_root(tmp.path().to_path_buf(), None);

    let wait_file = temp_response(tmp.path(), "slot-wait-no-pane");
    h.inject_ipc(AppRequest::SlotWait {
        pane_id: 999_999,
        slot_name: "status".to_string(),
        pattern: "ready".to_string(),
        timeout_secs: 60.0,
        response_file: wait_file.clone(),
    });
    h.run_frames(1);

    let response = read_json_response(&format!("{wait_file}.err"));
    assert_eq!(response["ok"].as_bool(), Some(false));
    assert!(
        response["error"]
            .as_str()
            .expect("error")
            .contains("pane 999999 not found"),
        "unexpected error: {response}"
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
/// Raw focus calls are banned in production (stint 0429); this helper exists
/// precisely to simulate a rogue steal the reconciler must undo.
#[allow(clippy::disallowed_methods)]
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
            .is_some_and(
                |pane| matches!(&pane.runtime, AppRuntime::Python(p) if p.has_rendered_tree()),
            );
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
    h.inject_click(
        pane_id,
        click_x,
        click_y,
        "left",
        Some(response_file.clone()),
    );
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

/// Stint 0510: drag counterpart of stint 0398's click e2e above. Drives a
/// REAL process-app pane (`apps/dev/canvas-click-probe`) through the
/// production `AppRequest::DragPane` dispatch — the exact path
/// `plexi pane drag` and `HostHarness::inject_drag` use. The schedule is
/// delivered one frame at a time (press → moves → release), each sample
/// resolved against that frame's live canvas rect, and the guest's
/// re-rendered view must report the full trajectory: press and release in
/// canvas space plus every intermediate move.
#[test]
fn drag_pane_delivers_press_moves_release_through_canvas_transform() {
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

    let start = std::time::Instant::now();
    loop {
        h.run_frames(1);
        let rendered = h.app.windows[h.app.active_window]
            .panes
            .get(&pane_id)
            .and_then(Pane::as_app)
            .is_some_and(
                |pane| matches!(&pane.runtime, AppRuntime::Python(p) if p.has_rendered_tree()),
            );
        if rendered {
            break;
        }
        assert!(
            start.elapsed() < std::time::Duration::from_secs(30),
            "canvas-click-probe did not render its first frame in time"
        );
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
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

    // Same hand-mirrored `fit="contain"` transform as the click e2e: the
    // probe's bare-canvas tree makes the widget rect equal the pane rect.
    let declared_w = 360.0_f32;
    let declared_h = 440.0_f32;
    let sx = pane_rect.width() / declared_w;
    let sy = pane_rect.height() / declared_h;
    let scale = sx.min(sy);
    let content_w = declared_w * scale;
    let content_h = declared_h * scale;
    let local_origin_x = (pane_rect.width() - content_w) / 2.0;
    let local_origin_y = (pane_rect.height() - content_h) / 2.0;
    let to_pane = |cx: f32, cy: f32| {
        (
            local_origin_x + cx * scale,
            local_origin_y + cy * scale,
        )
    };
    let (from_canvas_x, from_canvas_y) = (90.0_f32, 40.0_f32);
    let (to_canvas_x, to_canvas_y) = (270.0_f32, 220.0_f32);
    let steps = 6u32;

    let response_file = temp_response(tmp.path(), "drag-pane");
    h.inject_drag(
        pane_id,
        to_pane(from_canvas_x, from_canvas_y),
        to_pane(to_canvas_x, to_canvas_y),
        steps,
        "left",
        Some(response_file.clone()),
    );
    h.run_frames(1);
    let response = read_json_response(&response_file);
    assert_eq!(response["ok"], true, "drag dispatch failed: {response:?}");
    assert_eq!(response["frames"], steps + 2, "unexpected schedule length");

    // Deliver the whole schedule, then poll the guest's re-render for the
    // completed drag record painted back as the `drag:` canvas text.
    let start = std::time::Instant::now();
    let mut drag_text: Option<String> = None;
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
                    if let Some(text) = cmd.get("text").and_then(|v| v.as_str()) {
                        if text.starts_with("drag:") {
                            drag_text = Some(text.to_string());
                        }
                    }
                }
            }
        }
        if drag_text.is_some() {
            break;
        }
        assert!(
            start.elapsed() < std::time::Duration::from_secs(30),
            "app did not report a completed drag in time"
        );
        std::thread::sleep(std::time::Duration::from_millis(25));
    }

    let drag_text = drag_text.expect("drag record observed");
    assert!(
        drag_text.contains(&format!("press:{from_canvas_x:.2},{from_canvas_y:.2}")),
        "press endpoint missing/incorrect in {drag_text:?}"
    );
    assert!(
        drag_text.contains(&format!("release:{to_canvas_x:.2},{to_canvas_y:.2}")),
        "release endpoint missing/incorrect in {drag_text:?}"
    );
    assert!(
        drag_text.contains(&format!("moves:{steps}")),
        "expected every intermediate move to reach the guest in {drag_text:?}"
    );
}

/// Stint 0510: node-addressed drag endpoints resolve from the pane's cached
/// semantic bounds at dispatch and must fail loudly — named error, nothing
/// queued — when the node is absent or records no bounds. Process/WASM
/// semantic trees record no per-node bounds today, so both failure shapes
/// are exercised against the real canvas-click-probe pane.
#[test]
fn drag_pane_node_endpoints_fail_loudly_without_bounds() {
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

    let start = std::time::Instant::now();
    loop {
        h.run_frames(1);
        let rendered = h.app.windows[h.app.active_window]
            .panes
            .get(&pane_id)
            .and_then(Pane::as_app)
            .is_some_and(
                |pane| matches!(&pane.runtime, AppRuntime::Python(p) if p.has_rendered_tree()),
            );
        if rendered {
            break;
        }
        assert!(
            start.elapsed() < std::time::Duration::from_secs(30),
            "canvas-click-probe did not render its first frame in time"
        );
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    h.run_frames(2);

    // Node exists (the canvas root, arena id "0") but has no recorded bounds.
    let response_file = temp_response(tmp.path(), "drag-node-no-bounds");
    h.inject_node_drag(pane_id, "0", "0", 4, "left", Some(response_file.clone()));
    h.run_frames(1);
    let response = read_json_response(&response_file);
    let error = response["error"].as_str().unwrap_or_default();
    assert!(
        error.contains("no rendered bounds"),
        "expected a named no-bounds error, got {response:?}"
    );

    // Node absent from the current tree entirely.
    let response_file = temp_response(tmp.path(), "drag-node-missing");
    h.inject_node_drag(pane_id, "9999", "9999", 4, "left", Some(response_file.clone()));
    h.run_frames(1);
    let response = read_json_response(&response_file);
    let error = response["error"].as_str().unwrap_or_default();
    assert!(
        error.contains("not found"),
        "expected a named missing-node error, got {response:?}"
    );
}

/// Stint 0511: an emitted app event must land in the recorded event bus and
/// be assertable through `HostHarness::wait_for_app_event`. Drives the REAL
/// `apps/dev/event-probe` process app: `KeyPane('e')` makes the guest emit
/// `probe.tick`, which flows guest → bridge → `AppRequest::EmitEvent` →
/// `AppTimeline::record_event` — the exact path the scene `expect`
/// `event_stream` predicate reads.
#[test]
fn emitted_app_event_is_recorded_and_awaitable() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut h = HostHarness::new();

    let app_dir =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("apps/dev/event-probe");
    h.app
        .launch_app_by_path_with_layout(&app_dir.to_string_lossy(), None, None, &[])
        .expect("launch event-probe");
    let pane_id = *h
        .state()
        .open_panes
        .last()
        .expect("a pane appears after launching event-probe");

    let start = std::time::Instant::now();
    loop {
        h.run_frames(1);
        let rendered = h.app.windows[h.app.active_window]
            .panes
            .get(&pane_id)
            .and_then(Pane::as_app)
            .is_some_and(
                |pane| matches!(&pane.runtime, AppRuntime::Python(p) if p.has_rendered_tree()),
            );
        if rendered {
            break;
        }
        assert!(
            start.elapsed() < std::time::Duration::from_secs(30),
            "event-probe did not render its first frame in time"
        );
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    h.run_frames(2);

    let response_file = temp_response(tmp.path(), "key-pane");
    h.inject_ipc(crate::app_protocol::AppRequest::KeyPane {
        pane_id,
        key: "e".to_string(),
        response_file: Some(response_file.clone()),
    });
    h.run_frames(2);
    let response = read_json_response(&response_file);
    assert_eq!(response["ok"], true, "key dispatch failed: {response:?}");

    let record = h
        .wait_for_app_event(
            "probe.tick",
            Some("\"count\":1"),
            std::time::Duration::from_secs(30),
        )
        .expect("probe.tick event recorded on the timeline");
    assert_eq!(record.event, "probe.tick");
    assert_eq!(record.resource_id, "probe-session");
    assert!(
        record.summary.contains("Probe tick 1"),
        "unexpected summary: {:?}",
        record.summary
    );
}

/// Stint 0414: node-targeted counterpart of stint 0398's
/// `click_pane_delivers_canvas_space_coordinate_through_fit_contain_transform`.
/// Drives a REAL process-app pane (`apps/dev/node-click-probe`, a single
/// `Button` with no other input) through the production
/// `AppRequest::ClickPaneNode` dispatch — the exact path
/// `plexi pane click <pane_id> --node <node_id>` uses — resolving the
/// button's `node_id` from the pane's own semantic tree (`plexi pane state`)
/// exactly as a tester would, never a hardcoded arena id. Proves the
/// end-to-end contract the launch gate (stint 0413) needs: a widget can be
/// activated by node_id and the guest's re-rendered view reflects it.
#[test]
fn click_pane_node_activates_button_and_mutates_guest_view() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut h = HostHarness::new();

    let app_dir =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("apps/dev/node-click-probe");
    h.app
        .launch_app_by_path_with_layout(&app_dir.to_string_lossy(), None, None, &[])
        .expect("launch node-click-probe");
    let pane_id = *h
        .state()
        .open_panes
        .last()
        .expect("a pane appears after launching node-click-probe");

    // Real subprocess: poll for its first committed render before reading
    // the semantic tree.
    let start = std::time::Instant::now();
    loop {
        h.run_frames(1);
        let rendered = h.app.windows[h.app.active_window]
            .panes
            .get(&pane_id)
            .and_then(Pane::as_app)
            .is_some_and(
                |pane| matches!(&pane.runtime, AppRuntime::Python(p) if p.has_rendered_tree()),
            );
        if rendered {
            break;
        }
        assert!(
            start.elapsed() < std::time::Duration::from_secs(30),
            "node-click-probe did not render its first frame in time"
        );
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    h.run_frames(2);

    let semantic_state = h.app.windows[h.app.active_window]
        .panes
        .get(&pane_id)
        .and_then(Pane::as_app)
        .map(|pane| pane.semantic_state())
        .expect("app pane has a semantic tree");

    let button_id = semantic_state
        .nodes
        .iter()
        .find(|n| n.role == "button" && n.label.as_deref() == Some("Increment"))
        .map(|n| n.id.clone())
        .expect("semantic tree exposes the Increment button by role/label");
    let text_id = semantic_state
        .nodes
        .iter()
        .find(|n| n.role == "text")
        .map(|n| n.id.clone())
        .expect("semantic tree exposes the count Text node");
    assert_eq!(
        semantic_state
            .nodes
            .iter()
            .find(|n| n.id == text_id)
            .and_then(|n| n.label.as_deref()),
        Some("0"),
        "count starts at 0"
    );

    // Fail-loud path: a node id absent from the current tree is rejected,
    // never silently no-op'd.
    let missing_response = temp_response(tmp.path(), "click-node-missing");
    h.inject_node_click(pane_id, "9999", "left", Some(missing_response.clone()));
    h.run_frames(1);
    let missing = read_json_response(&missing_response);
    assert!(
        missing.get("error").is_some(),
        "clicking an absent node_id must return a named error, not ok:true: {missing:?}"
    );

    // Fail-loud path: a non-interactive node (the Text node) is rejected too.
    let non_interactive_response = temp_response(tmp.path(), "click-node-non-interactive");
    h.inject_node_click(
        pane_id,
        &text_id,
        "left",
        Some(non_interactive_response.clone()),
    );
    h.run_frames(1);
    let non_interactive = read_json_response(&non_interactive_response);
    assert!(
        non_interactive.get("error").is_some(),
        "clicking a non-interactive node_id must return a named error: {non_interactive:?}"
    );

    // Happy path: click the button by node_id and observe the guest's
    // re-rendered count. The click's egui-level effect is synchronous, but
    // delivery to the Python subprocess and its reply round-trip the IPC
    // pipe — poll for the app's re-render to reflect it.
    let click_response = temp_response(tmp.path(), "click-node-increment");
    h.inject_node_click(pane_id, &button_id, "left", Some(click_response.clone()));
    h.run_frames(1);
    let response = read_json_response(&click_response);
    assert_eq!(
        response["ok"], true,
        "node click dispatch failed: {response:?}"
    );

    let start = std::time::Instant::now();
    let mut observed_count: Option<String> = None;
    loop {
        h.run_frames(1);
        let state = h.app.windows[h.app.active_window]
            .panes
            .get(&pane_id)
            .and_then(Pane::as_app)
            .map(|pane| pane.semantic_state());
        if let Some(state) = &state {
            observed_count = state
                .nodes
                .iter()
                .find(|n| n.role == "text")
                .and_then(|n| n.label.clone());
        }
        if observed_count.as_deref() == Some("1") {
            break;
        }
        assert!(
            start.elapsed() < std::time::Duration::from_secs(30),
            "app did not reflect the node-targeted click's mutation in time (last seen: {observed_count:?})"
        );
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
}

/// Stint 0469: node-targeted click passthrough into a *builtin* app, exercising
/// the FULL two-step workflow entirely through synthetic node clicks — the same
/// path `plexi pane click <id> --node <id>` drives — with no real pointer input
/// anywhere. Step 1 clicks the "View" toolbar button (opening its menu, which is
/// what the tester's PR #2448 reject proved was a silent no-op: `ui.menu_button`
/// only toggles open from a real click resolved inside `Context::begin_pass`, so
/// the menu never opened and the checkbox never entered the tree). Step 2 clicks
/// the now-exposed "Show hidden files" checkbox. Proves: `pending_click` reaches
/// `AppRuntime::Builtin`; a synthetic click force-opens the menu popup so its
/// items enter the semantic tree; and `resolve_interactive_node`/
/// `INTERACTIVE_ROLES` accept a real `checkbox` node whose id is a full `u64`
/// egui hash (why `PaneClickTarget::Node` was widened from `u32`). Every node id
/// is read off the live semantic tree, never fabricated.
#[test]
fn click_pane_node_toggles_file_explorer_hidden_files_checkbox() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut h = HostHarness::new();
    h.app.open_builtin_app_pane(
        Box::new(crate::file_browser::FileBrowserApp::new(
            tmp.path().to_path_buf(),
        )),
        AppPermissions::builtin(),
        tmp.path().to_path_buf(),
        None,
        Some("split_h"),
        None,
    );
    let pane_id = h.state().open_panes[0];
    h.run_frames(2);

    let read_show_hidden = |h: &mut HostHarness| -> bool {
        let win = h.app.active_window;
        let pane = h.app.windows[win]
            .panes
            .get_mut(&pane_id)
            .expect("file explorer pane exists");
        let AppRuntime::Builtin(app) = &mut pane.as_app_mut().expect("app pane").runtime else {
            panic!("pane {pane_id} is not a builtin app");
        };
        app.as_any_mut()
            .downcast_mut::<crate::file_browser::FileBrowserApp>()
            .expect("pane is a FileBrowserApp")
            .show_hidden()
    };
    let find_node = |h: &mut HostHarness, role: &str, label: &str| -> Option<String> {
        h.app.windows[h.app.active_window]
            .panes
            .get(&pane_id)
            .and_then(Pane::as_app)
            .map(|pane| pane.semantic_state())
            .expect("app pane has a semantic tree")
            .nodes
            .iter()
            .find(|n| n.role == role && n.label.as_deref() == Some(label))
            .map(|n| n.id.clone())
    };

    assert!(!read_show_hidden(&mut h), "show_hidden must start disabled");
    assert!(
        find_node(&mut h, "checkbox", "Show hidden files").is_none(),
        "checkbox must NOT be in the tree before the View menu is opened"
    );

    // Step 1 (SYNTHETIC): click the "View \u{2304}" toolbar button by node id.
    // Before the fix this was a silent no-op — the menu stayed closed and the
    // checkbox never appeared, so `find_node` below would return None.
    let view_id = find_node(&mut h, "button", "View \u{2304}")
        .expect("semantic tree exposes the View menu button by role/label");
    let view_response = temp_response(tmp.path(), "click-node-view-menu");
    h.inject_node_click(pane_id, &view_id, "left", Some(view_response.clone()));
    h.run_frames(2);
    let view_result = read_json_response(&view_response);
    assert_eq!(
        view_result["ok"], true,
        "View button node click dispatch failed: {view_result:?}"
    );

    // The synthetic click must have opened the menu, so the checkbox now
    // renders inside it and enters the semantic tree.
    let checkbox_id = find_node(&mut h, "checkbox", "Show hidden files").expect(
        "synthetic click on the View button must open the menu and expose the \
         Show hidden files checkbox in the semantic tree",
    );

    // Step 2 (SYNTHETIC): click the now-exposed checkbox by node id.
    let click_response = temp_response(tmp.path(), "click-node-hidden-files");
    h.inject_node_click(pane_id, &checkbox_id, "left", Some(click_response.clone()));
    h.run_frames(1);
    let response = read_json_response(&click_response);
    assert_eq!(
        response["ok"], true,
        "checkbox node click dispatch failed: {response:?}"
    );

    assert!(
        read_show_hidden(&mut h),
        "synthetic node click on the checkbox must toggle show_hidden on"
    );
}

/// Stint 0426: `plexi pane key` / `AppRequest::KeyPane` targeting a real
/// WASM Python pane (`apps/dev/key-event-probe`, a keyboard-only counter —
/// no click/pointer input). Drives a real subprocess through the exact
/// dispatch `KeyPane` uses (`drive_native_pane_key` → `AppRuntime::handle_key`
/// → `LivePythonPane::handle_key`) and asserts the guest's re-rendered state
/// actually changed — not just that the command returned `{"ok": true}`.
/// Written first per TESTING.md/TDD to confirm this path was broken before
/// any fix, mirroring `click_pane_node_activates_button_and_mutates_guest_view`.
#[test]
fn key_pane_delivers_key_event_and_mutates_guest_view() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut h = HostHarness::new();

    let app_dir =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("apps/dev/key-event-probe");
    h.app
        .launch_app_by_path_with_layout(&app_dir.to_string_lossy(), None, None, &[])
        .expect("launch key-event-probe");
    let pane_id = *h
        .state()
        .open_panes
        .last()
        .expect("a pane appears after launching key-event-probe");

    // Real subprocess: poll for its first committed render before driving input.
    let start = std::time::Instant::now();
    loop {
        h.run_frames(1);
        let rendered = h.app.windows[h.app.active_window]
            .panes
            .get(&pane_id)
            .and_then(Pane::as_app)
            .is_some_and(
                |pane| matches!(&pane.runtime, AppRuntime::Python(p) if p.has_rendered_tree()),
            );
        if rendered {
            break;
        }
        assert!(
            start.elapsed() < std::time::Duration::from_secs(30),
            "key-event-probe did not render its first frame in time"
        );
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    h.run_frames(2);

    let initial_count = h.app.windows[h.app.active_window]
        .panes
        .get(&pane_id)
        .and_then(Pane::as_app)
        .map(|pane| pane.semantic_state())
        .and_then(|state| {
            state
                .nodes
                .iter()
                .find(|n| n.role == "text")
                .and_then(|n| n.label.clone())
        });
    assert_eq!(initial_count.as_deref(), Some("0"), "count starts at 0");

    let key_response = temp_response(tmp.path(), "key-pane-plus");
    h.inject_ipc(AppRequest::KeyPane {
        pane_id,
        key: "plus".to_string(),
        response_file: Some(key_response.clone()),
    });
    h.run_frames(1);
    let response = read_json_response(&key_response);
    assert_eq!(response["ok"], true, "key dispatch failed: {response:?}");

    let start = std::time::Instant::now();
    let mut observed_count: Option<String> = None;
    loop {
        h.run_frames(1);
        let state = h.app.windows[h.app.active_window]
            .panes
            .get(&pane_id)
            .and_then(Pane::as_app)
            .map(|pane| pane.semantic_state());
        if let Some(state) = &state {
            observed_count = state
                .nodes
                .iter()
                .find(|n| n.role == "text")
                .and_then(|n| n.label.clone());
        }
        if observed_count.as_deref() == Some("1") {
            break;
        }
        assert!(
            start.elapsed() < std::time::Duration::from_secs(30),
            "app did not reflect the key event's mutation in time (last seen: {observed_count:?})"
        );
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
}

/// Stint 0430: bare Escape on a focused WASM/Python pane must close it. The
/// Escape→CloseApp binding (keys.rs, `BindingContext::AppActive`) fires in
/// `poll_actions` only when the focused app's `handle_key` did NOT consume the
/// key. `LivePythonPane::handle_key` used to report `Consumed` for bare Escape
/// (`python_key_events` forwarded it), which claimed Escape out of the frame's
/// input buffer before `poll_actions` ran — so the pane never closed. Drive a
/// real subprocess pane and a genuine Escape `RawInput` through a live frame
/// and assert the pane closes.
#[test]
fn bare_escape_closes_focused_python_wasm_pane() {
    let mut h = HostHarness::new();

    let app_dir =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("apps/dev/key-event-probe");
    h.app
        .launch_app_by_path_with_layout(&app_dir.to_string_lossy(), None, None, &[])
        .expect("launch key-event-probe");
    let pane_id = *h
        .state()
        .open_panes
        .last()
        .expect("a pane appears after launching key-event-probe");

    // Real subprocess: poll for its first committed render before driving input.
    let start = std::time::Instant::now();
    loop {
        h.run_frames(1);
        let rendered = h.app.windows[h.app.active_window]
            .panes
            .get(&pane_id)
            .and_then(Pane::as_app)
            .is_some_and(
                |pane| matches!(&pane.runtime, AppRuntime::Python(p) if p.has_rendered_tree()),
            );
        if rendered {
            break;
        }
        assert!(
            start.elapsed() < std::time::Duration::from_secs(30),
            "key-event-probe did not render its first frame in time"
        );
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    h.focus_pane(pane_id);
    h.run_frames(1);
    assert!(
        h.state().open_panes.contains(&pane_id),
        "pane must be open and focused before the Escape press"
    );

    // Drive a genuine bare Escape through a real frame's RawInput. The focused
    // Python pane's handle_key returns Passthrough for it, so poll_actions fires
    // CloseApp within this same frame.
    h.frame(egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1280.0, 800.0),
        )),
        events: vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
        ..Default::default()
    });

    assert!(
        !h.state().open_panes.contains(&pane_id),
        "bare Escape on a focused Python-WASM pane must close it (Escape→CloseApp)"
    );
}

/// Stint 0426 root cause: `PlexiApp::pending_pane_clicks` is removed from its
/// map by the render dispatcher (`tiling.rs`/`render.rs`) *before* calling
/// `AppRuntime::ui`, so `LivePythonPane::ui` is a queued click's only chance
/// at delivery. It had three early-return branches (fatal error, not
/// initialized, not `ready`) that discarded an already-dequeued click
/// without using it and without any error — `plexi pane click --node`
/// reported `{"ok": true}` while nothing ever happened. A hot-reload
/// relaunch (`AppRuntime::Python::relaunch`, triggered by file-watch or
/// `reload_app_pane`) is the most realistic way to land a queued click on
/// exactly that not-ready window: the click resolves and queues against the
/// pane's current tree, then the pane briefly cycles through
/// not-initialized/not-ready before its relaunched subprocess re-renders.
///
/// This test forces that exact race deterministically (no IPC timing
/// dependent sleep-and-hope): queue a node click against the live tree,
/// force a relaunch before any frame drains the queue, then confirm the
/// click still lands once the relaunched pane is ready again. Before the
/// fix in `LivePythonPane::ui`, this hung until timeout because the click
/// was dropped the instant it landed on the not-ready frame.
#[test]
fn queued_node_click_survives_a_hot_reload_relaunch_race() {
    let mut h = HostHarness::new();

    let app_dir =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("apps/dev/node-click-probe");
    h.app
        .launch_app_by_path_with_layout(&app_dir.to_string_lossy(), None, None, &[])
        .expect("launch node-click-probe");
    let pane_id = *h
        .state()
        .open_panes
        .last()
        .expect("a pane appears after launching node-click-probe");

    let start = std::time::Instant::now();
    loop {
        h.run_frames(1);
        let rendered = h.app.windows[h.app.active_window]
            .panes
            .get(&pane_id)
            .and_then(Pane::as_app)
            .is_some_and(
                |pane| matches!(&pane.runtime, AppRuntime::Python(p) if p.has_rendered_tree()),
            );
        if rendered {
            break;
        }
        assert!(
            start.elapsed() < std::time::Duration::from_secs(30),
            "node-click-probe did not render its first frame in time"
        );
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    h.run_frames(2);

    let semantic_state = h.app.windows[h.app.active_window]
        .panes
        .get(&pane_id)
        .and_then(Pane::as_app)
        .map(|pane| pane.semantic_state())
        .expect("app pane has a semantic tree");
    let button_arena_id = semantic_state
        .nodes
        .iter()
        .find(|n| n.role == "button" && n.label.as_deref() == Some("Increment"))
        .map(|n| n.id.clone())
        .and_then(|id| semantic_state.resolve_interactive_node(&id).ok())
        .expect("Increment button resolves to an arena id");

    // Queue the click directly on the pending-click map — bypassing the IPC
    // queue (`inject_node_click`) — so the relaunch below is guaranteed to
    // land before any frame drains it. Going through IPC would risk the
    // same `run_frames` call both draining the queue AND rendering the
    // still-live (pre-reload) pane, delivering the click before the race
    // window this test exists to exercise.
    h.app.pending_pane_clicks.insert(
        pane_id,
        crate::host::pane::PendingPaneClick {
            target: crate::host::pane::PaneClickTarget::Node(button_arena_id),
            button: "left",
            phase: crate::host::pane::PointerPhase::Click,
        },
    );

    // Force the exact race: relaunch before any frame drains the queued
    // click, landing it on a not-initialized/not-ready `ui()` call.
    assert!(
        h.app.reload_app_pane(pane_id, "test: relaunch race"),
        "reload_app_pane must find and relaunch the Python pane"
    );

    // Wait for the relaunched subprocess to become ready and render again,
    // then confirm the carried click still activated the button.
    let start = std::time::Instant::now();
    let mut observed_count: Option<String> = None;
    loop {
        h.run_frames(1);
        let state = h.app.windows[h.app.active_window]
            .panes
            .get(&pane_id)
            .and_then(Pane::as_app)
            .map(|pane| pane.semantic_state());
        if let Some(state) = &state {
            observed_count = state
                .nodes
                .iter()
                .find(|n| n.role == "text")
                .and_then(|n| n.label.clone());
        }
        if observed_count.as_deref() == Some("1") {
            break;
        }
        assert!(
            start.elapsed() < std::time::Duration::from_secs(30),
            "click queued before a hot-reload relaunch must still land once the \
             relaunched pane is ready again (last seen count: {observed_count:?})"
        );
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
}

// -- Single-authority input ownership (stint 0429) --------------------------

/// Run one frame with the given input events.
fn frame_with_events(h: &mut HostHarness, events: Vec<egui::Event>) {
    h.frame(egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1280.0, 800.0),
        )),
        events,
        ..Default::default()
    });
}

/// Typing while pane A is active must leave pane B's assistant composer
/// unchanged. Before stint 0429 the composer claimed egui focus when active
/// and nothing surrendered it on deactivation, so the stale focus kept
/// consuming Text events for an inactive pane.
#[test]
fn typing_while_pane_a_active_leaves_inactive_assistant_composer_unchanged() {
    let mut h = HostHarness::new();
    let pane_a = h.add_test_pane();
    let pane_b = h.add_assistant_pane();

    // Activate the assistant so its composer takes egui focus.
    h.focus_pane(pane_b);
    h.run_frames(2);

    // Switch host focus to pane A and let the frame settle — the reconciler
    // must project the ownership change onto egui focus here.
    h.focus_pane(pane_a);
    h.run_frames(1);

    frame_with_events(&mut h, vec![egui::Event::Text("x".to_string())]);

    assert_eq!(
        h.assistant_mut(pane_b).model.composer,
        "",
        "text typed while pane A is active must not reach pane B's composer"
    );
}

/// Enter must never submit an inactive assistant. Before stint 0429 the
/// composer's stale egui focus passed the `has_focus` submit gate even when
/// its pane was not the active pane.
#[test]
fn enter_never_submits_inactive_assistant() {
    let mut h = HostHarness::new();
    let pane_a = h.add_test_pane();
    let pane_b = h.add_assistant_pane();

    h.focus_pane(pane_b);
    h.run_frames(2);
    h.assistant_mut(pane_b).model.composer = "draft message".to_string();

    h.focus_pane(pane_a);
    h.run_frames(1);

    let turns_before = h.assistant_mut(pane_b).model.turns.len();
    frame_with_events(
        &mut h,
        vec![egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );

    let assistant = h.assistant_mut(pane_b);
    assert_eq!(
        assistant.model.turns.len(),
        turns_before,
        "Enter while pane A is active must not submit pane B's assistant"
    );
    assert_eq!(
        assistant.model.composer, "draft message",
        "the inactive assistant's draft must survive an Enter in another pane"
    );
}

/// Sanity: the active assistant still receives typed text (the reconciler
/// grants its composer focus as the input owner's default surface).
#[test]
fn active_assistant_composer_receives_typed_text() {
    let mut h = HostHarness::new();
    let _pane_a = h.add_test_pane();
    let pane_b = h.add_assistant_pane();

    h.focus_pane(pane_b);
    h.run_frames(2);

    frame_with_events(&mut h, vec![egui::Event::Text("hi".to_string())]);

    assert_eq!(
        h.assistant_mut(pane_b).model.composer,
        "hi",
        "the active assistant's composer must receive typed text"
    );
}

/// Synthetic text injection (`plexi pane send`, drive-host, harness) arrives
/// while the OS window is blurred — the CLI issuing it runs in another
/// window. OS blur must not un-own the composer: egui focus stays projected
/// from host pane focus, so the injected Text event still lands. Regression
/// (PR #2421 tester): the reconciler treated `OsUnfocused` as "no owner",
/// surrendered the composer, and every pane-send into an assistant dropped.
#[test]
fn blurred_window_still_delivers_injected_text_to_active_composer() {
    let mut h = HostHarness::new();
    let _pane_a = h.add_test_pane();
    let pane_b = h.add_assistant_pane();

    h.focus_pane(pane_b);
    h.run_frames(2);

    // Blur the OS window and let a frame settle — the composer must keep
    // egui focus through the blur.
    let mut blurred = egui::RawInput::default();
    blurred
        .viewports
        .entry(egui::ViewportId::ROOT)
        .or_default()
        .focused = Some(false);
    h.frame(blurred.clone());

    // Inject text the way `SendToPane` does, still blurred.
    let mut with_text = blurred;
    with_text.events.push(egui::Event::Text("hi".to_string()));
    h.frame(with_text);

    assert_eq!(
        h.assistant_mut(pane_b).model.composer,
        "hi",
        "pane-send text must reach the active composer while the window is blurred"
    );
}

/// Overlay-over-pane ownership: while the command palette is open, the
/// palette search field owns egui focus even though a pane is host-focused;
/// closing the palette hands focus ownership back to the pane's surface.
#[test]
fn command_palette_owns_focus_over_focused_assistant_pane() {
    let mut h = HostHarness::new();
    let pane_b = h.add_assistant_pane();
    h.focus_pane(pane_b);
    h.run_frames(2);

    h.app.show_command_palette = true;
    h.app.sync_command_palette_focus();
    h.run_frames(2);

    assert_eq!(
        h.app.ctx.memory(|m| m.focused()),
        Some(egui::Id::new("palette_search")),
        "palette search must own egui focus while the palette is open"
    );

    // Typing goes to the palette, not the assistant composer underneath.
    frame_with_events(&mut h, vec![egui::Event::Text("q".to_string())]);
    assert_eq!(
        h.assistant_mut(pane_b).model.composer,
        "",
        "typed text must stay in the palette while it owns input"
    );
}

/// Derivation precedence for `InputOwner` (stint 0429):
/// OS focus > overlay surfaces > the focused pane.
#[test]
fn input_owner_precedence_os_overlay_pane() {
    use crate::app::input_owner::{InputOwner, OverlaySurface};

    let mut h = HostHarness::new();
    let pane = h.add_test_pane();
    h.focus_pane(pane);
    h.run_frames(1);

    let ctx = h.app.ctx.clone();
    assert_eq!(
        h.app.input_owner(&ctx),
        InputOwner::Pane(pane),
        "with no overlay, the focused pane owns input"
    );

    // Inline sidebar rename outranks the pane.
    h.app.sidebar_visible = true;
    h.app.renaming_window = Some(0);
    assert_eq!(
        h.app.input_owner(&ctx),
        InputOwner::Overlay(OverlaySurface::SidebarRename),
        "the inline sidebar rename editor owns input over the focused pane"
    );

    // A modal focus layer outranks the inline editor.
    h.app
        .push_focus_layer(crate::app::FocusKind::CommandPalette);
    assert_eq!(
        h.app.input_owner(&ctx),
        InputOwner::Overlay(OverlaySurface::Layer(crate::app::FocusKind::CommandPalette)),
        "the focus-stack top outranks inline editors"
    );
    h.app.renaming_window = None;
    h.app
        .pop_focus_layer(&crate::app::FocusKind::CommandPalette);

    // An unfocused OS window outranks everything.
    let mut raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1280.0, 800.0),
        )),
        ..Default::default()
    };
    raw.viewports
        .entry(egui::ViewportId::ROOT)
        .or_default()
        .focused = Some(false);
    h.frame(raw);
    let ctx = h.app.ctx.clone();
    assert_eq!(
        h.app.input_owner(&ctx),
        InputOwner::OsUnfocused,
        "an unfocused OS window owns nothing"
    );
}

/// Typing into the inline sidebar rename must not reach the focused app
/// pane's `handle_key` — the editor is `InputOwner::Overlay(SidebarRename)`,
/// so `dispatch_app_key_events` is gated off (stint 0429). Before, the keys
/// fed both the rename box and the app.
#[test]
fn sidebar_rename_keys_do_not_reach_focused_app_pane() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut h = HostHarness::new();
    h.app.open_builtin_app_pane(
        Box::<KeyBurstProbe>::default(),
        AppPermissions::builtin(),
        tmp.path().to_path_buf(),
        None,
        Some("split_h"),
        None,
    );
    let pane_id = h.state().open_panes[0];
    h.focus_pane(pane_id);
    h.run_frames(1);

    h.app.sidebar_visible = true;
    h.app.renaming_window = Some(0);
    h.run_frames(1);

    frame_with_events(
        &mut h,
        vec![egui::Event::Key {
            key: egui::Key::ArrowDown,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );

    let state = h.app.windows[0]
        .panes
        .get(&pane_id)
        .and_then(Pane::as_app)
        .and_then(|pane| pane.runtime.serialize_state())
        .expect("burst probe state");
    assert_eq!(
        state["received"],
        serde_json::json!([] as [&str; 0]),
        "keys typed during sidebar rename must not reach the focused app pane"
    );
}

// -- First-boot context seeding (stint 0436) ------------------------------

/// Stint 0436: a fresh-profile boot must land in a live root terminal, not the
/// empty welcome screen. The real fix lives in `PlexiApp::new`'s Default branch,
/// but the harness bypasses `PlexiApp::new` — `new_for_test` hand-builds an empty
/// window 0 (no root pane, no focus), exactly the pre-seed first-boot shape. So
/// we drive the closest reachable seam: `seed_window_root_pane(0, ..)`, which is
/// precisely what the Default branch now calls on window 0.
#[test]
fn first_boot_seam_seeds_base_root_pane() {
    let mut h = HostHarness::new();
    // new_for_test starts window 0 empty — the old first-boot welcome state.
    assert!(
        h.app.windows[0].panes.is_empty(),
        "setup: window starts with no panes"
    );
    assert!(
        h.app.windows[0].tree.root.is_none(),
        "setup: window starts with no tree root"
    );
    assert_eq!(
        h.app.windows[0].focused_pane, None,
        "setup: window starts with no focused pane"
    );

    let cwd = std::env::temp_dir();
    if h.app.seed_window_root_pane(0, cwd, None, false).is_none() {
        // No PTY in this env — the installer degrades to the welcome screen,
        // matching first boot's documented fallback. Nothing more to assert.
        return;
    }

    let win = &h.app.windows[0];
    assert_eq!(win.panes.len(), 1, "seeded window has exactly one pane");
    assert!(
        win.panes.values().all(|p| p.as_terminal().is_some()),
        "seeded root pane is a terminal"
    );
    assert!(win.tree.root.is_some(), "seeded window has a tree root");
    assert!(
        win.focused_pane.is_some(),
        "seeded window has a focused pane"
    );
}

/// The seeded first-boot window is identical in shape to a context created via
/// the new-context flow — both funnel through `seed_window_root_pane`, so a fresh
/// boot is indistinguishable from a manually created context.
#[test]
fn first_boot_shape_matches_new_context() {
    let mut h = HostHarness::new();
    let cwd = std::env::temp_dir();
    if h.app.seed_window_root_pane(0, cwd, None, false).is_none() {
        return; // no PTY in this env
    }
    let boot_pane_count = h.app.windows[0].panes.len();
    let boot_has_root = h.app.windows[0].tree.root.is_some();
    let boot_has_focus = h.app.windows[0].focused_pane.is_some();

    h.app.new_context();
    if h.app.router.len() == 1 {
        return; // new_context degraded (no PTY) — nothing to compare
    }
    let new_idx = h.app.active_window;
    let ctx = &h.app.windows[new_idx];
    assert_eq!(
        ctx.panes.len(),
        boot_pane_count,
        "new-context window has the same pane count as first boot"
    );
    assert_eq!(
        ctx.tree.root.is_some(),
        boot_has_root,
        "new-context window has a tree root iff first boot does"
    );
    assert_eq!(
        ctx.focused_pane.is_some(),
        boot_has_focus,
        "new-context window has a focused pane iff first boot does"
    );
    assert!(
        ctx.panes.values().all(|p| p.as_terminal().is_some()),
        "new-context root pane is a terminal"
    );
}

// -- Stint 0456: focused TextInput typing ----------------------------------

/// One pressed key event with no modifiers.
fn pressed_key(key: egui::Key) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

/// The label of the first `text` semantic node starting with `prefix`, from
/// the pane's current semantic tree.
fn text_label_with_prefix(h: &HostHarness, pane_id: PaneId, prefix: &str) -> Option<String> {
    let state = h.app.windows[h.app.active_window]
        .panes
        .get(&pane_id)
        .and_then(Pane::as_app)
        .map(|pane| pane.semantic_state())?;
    state
        .nodes
        .iter()
        .filter(|n| n.role == "text")
        .filter_map(|n| n.label.clone())
        .find(|label| label.starts_with(prefix))
}

/// Poll idle frames until the pane's semantic tree contains a `text` label
/// exactly equal to `expected`, panicking after 30s with the last seen state.
fn wait_for_text_label(h: &mut HostHarness, pane_id: PaneId, prefix: &str, expected: &str) {
    let start = std::time::Instant::now();
    loop {
        h.run_frames(1);
        let seen = text_label_with_prefix(h, pane_id, prefix);
        if seen.as_deref() == Some(expected) {
            return;
        }
        assert!(
            start.elapsed() < std::time::Duration::from_secs(30),
            "timed out waiting for label '{expected}' (last seen: {seen:?})"
        );
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
}

/// Stint 0456: end-to-end typing contract for the declarative `TextInput`,
/// against a REAL process app (`apps/dev/text-input-probe`). Keystrokes
/// with no field focused route to the app's raw `KeyEvent` path; after a
/// node-targeted click focuses the field, the dispatch gate
/// (`focused_pane_text_surface`) keeps keystrokes with the host TextEdit —
/// the guest's `keys:` counter must stay frozen — while `on_change`
/// round-trips the draft and Enter fires `on_submit` without dropping
/// focus.
#[test]
fn focused_text_input_receives_typing_and_enter_submits() {
    let mut h = HostHarness::new();

    let app_dir =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("apps/dev/text-input-probe");
    h.app
        .launch_app_by_path_with_layout(&app_dir.to_string_lossy(), None, None, &[])
        .expect("launch text-input-probe");
    let pane_id = *h
        .state()
        .open_panes
        .last()
        .expect("a pane appears after launching text-input-probe");

    // Real subprocess: poll for its first committed render before driving input.
    let start = std::time::Instant::now();
    loop {
        h.run_frames(1);
        let rendered = h.app.windows[h.app.active_window]
            .panes
            .get(&pane_id)
            .and_then(Pane::as_app)
            .is_some_and(
                |pane| matches!(&pane.runtime, AppRuntime::Python(p) if p.has_rendered_tree()),
            );
        if rendered {
            break;
        }
        assert!(
            start.elapsed() < std::time::Duration::from_secs(30),
            "text-input-probe did not render its first frame in time"
        );
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    h.focus_pane(pane_id);
    h.run_frames(2);

    // Baseline: no field focused — a keystroke routes to the app's raw
    // KeyEvent path and bumps the guest's counter.
    frame_with_events(&mut h, vec![pressed_key(egui::Key::X)]);
    wait_for_text_label(&mut h, pane_id, "keys:", "keys:1");

    // Focus the TextInput by node id, exactly as `plexi pane click --node`
    // would. The claim routes through the focus reconciler at frame end.
    let input_node_id = h.app.windows[h.app.active_window]
        .panes
        .get(&pane_id)
        .and_then(Pane::as_app)
        .map(|pane| pane.semantic_state())
        .and_then(|state| {
            state
                .nodes
                .iter()
                .find(|n| n.role == "text_input")
                .map(|n| n.id.clone())
        })
        .expect("semantic tree exposes the TextInput node");
    h.inject_node_click(pane_id, &input_node_id, "left", None);
    h.run_frames(2);

    // Typing now lands in the host TextEdit: the draft round-trips through
    // on_change while the guest's raw-key counter stays frozen.
    frame_with_events(
        &mut h,
        vec![
            pressed_key(egui::Key::H),
            egui::Event::Text("h".to_string()),
        ],
    );
    wait_for_text_label(&mut h, pane_id, "draft:", "draft:h");
    assert_eq!(
        text_label_with_prefix(&h, pane_id, "keys:").as_deref(),
        Some("keys:1"),
        "keystrokes into the focused TextInput must not reach the app's KeyEvent path"
    );

    // Enter fires on_submit (the probe records the draft and clears it).
    frame_with_events(&mut h, vec![pressed_key(egui::Key::Enter)]);
    wait_for_text_label(&mut h, pane_id, "submitted:", "submitted:h");
    wait_for_text_label(&mut h, pane_id, "draft:", "draft:");

    // Focus survives the submit: more typing lands without another click.
    frame_with_events(
        &mut h,
        vec![
            pressed_key(egui::Key::I),
            egui::Event::Text("i".to_string()),
        ],
    );
    wait_for_text_label(&mut h, pane_id, "draft:", "draft:i");
    assert_eq!(
        text_label_with_prefix(&h, pane_id, "keys:").as_deref(),
        Some("keys:1"),
        "post-submit typing must stay with the still-focused TextInput"
    );

    // Escape leaves the field without closing the app (the AppActive
    // CloseApp binding must not fire); the next keystroke routes to the
    // app's raw KeyEvent path again.
    let panes_before = h.pane_count();
    frame_with_events(&mut h, vec![pressed_key(egui::Key::Escape)]);
    h.run_frames(2);
    assert_eq!(
        h.pane_count(),
        panes_before,
        "Escape in a focused TextInput must not close the app pane"
    );
    frame_with_events(&mut h, vec![pressed_key(egui::Key::Z)]);
    wait_for_text_label(&mut h, pane_id, "keys:", "keys:2");
}

// ─── Editor release gate: host-driven layer (stint 0479) ─────────────────────

/// One host-level input step for the notes pane: either raw text through
/// `SendToPane` or a key combo through `KeyPane`.
enum GateHostInput {
    Text(String),
    Key(String),
}

fn gate_movement_key(
    movement: crate::editor::commands::Movement,
    extend: bool,
) -> Option<String> {
    use crate::editor::commands::Movement;
    let base = match movement {
        Movement::Left => "left",
        Movement::Right => "right",
        Movement::Up => "up",
        Movement::Down => "down",
        Movement::WordLeft => "alt+left",
        Movement::WordRight => "alt+right",
        Movement::LineStart => "cmd+left",
        Movement::LineEnd => "cmd+right",
        Movement::VisualLineStart => "home",
        Movement::VisualLineEnd => "end",
        Movement::DocStart => "cmd+up",
        Movement::DocEnd => "cmd+down",
        Movement::PageUp(_) | Movement::PageDown(_) => return None,
    };
    Some(if extend {
        format!("shift+{base}")
    } else {
        base.to_string()
    })
}

/// Maps a pure editor command onto the equivalent installed-host inputs, or
/// `None` when the host keyboard surface cannot express it (IME, page
/// movement, non-collapsed pointer placements). `SetCursor` expands into an
/// arrow-key walk, valid for the ASCII documents the whitelist uses.
fn gate_host_inputs_for(command: &crate::editor::EditorCommand) -> Option<Vec<GateHostInput>> {
    use crate::editor::EditorCommand;
    let key = |k: &str| Some(vec![GateHostInput::Key(k.to_string())]);
    match command {
        EditorCommand::InsertText(text) => Some(vec![GateHostInput::Text(text.clone())]),
        EditorCommand::InsertNewline | EditorCommand::MarkdownNewline => key("enter"),
        EditorCommand::Backspace | EditorCommand::MarkdownBackspace => key("backspace"),
        EditorCommand::DeleteForward => key("delete"),
        EditorCommand::MarkdownIndent => key("tab"),
        EditorCommand::MarkdownOutdent => key("shift+tab"),
        EditorCommand::Undo => key("cmd+z"),
        EditorCommand::Redo => key("cmd+shift+z"),
        EditorCommand::SelectAll => key("cmd+a"),
        EditorCommand::Move { movement, extend } => {
            gate_movement_key(*movement, *extend).map(|k| vec![GateHostInput::Key(k)])
        }
        EditorCommand::SetCursor(cursor) => {
            let mut keys = vec![GateHostInput::Key("cmd+up".to_string())];
            keys.extend((0..cursor.line).map(|_| GateHostInput::Key("down".to_string())));
            keys.push(GateHostInput::Key("cmd+left".to_string()));
            keys.extend((0..cursor.column).map(|_| GateHostInput::Key("right".to_string())));
            Some(keys)
        }
        _ => None,
    }
}

/// Gate cases the host keyboard surface can replay faithfully in a Markdown
/// note: every command maps, and the plain command matches what the widget's
/// Markdown key bindings dispatch (Enter → MarkdownNewline etc. degrade to
/// the same plain behavior on these inputs).
const GATE_HARNESS_CASES: &[&str] = &[
    "plain_typing",
    "typing_after_move_two_groups",
    "unicode_accented_typing",
    "grapheme_combining_backspace",
    "zwj_emoji_backspace",
    "flag_emoji_typing",
    "combining_mark_left_navigation",
    "nav_word_left",
    "nav_word_right",
    "nav_vertical_goal_clamp",
    "home_end_equivalents",
    "doc_start_end_movements",
    "select_all_replace",
    "shift_word_select",
    "delete_forward_at_end_noop",
    "backspace_join_lines",
    "undo_coalesced_group",
    "redo_after_undo",
    "redo_invalidated_by_new_edit",
    "markdown_newline_unordered",
    "markdown_newline_task",
    "markdown_newline_quote",
    "markdown_empty_continuation_exits",
    "markdown_backspace_empty_marker",
    "markdown_backspace_plain_content",
    "smart_backspace_indent_level",
    "newline_carries_auto_indent",
];

fn open_gate_note(h: &mut HostHarness, dir: &std::path::Path, initial: &str) -> PaneId {
    let note = dir.join("gate-note.md");
    std::fs::write(&note, initial).expect("seed gate note");
    h.app.open_builtin_app_pane(
        Box::new(crate::app::text_editor_app::TextEditorApp::new_for_test_note(note)),
        crate::app::permissions::AppPermissions::builtin(),
        dir.to_path_buf(),
        None,
        Some("split_h"),
        None,
    );
    let pane_id = *h.state().open_panes.last().expect("notes pane opened");
    h.run_frames(2);
    pane_id
}

fn gate_note_semantics(h: &HostHarness, pane_id: PaneId) -> serde_json::Value {
    h.app.windows[0]
        .panes
        .get(&pane_id)
        .and_then(Pane::as_app)
        .and_then(|pane| pane.runtime.semantic_details())
        .expect("notes semantic details")
}

/// Char offset of `cursor` within `text` (the notes semantic `caret` unit).
fn gate_char_offset(text: &str, cursor: crate::editor::Cursor) -> usize {
    let mut offset = 0;
    for (index, line) in text.split('\n').enumerate() {
        if index == cursor.line {
            return offset + cursor.column;
        }
        offset += line.chars().count() + 1;
    }
    offset + cursor.column
}

/// Drives a representative subset of the editor gate matrix through the real
/// installed-host input paths (`SendToPane` text + `KeyPane` combos against a
/// production builtin Notes pane) and asserts the pane's semantic JSON agrees
/// with the pure-model expectations after every step.
#[test]
fn editor_gate_harness_matrix() {
    let cases = crate::editor::gate::gate_cases();
    for name in GATE_HARNESS_CASES {
        let case = cases
            .iter()
            .find(|case| case.name == *name)
            .unwrap_or_else(|| panic!("gate case {name:?} missing from gate_cases()"));
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut h = HostHarness::new();
        let pane_id = open_gate_note(&mut h, tmp.path(), case.initial);

        for (index, command) in case.commands.iter().enumerate() {
            let inputs = gate_host_inputs_for(command).unwrap_or_else(|| {
                panic!("case {name:?} command[{index}] {command:?} is not host-expressible")
            });
            for (sub, input) in inputs.into_iter().enumerate() {
                let response = temp_response(tmp.path(), &format!("gate-{name}-{index}-{sub}"));
                match input {
                    GateHostInput::Text(text) => h.inject_ipc(AppRequest::SendToPane {
                        pane_id,
                        text,
                        submit: false,
                        response_file: Some(response.clone()),
                    }),
                    GateHostInput::Key(key) => h.inject_ipc(AppRequest::KeyPane {
                        pane_id,
                        key,
                        response_file: Some(response.clone()),
                    }),
                };
                h.run_frames(2);
                assert_eq!(
                    read_json_response(&response)["ok"],
                    true,
                    "case {name:?} command[{index}] host input failed"
                );
            }
        }

        let state = gate_note_semantics(&h, pane_id);
        assert_eq!(
            state["source_text"].as_str(),
            Some(case.expect.text),
            "case {name:?}: host-driven text diverged from the gate expectation; state={state}"
        );
        if let Some(cursor) = case.expect.cursor {
            assert_eq!(
                state["caret"].as_u64(),
                Some(gate_char_offset(case.expect.text, cursor) as u64),
                "case {name:?}: caret mismatch; state={state}"
            );
        }
        if let Some(depth) = case.expect.undo_depth {
            assert_eq!(
                state["undo_available"].as_bool(),
                Some(depth > 0),
                "case {name:?}: undo_available mismatch; state={state}"
            );
        }
        if let Some(can_redo) = case.expect.can_redo {
            assert_eq!(
                state["redo_available"].as_bool(),
                Some(can_redo),
                "case {name:?}: redo_available mismatch; state={state}"
            );
        }
    }
}

/// Host-only editor gate surfaces the pure matrix cannot cover: pointer caret
/// placement, Live Preview toggling, save success and failure reporting, and
/// drop accept/reject — all through production dispatch paths.
#[test]
fn editor_gate_host_surfaces() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut h = HostHarness::new();
    let body: String = (0..30).map(|i| format!("gate line {i}\n")).collect();
    let pane_id = open_gate_note(&mut h, tmp.path(), &body);

    // Pointer caret placement through the production click path.
    let response = temp_response(tmp.path(), "gate-click");
    h.inject_click(pane_id, 60.0, 120.0, "left", Some(response.clone()));
    h.run_frames(3);
    assert_eq!(read_json_response(&response)["ok"], true);
    let state = gate_note_semantics(&h, pane_id);
    assert!(
        state["caret"].as_u64().expect("caret") > 0,
        "click must move the caret off offset 0; state={state}"
    );

    // The production semantic commit path (accesskit → SemanticPaneState in
    // app_pane::render, the exact state `plexi pane state` serves live) must
    // carry the editor's rendered text so `rendered_text_contains` scene
    // assertions are truthful against an installed host.
    let semantic = h.app.windows[0]
        .panes
        .get(&pane_id)
        .and_then(Pane::as_app)
        .expect("notes pane")
        .semantic_state();
    assert!(
        semantic.nodes.iter().any(|node| {
            node.role == "paragraph"
                && node
                    .value
                    .as_deref()
                    .is_some_and(|value| value.contains("gate line 0"))
        }),
        "pane semantic nodes must include per-row rendered editor text; nodes={:?}",
        semantic.nodes
    );

    // Live Preview toggle through the app's real key handler.
    assert_eq!(state["preview_mode"].as_str(), Some("live_preview"));
    let response = temp_response(tmp.path(), "gate-toggle");
    h.inject_ipc(AppRequest::KeyPane {
        pane_id,
        key: "cmd+g".to_string(),
        response_file: Some(response.clone()),
    });
    h.run_frames(2);
    assert_eq!(read_json_response(&response)["ok"], true);
    let state = gate_note_semantics(&h, pane_id);
    assert_eq!(state["preview_mode"].as_str(), Some("source"));

    // Cmd+S saves and reports ok.
    let response = temp_response(tmp.path(), "gate-save");
    h.inject_ipc(AppRequest::KeyPane {
        pane_id,
        key: "cmd+s".to_string(),
        response_file: Some(response.clone()),
    });
    h.run_frames(2);
    assert_eq!(read_json_response(&response)["ok"], true);
    let state = gate_note_semantics(&h, pane_id);
    assert_eq!(state["last_save_result"].as_str(), Some("ok"));

    // Drop accept: a real decodable image lands as an assets/ reference.
    let png = tmp.path().join("pic.png");
    image::RgbaImage::new(2, 2)
        .save(&png)
        .expect("write fixture png");
    let response = temp_response(tmp.path(), "gate-drop-accept");
    h.inject_ipc(AppRequest::DropFile {
        pane_id,
        path_or_url: png.to_string_lossy().into_owned(),
        response_file: response.clone(),
    });
    h.run_frames(2);
    let drop_response = read_json_response(&response);
    assert!(
        drop_response.get("error").is_none(),
        "image drop must be accepted: {drop_response:?}"
    );
    let state = gate_note_semantics(&h, pane_id);
    assert_eq!(state["last_drop_result"]["result"].as_str(), Some("accepted"));
    assert!(
        state["source_text"]
            .as_str()
            .is_some_and(|text| text.contains("![](assets/")),
        "accepted drop must insert an assets/ reference; state={state}"
    );

    // Drop reject: undecodable content reports a semantic rejection.
    let junk = tmp.path().join("junk.png");
    std::fs::write(&junk, b"not an image").expect("write junk");
    let response = temp_response(tmp.path(), "gate-drop-reject");
    h.inject_ipc(AppRequest::DropFile {
        pane_id,
        path_or_url: junk.to_string_lossy().into_owned(),
        response_file: response.clone(),
    });
    h.run_frames(2);
    assert!(read_json_response(&response)["error"]
        .as_str()
        .is_some_and(|error| error.contains("not a decodable image")));
    let state = gate_note_semantics(&h, pane_id);
    assert_eq!(state["last_drop_result"]["result"].as_str(), Some("rejected"));
}

/// Save failure must surface through the host path: Cmd+S against a note
/// whose parent is blocked by a plain file reports `last_save_result` as an
/// error while the buffer stays dirty.
#[test]
fn editor_gate_save_failure_reports_error_semantically() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().expect("tempdir");
    let note_dir = tmp.path().join("note-dir");
    std::fs::create_dir_all(&note_dir).expect("create note dir");
    let mut h = HostHarness::new();
    let pane_id = open_gate_note(&mut h, &note_dir, "seed ");

    let response = temp_response(tmp.path(), "gate-save-fail-text");
    h.inject_ipc(AppRequest::SendToPane {
        pane_id,
        text: "unsaveable".to_string(),
        submit: false,
        response_file: Some(response.clone()),
    });
    h.run_frames(2);
    assert_eq!(read_json_response(&response)["ok"], true);

    // Make the note's directory unwritable so the atomic save (temp file in
    // the same directory) fails.
    std::fs::set_permissions(&note_dir, std::fs::Permissions::from_mode(0o555))
        .expect("make note dir read-only");
    let response = temp_response(tmp.path(), "gate-save-fail-key");
    h.inject_ipc(AppRequest::KeyPane {
        pane_id,
        key: "cmd+s".to_string(),
        response_file: Some(response.clone()),
    });
    h.run_frames(2);
    assert_eq!(read_json_response(&response)["ok"], true);
    let state = gate_note_semantics(&h, pane_id);
    assert!(
        state["last_save_result"]
            .as_str()
            .is_some_and(|result| result.starts_with("error:")),
        "failed save must report an error result; state={state}"
    );
    assert_eq!(
        state["dirty"].as_bool(),
        Some(true),
        "failed save must stay dirty for retry; state={state}"
    );

    // Restore write access so the pane's Drop-flush retry succeeds at
    // teardown (and the tempdir can be cleaned up).
    std::fs::set_permissions(&note_dir, std::fs::Permissions::from_mode(0o755))
        .expect("restore note dir permissions");
}

/// `plexi host log` / `AppRequest::LogMarker` must land in the host process
/// (the owner of the channel logger) and acknowledge through the response
/// file — the mechanism installed release gates use to leave start/finish
/// summaries in `~/.plexi-<channel>/plexi.log`.
#[test]
fn host_log_marker_dispatches_and_acknowledges() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut h = HostHarness::new();
    let response = temp_response(tmp.path(), "log-marker");
    h.app.handle_pane_ipc_request(AppRequest::LogMarker {
        source: "editor_gate".to_string(),
        message: "started scenes=9\nsecond line flattened".to_string(),
        response_file: Some(response.clone()),
    });
    assert_eq!(read_json_response(&response)["ok"], true);
}


// -- Stint 0505: focused-terminal keyboard ownership --------------------------
//
// After the s3 editor sprint (egui 0.34 upgrade + shared editor core), plain
// Tab stopped reaching terminal panes: egui 0.34's built-in focus traversal
// claimed Tab/arrows/Escape from the focused terminal because the terminal
// widget carried the default `EventFilter` (nothing locked). The reconciler
// restores egui focus at frame end, so the *final* focus state looks correct —
// the only observable difference is whether the keystroke's bytes actually
// reached the PTY writer. These tests assert exactly that via the backend's
// input tap, so they fail on the regression (bytes dropped) and pass once the
// reconciler locks the traversal keys to the focused terminal.

/// Failing-first regression guard for stint 0505: a plain Tab pressed while a
/// terminal pane is focused must reach the PTY as `\x09`. Before the fix the
/// byte is dropped (egui steals Tab for focus traversal); after, it arrives.
#[test]
fn plain_tab_reaches_focused_terminal_pty() {
    let mut h = HostHarness::new();
    let term = h.add_focused_terminal();
    let wid = egui_term::terminal_widget_id(term);
    assert_eq!(
        h.ctx.memory(|m| m.focused()),
        Some(wid),
        "terminal must hold egui focus before the Tab press"
    );

    h.terminal_backend(term).enable_input_tap();
    h.press_key(egui::Key::Tab, egui::Modifiers::NONE);

    let written = h.terminal_backend(term).take_input_tap();
    assert_eq!(
        written, b"\x09",
        "plain Tab must reach the focused terminal's PTY as 0x09, got {written:?}"
    );
}

/// The other keys a TUI depends on must survive the same host consumer chain
/// and reach the PTY unmodified: Escape (`\x1b`, e.g. leaving vim insert mode),
/// Enter (`\r`), Shift+Tab (`\x1b[Z`, which already worked — a guard against
/// regressing the backward-traversal case), and a bare arrow (`\x1b[A`).
#[test]
fn traversal_and_control_keys_reach_focused_terminal_pty() {
    let mut h = HostHarness::new();
    let term = h.add_focused_terminal();

    let cases: &[(egui::Key, egui::Modifiers, &[u8])] = &[
        (egui::Key::Escape, egui::Modifiers::NONE, b"\x1b"),
        (egui::Key::Enter, egui::Modifiers::NONE, b"\r"),
        (egui::Key::Tab, egui::Modifiers::SHIFT, b"\x1b[Z"),
        (egui::Key::ArrowUp, egui::Modifiers::NONE, b"\x1b[A"),
    ];
    for (key, modifiers, expected) in cases {
        h.terminal_backend(term).enable_input_tap();
        h.press_key(*key, *modifiers);
        let written = h.terminal_backend(term).take_input_tap();
        assert_eq!(
            written, *expected,
            "{key:?} (mods={modifiers:?}) must reach the PTY as {expected:?}, got {written:?}"
        );
    }
}

/// Editor-scoped chords must not hijack a focused terminal. The editor's
/// link-activation (Ctrl+Enter) and find (Cmd+F) live in editor/notes app
/// panes; with a terminal focused, no editor renders, so these must leave the
/// terminal as the input owner with egui focus intact — never surrender it to
/// an editor surface or overlay. Guards the broader stint 0505 invariant that
/// key scoping is focus-scoped.
#[test]
fn editor_chords_do_not_hijack_focused_terminal() {
    use crate::app::input_owner::InputOwner;
    let mut h = HostHarness::new();
    let term = h.add_focused_terminal();
    let wid = egui_term::terminal_widget_id(term);

    for (key, modifiers) in [
        (egui::Key::Enter, egui::Modifiers::CTRL),
        (egui::Key::F, egui::Modifiers::COMMAND),
    ] {
        h.press_key(key, modifiers);
        let ctx = h.ctx.clone();
        assert_eq!(
            h.app.input_owner(&ctx),
            InputOwner::Pane(term),
            "{key:?}+{modifiers:?} must leave the terminal as input owner"
        );
        assert_eq!(
            h.ctx.memory(|m| m.focused()),
            Some(wid),
            "{key:?}+{modifiers:?} must not steal egui focus from the terminal"
        );
    }
}

// -- stint 0506: notes/editor UX regressions ------------------------------

/// Open a focused, settled text-editor note pane holding `body`. Returns its
/// pane id after the reconciler has granted the editor egui focus.
fn open_focused_note(h: &mut HostHarness, dir: &std::path::Path, body: &str) -> PaneId {
    let note = dir.join("note.md");
    std::fs::write(&note, body).expect("seed note");
    h.app.open_builtin_app_pane(
        Box::new(crate::app::text_editor_app::TextEditorApp::new_for_test_note(note)),
        crate::app::permissions::AppPermissions::builtin(),
        dir.to_path_buf(),
        None,
        Some("split_h"),
        None,
    );
    let pane_id = h.state().open_panes[0];
    h.focus_pane(pane_id);
    h.run_frames(3);
    pane_id
}

fn note_semantics(h: &HostHarness, pane_id: PaneId) -> serde_json::Value {
    h.app.windows[0]
        .panes
        .get(&pane_id)
        .and_then(Pane::as_app)
        .expect("notes pane")
        .runtime
        .semantic_details()
        .expect("notes semantics")
}

/// A fresh scratch editor opened while the host is occluded still owns input
/// once it becomes visible: pane text waits for the editor's first focusable
/// render instead of being discarded by that render's preamble.
#[test]
fn blank_text_editor_open_acquires_input_focus() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut h = HostHarness::new();
    let open_response = temp_response(tmp.path(), "blank-editor-open");
    h.inject_ipc(AppRequest::SpawnPane {
        type_id: "text-editor".to_string(),
        layout: Some("split_h".to_string()),
        args: vec![],
        from_pane_id: None,
        request_id: None,
        response_file: Some(open_response.clone()),
        ephemeral: false,
        cwd: Some(tmp.path().to_string_lossy().into_owned()),
        no_focus: false,
        path: None,
        workspace_root: None,
        target_context: None,
        context_name: None,
        name: None,
        agent_cmd: None,
        boot_timeout_secs: None,
    });
    // R3 moved IPC draining into logic so the editor can be opened while eframe
    // skips ui for an occluded window. At this point no text surface has
    // rendered or received egui focus yet.
    h.hidden_frame();
    let pane_id = read_json_response(&open_response)["pane_id"]
        .as_u64()
        .expect("open response pane id");

    let text_response = temp_response(tmp.path(), "blank-editor-text");
    h.inject_ipc(AppRequest::SendToPane {
        pane_id,
        text: "hello".to_string(),
        submit: false,
        response_file: Some(text_response.clone()),
    });
    h.hidden_frame();
    h.run_frames(4);
    assert_eq!(read_json_response(&text_response)["ok"], true);
    let after_text = note_semantics(&h, pane_id);
    assert_eq!(after_text["source_text"], "hello");
    assert_eq!(
        after_text["focused"], true,
        "editor owns input after first visible render"
    );

    let key_response = temp_response(tmp.path(), "blank-editor-ctrl-enter");
    h.inject_ipc(AppRequest::KeyPane {
        pane_id,
        key: "ctrl+enter".to_string(),
        response_file: Some(key_response.clone()),
    });
    h.run_frames(1);
    assert_ne!(
        read_json_response(&key_response)["disposition"],
        "passthrough",
        "focused editor must own Ctrl+Enter rather than fall through to pane routing"
    );
}

/// Ctrl+Enter with the caret inside a Markdown link activates it through the
/// real key path — the target note opens in a split (stint 0506 item 3).
#[test]
fn ctrl_enter_activates_markdown_link_at_caret() {
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::write(tmp.path().join("target.md"), "target body").expect("seed target");
    let mut h = HostHarness::new();
    // Caret defaults to (0,0), inside the leading [md](target.md) link.
    let pane_id = open_focused_note(&mut h, tmp.path(), "[md](target.md) rest");

    h.press_key(egui::Key::Enter, egui::Modifiers::CTRL);
    h.run_frames(1);

    let state = note_semantics(&h, pane_id);
    assert_eq!(
        state["last_link_activation"]["outcome"], "opened_note",
        "Ctrl+Enter should activate the markdown link at caret; state={state}"
    );
    assert_eq!(h.pane_count(), 2, "the target note opens in a split");
}

/// Ctrl+Enter on a `[[wiki]]` link to a nonexistent note creates the note and
/// opens it (stint 0506 items 3 + 4).
#[test]
fn ctrl_enter_on_missing_wiki_link_creates_and_opens_note() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut h = HostHarness::new();
    let notes_dir = crate::config::config_dir().join("notes");
    std::fs::create_dir_all(&notes_dir).expect("notes dir");
    let pane_id = open_focused_note(&mut h, tmp.path(), "[[fresh-idea]] rest");

    h.press_key(egui::Key::Enter, egui::Modifiers::CTRL);
    h.run_frames(1);

    let state = note_semantics(&h, pane_id);
    assert_eq!(
        state["last_link_activation"]["outcome"], "created_note",
        "missing wiki target should be created; state={state}"
    );
    assert!(notes_dir.join("fresh-idea.md").exists());
    assert_eq!(h.pane_count(), 2, "the new note opens in a split");
}

/// Escape while the note body is focused releases the editor to pane-level
/// navigation: the pane stays open and focused, but the editor no longer owns
/// the keyboard (stint 0506 item 1 / stint 0496).
#[test]
fn escape_releases_note_editor_to_pane_navigation() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut h = HostHarness::new();
    let pane_id = open_focused_note(&mut h, tmp.path(), "hello world");

    let before = note_semantics(&h, pane_id);
    assert_eq!(before["focused"], true, "editor owns input before Escape");
    let panes_before = h.pane_count();

    h.press_key(egui::Key::Escape, egui::Modifiers::NONE);
    h.run_frames(2);

    let after = note_semantics(&h, pane_id);
    assert_eq!(after["input_released"], true, "Escape releases the editor");
    assert_eq!(after["focused"], false, "editor no longer holds egui focus");
    assert_eq!(h.pane_count(), panes_before, "the pane stays open");
    use crate::app::input_owner::InputOwner;
    let ctx = h.ctx.clone();
    assert_eq!(
        h.app.input_owner(&ctx),
        InputOwner::Pane(pane_id),
        "the pane stays focused for pane-level navigation"
    );
}

/// Pressing Enter on a text file in the File Explorer opens it in the builtin
/// text-editor as a split to the right of the explorer — never the OS opener
/// (stint 0506 item 5).
#[test]
fn explorer_enter_opens_text_file_in_text_editor_split() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let doc = tmp.path().join("readme.md");
    std::fs::write(&doc, "# hello").expect("seed doc");
    let mut h = HostHarness::new();
    h.app.open_builtin_app_pane(
        Box::new(crate::file_browser::FileBrowserApp::new(tmp.path().to_path_buf())),
        crate::app::permissions::AppPermissions::builtin(),
        tmp.path().to_path_buf(),
        None,
        Some("split_h"),
        None,
    );
    let explorer = h.state().open_panes[0];
    h.focus_pane(explorer);
    h.run_frames(2);
    assert_eq!(h.pane_count(), 1);

    // The only entry (readme.md) is selected by default; Enter activates it.
    h.press_key(egui::Key::Enter, egui::Modifiers::NONE);
    h.run_frames(2);

    assert_eq!(
        h.pane_count(),
        2,
        "the text file opens a second pane, not the OS opener"
    );
    let opened = h.app.windows[0]
        .panes
        .iter()
        .find(|(id, _)| **id != explorer)
        .map(|(_, pane)| pane)
        .and_then(Pane::as_app)
        .expect("a new app pane");
    assert_eq!(opened.manifest_id, "text-editor");
}

// -- Hidden-window IPC servicing (stint 0505 fix round 3) -----------------

/// Regression guard for the occluded-host IPC wedge: eframe skips `App::ui`
/// entirely while the window is hidden (minimized or fully occluded by other
/// windows) and runs logic-only passes instead. Every external drain lived in
/// `ui`, so a fully covered host serviced no pane IPC at all — wakes fired,
/// passes ran, and queued commands (including `plexi host stop`'s Shutdown)
/// sat until the window was next uncovered. External servicing must complete
/// on hidden passes alone.
#[test]
fn pane_ipc_serviced_while_window_hidden() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut h = HostHarness::new();
    let pane = h.add_test_pane();
    h.run_frames(1);

    let list_file = temp_response(tmp.path(), "hidden-pane-list");
    h.inject_ipc(AppRequest::ListPanes {
        response_file: list_file.clone(),
        context_id: None,
    });
    h.run_hidden_frames(1);

    let list = std::fs::read_to_string(&list_file)
        .expect("pane list must be serviced on a hidden (logic-only) pass");
    let list: serde_json::Value = serde_json::from_str(&list).expect("valid JSON response");
    assert!(
        list.as_array()
            .expect("pane list array")
            .iter()
            .any(|entry| entry["id"].as_u64() == Some(pane)),
        "hidden-pass pane list must include the open pane"
    );
}

/// Companion to `pane_ipc_serviced_while_window_hidden`: queued pane inputs
/// must NOT be swallowed by hidden passes. A hidden pass has no widget pass to
/// receive events, so `raw_input_hook` must leave `pending_pane_inputs` queued
/// for the next visible frame instead of injecting them into a pass that
/// discards them unseen.
#[test]
fn pending_pane_inputs_survive_hidden_passes() {
    let mut h = HostHarness::new();
    let pane = h.add_test_pane();
    h.focus_pane(pane);
    h.run_frames(1);

    h.app.pending_pane_inputs.entry(pane).or_default().push(
        egui::RawInput {
            events: vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            ..Default::default()
        },
    );
    h.run_hidden_frames(3);
    assert!(
        h.app
            .pending_pane_inputs
            .get(&pane)
            .is_some_and(|batches| !batches.is_empty()),
        "queued pane input must survive hidden passes for the next visible frame"
    );

    // The visible-frame reconciler drops focus from a bare test pane, so
    // re-assert it the way a real un-occlusion refocus would before checking
    // that the queued input is finally delivered.
    h.focus_pane(pane);
    h.run_frames(1);
    assert!(
        h.app
            .pending_pane_inputs
            .get(&pane)
            .is_none_or(|batches| batches.is_empty()),
        "queued pane input must be delivered on the next visible frame"
    );
}

// -- Screenshot reply invariant (stints 0495, 0504) -----------------------

/// `plexi host screenshot` must ask the viewport for a capture even while the
/// window is hidden. The request was issued from `App::ui`, which eframe skips
/// entirely on an occluded or minimized host, so no capture was ever requested:
/// the PNG landed only once something un-occluded the window (after the CLI had
/// already timed out — stint 0495), or never at all (stint 0504).
#[test]
fn screenshot_capture_requested_on_hidden_passes() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut h = HostHarness::new();
    h.run_frames(1);

    let response_file = temp_response(tmp.path(), "hidden-screenshot");
    h.inject_ipc(AppRequest::Screenshot {
        pane_id: None,
        output_path: tmp.path().join("hidden.png").to_string_lossy().into_owned(),
        response_file,
    });
    h.run_hidden_frames(1);

    let pending = h
        .app
        .pending_screenshots
        .first()
        .expect("request must stay queued until the viewport delivers a capture");
    assert!(
        pending.capture_requested_at.is_some(),
        "a hidden (logic-only) pass must still issue the viewport capture request"
    );
}

/// Every screenshot request gets a typed reply — including when the compositor
/// never produces a frame at all. Before the bounded deadline the request sat
/// queued forever: no PNG, no response file, and nothing in the host log, so
/// the CLI reported its own generic timeout and the failure was
/// indistinguishable from a hung host.
#[test]
fn screenshot_without_viewport_delivery_replies_with_typed_error() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut h = HostHarness::new();
    h.run_frames(1);

    let response_file = temp_response(tmp.path(), "stalled-screenshot");
    let output_path = tmp.path().join("stalled.png").to_string_lossy().into_owned();
    h.inject_ipc(AppRequest::Screenshot {
        pane_id: None,
        output_path: output_path.clone(),
        response_file: response_file.clone(),
    });
    h.run_hidden_frames(1);

    // No capture is ever delivered. Nothing may be written before the deadline.
    h.run_hidden_frames(3);
    assert!(
        !std::path::Path::new(&response_file).exists(),
        "a request inside its deadline must not be answered early"
    );

    // Age the request past its deadline rather than sleeping for it.
    h.app
        .pending_screenshots
        .iter_mut()
        .for_each(|request| request.expires_at = std::time::Instant::now());
    h.run_hidden_frames(1);

    let response = read_json_response(&response_file);
    let error = response["error"]
        .as_str()
        .expect("undelivered screenshot must reply with a typed error");
    assert!(
        error.contains("screenshot readback did not complete"),
        "error must name the failure verbatim for the CLI to surface: {error}"
    );
    assert!(
        h.app.pending_screenshots.is_empty(),
        "an expired request must be dropped, not answered twice"
    );
    assert!(
        !std::path::Path::new(&output_path).exists(),
        "a failed capture must not leave a partial PNG behind"
    );
}

/// The deadline is a backstop, not a replacement: a delivered capture still
/// fulfills normally and writes the PNG plus a success response.
#[test]
fn screenshot_delivered_capture_still_fulfills() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut h = HostHarness::new();
    h.run_frames(1);

    let response_file = temp_response(tmp.path(), "delivered-screenshot");
    let output_path = tmp
        .path()
        .join("delivered.png")
        .to_string_lossy()
        .into_owned();
    h.inject_ipc(AppRequest::Screenshot {
        pane_id: None,
        output_path: output_path.clone(),
        response_file: response_file.clone(),
    });
    h.run_frames(1);

    // Stand in for wgpu's asynchronous readback, which no headless pass can
    // produce: hand the app the capture event eframe would have injected.
    let image = std::sync::Arc::new(egui::ColorImage::filled(
        [64, 48],
        egui::Color32::from_rgb(10, 20, 30),
    ));
    let raw_input = egui::RawInput {
        events: vec![egui::Event::Screenshot {
            viewport_id: egui::ViewportId::ROOT,
            user_data: egui::UserData::default(),
            image,
        }],
        ..Default::default()
    };
    let ctx = h.ctx.clone();
    h.app.fulfill_screenshot_events(&ctx, &raw_input);

    let response = read_json_response(&response_file);
    assert_eq!(
        response["ok"].as_bool(),
        Some(true),
        "a delivered capture must reply with success: {response}"
    );
    assert!(
        std::path::Path::new(&output_path).exists(),
        "a delivered capture must write the PNG"
    );
    assert!(
        h.app.pending_screenshots.is_empty(),
        "a fulfilled request must be dequeued"
    );
}

// -- Rename pre-selection (stint 0545) ------------------------------------

/// The sorted char range of a TextEdit's current selection, if any.
fn text_selection(ctx: &egui::Context, id: egui::Id) -> Option<(usize, usize)> {
    egui::TextEdit::load_state(ctx, id)
        .and_then(|state| state.cursor.char_range())
        .map(|range| {
            let [start, end] = range.sorted_cursors();
            (start.index, end.index)
        })
}

/// Stint 0545: the rename-pane overlay (Cmd+R) must open with its prefill
/// fully selected so typing replaces the old name. The reconciler grants
/// focus post-frame, after every widget has drawn, so egui's
/// `Response::gained_focus` never fires for reconciler-granted focus —
/// `TextField` must detect the focus edge itself.
#[test]
fn rename_pane_overlay_preselects_prefill() {
    let mut h = HostHarness::new();
    let pane = h.add_test_pane();
    h.app.renaming_pane = Some(pane);
    h.app.rename_buffer = "test name".to_string();
    h.app.push_focus_layer(crate::app::FocusKind::RenamePane);
    h.run_frames(3);

    let field = egui::Id::new("rename_pane_input");
    assert_eq!(
        h.ctx.memory(|m| m.focused()),
        Some(field),
        "rename field must hold focus after the reconciler grants it"
    );
    assert_eq!(
        text_selection(&h.ctx, field),
        Some((0, "test name".chars().count())),
        "rename-pane prefill must be fully selected on open"
    );
}

/// Stint 0545: the rename-context overlay (Cmd+Shift+R, sidebar hidden)
/// must open with its prefill fully selected.
#[test]
fn rename_context_overlay_preselects_prefill() {
    let mut h = HostHarness::new();
    h.app.renaming_window = Some(0);
    h.app.rename_buffer = "new context".to_string();
    h.app.sidebar_visible = false;
    h.app.push_focus_layer(crate::app::FocusKind::ContextRename);
    h.run_frames(3);

    let field = egui::Id::new("rename_context_input");
    assert_eq!(
        h.ctx.memory(|m| m.focused()),
        Some(field),
        "context rename field must hold focus after the reconciler grants it"
    );
    assert_eq!(
        text_selection(&h.ctx, field),
        Some((0, "new context".chars().count())),
        "rename-context prefill must be fully selected on open"
    );
}

/// Run `n` frames with the OS window blurred (`RawInput::focused = false`) —
/// how every CLI-driven session (`plexi pane key/send`, drive-host, the
/// babysitter tester) actually reaches the host. `Response::has_focus` is
/// OS-focus-gated and reads false for the whole blurred session, which is
/// exactly how the first 0545 fix passed the (default-focused) harness while
/// never firing in the real build.
fn run_blurred_frames(h: &mut HostHarness, n: u32) {
    for _ in 0..n {
        h.frame(egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            focused: false,
            ..Default::default()
        });
    }
}

/// Stint 0545: the File Browser rename modal (Cmd+R on a selected entry)
/// must open with the entry name fully selected. Driven through the real
/// KeyPane path so the modal opens exactly as a keystroke opens it, on
/// OS-blurred frames so the test models the CLI-driven session where the
/// first fix silently never fired.
#[test]
fn file_browser_rename_modal_preselects_entry_name() {
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::write(tmp.path().join("alpha.txt"), b"x").expect("seed file");
    let mut h = HostHarness::new();
    h.app.open_builtin_app_pane(
        Box::new(crate::file_browser::FileBrowserApp::new(
            tmp.path().to_path_buf(),
        )),
        AppPermissions::builtin(),
        tmp.path().to_path_buf(),
        None,
        Some("split_h"),
        None,
    );
    let pane_id = h.state().open_panes[0];
    h.run_frames(2);

    let response_file = temp_response(tmp.path(), "rename-key");
    h.inject_ipc(AppRequest::KeyPane {
        pane_id,
        key: "cmd+r".to_string(),
        response_file: Some(response_file.clone()),
    });
    run_blurred_frames(&mut h, 3);
    assert_eq!(read_json_response(&response_file)["ok"], true);

    let field = egui::Id::new(("file_browser_rename_input", pane_id));
    assert_eq!(
        h.ctx.memory(|m| m.focused()),
        Some(field),
        "file browser rename field must hold focus while its modal is open"
    );
    assert_eq!(
        text_selection(&h.ctx, field),
        Some((0, "alpha.txt".chars().count())),
        "file browser rename prefill must be fully selected on open"
    );

    // Complete the tester's repro end-to-end: typed text must REPLACE the
    // selected prefill, and the rename must land in the source's own
    // directory (not the browse/host cwd).
    h.app.handle_pane_ipc_request(AppRequest::SendToPane {
        pane_id,
        text: "QQ".to_string(),
        submit: false,
        response_file: None,
    });
    run_blurred_frames(&mut h, 2);
    let enter_response = temp_response(tmp.path(), "rename-enter");
    h.inject_ipc(AppRequest::KeyPane {
        pane_id,
        key: "enter".to_string(),
        response_file: Some(enter_response.clone()),
    });
    run_blurred_frames(&mut h, 3);
    assert_eq!(read_json_response(&enter_response)["ok"], true);
    assert!(
        tmp.path().join("QQ").exists(),
        "typing over the selected prefill then Enter must rename alpha.txt to QQ in place"
    );
    assert!(
        !tmp.path().join("alpha.txt").exists(),
        "the original file name must be gone after the rename"
    );
    assert!(
        !tmp.path().join("alpha.txtQQ").exists(),
        "typed text must replace the selection, never append to the prefill"
    );
}

/// Stint 0537 (review finding): queued IPC text for a just-focused pane must
/// not be swallowed by a raw-focus widget belonging to the previously focused
/// pane. The unknown-focused-id delivery path only counts for the pane that
/// already owned input on the last reconciled frame.
#[test]
fn send_to_pane_never_routes_text_into_previous_panes_raw_focus_widget() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut h = HostHarness::new();
    h.app.open_builtin_app_pane(
        Box::new(TextInputProbe {
            id_salt: 1,
            ..Default::default()
        }),
        AppPermissions::builtin(),
        tmp.path().to_path_buf(),
        None,
        Some("split_h"),
        None,
    );
    let pane_a = h.state().open_panes[0];
    // Pane A renders, grabs raw egui focus, and owns input for real frames.
    h.run_frames(2);

    h.app.open_builtin_app_pane(
        Box::new(TextInputProbe {
            id_salt: 2,
            ..Default::default()
        }),
        AppPermissions::builtin(),
        tmp.path().to_path_buf(),
        None,
        Some("split_h"),
        None,
    );
    let pane_b = *h
        .state()
        .open_panes
        .iter()
        .find(|id| **id != pane_a)
        .expect("second probe pane");

    // Queue text for B while A's raw widget still holds egui focus.
    let response_file = temp_response(tmp.path(), "send-text-b");
    h.app.handle_pane_ipc_request(AppRequest::SendToPane {
        pane_id: pane_b,
        text: "for-b-only".to_string(),
        submit: false,
        response_file: Some(response_file.clone()),
    });
    h.run_frames(4);

    let probe_text = |h: &HostHarness, pane_id| -> String {
        h.app.windows[0]
            .panes
            .get(&pane_id)
            .and_then(Pane::as_app)
            .and_then(|pane| pane.runtime.serialize_state())
            .expect("probe state")["text"]
            .as_str()
            .expect("text field")
            .to_string()
    };
    assert_eq!(
        probe_text(&h, pane_a),
        "",
        "pane A's raw-focus widget must never receive text queued for pane B"
    );
    assert_eq!(
        probe_text(&h, pane_b),
        "for-b-only",
        "pane B must consume its queued text once it owns input"
    );
}

// -- Routine firing path (stint 0575) ---------------------------------------
//
// Integration coverage for `tick_scheduler` / `fire_routine`: context
// targeting, focus restore, ephemeral teardown, the missing-context skip,
// the 0573 overlap guard, and the persisted-last_fire startup behavior.

mod routine_firing {
    use super::*;
    use crate::host::context::{Context, Window};
    use crate::host::pane::{AppPane, AppRuntime, Pane};
    use crate::spatial::tiling::PaneId;

    /// Write `routines.toml` into `root`'s workspace channel dir.
    fn write_routines(root: &std::path::Path, body: &str) {
        let dir = root.join(crate::config::workspace_channel_dir());
        std::fs::create_dir_all(&dir).expect("create channel dir");
        std::fs::write(dir.join("routines.toml"), body).expect("write routines.toml");
    }

    /// Point the harness's default context at `root` so `tick_scheduler`
    /// loads routines from it, and give it a stable name.
    fn root_default_context(h: &mut HostHarness, root: &std::path::Path) {
        let ctx = h.app.router.get_mut(0);
        ctx.root = Some(root.to_path_buf());
        ctx.name = "main".to_string();
    }

    /// Insert a builtin app pane (rooted at `root`, never a real machine dir)
    /// into `win_idx` and focus it, so `split_focused` has a split target.
    fn add_focused_app_pane(h: &mut HostHarness, win_idx: usize, root: &std::path::Path) -> PaneId {
        let pane_id = h.app.host.alloc_pane_id();
        let pane = AppPane {
            pip_status: None,
            id: pane_id,
            runtime: AppRuntime::Builtin(Box::new(crate::file_browser::FileBrowserApp::new(
                root.to_path_buf(),
            ))),
            workspace_root: root.to_path_buf(),
            permissions: crate::app::permissions::AppPermissions::builtin(),
            manifest_id: "test".to_string(),
            name: "Test App".to_string(),
            pane_group: None,
            linked_pane_id: None,
            overlay_replaced: None,
            hidden: false,
            agent: None,
            slots: std::collections::HashMap::new(),
            semantic_state: Default::default(),
        };
        let win = &mut h.app.windows[win_idx];
        win.panes.insert(pane_id, Pane::App(Box::new(pane)));
        let tile_id = win.tree.tiles.insert_pane(pane_id);
        if win.tree.root.is_none() {
            win.tree.root = Some(tile_id);
        }
        win.focused_pane = Some(tile_id);
        pane_id
    }

    /// Add a second context named `name` with its own window and a focused
    /// pane, without going through the PTY-seeding production path. Returns
    /// the new window's index.
    fn add_secondary_context(h: &mut HostHarness, name: &str, root: &std::path::Path) -> usize {
        let ctx_id = h.app.next_window_id;
        h.app.next_window_id += 1;
        let win_id = h.app.next_window_id;
        h.app.next_window_id += 1;
        h.app.windows.push(Window {
            name: String::new(),
            path: root.to_path_buf(),
            tree: egui_tiles::Tree::empty("plexi"),
            panes: std::collections::HashMap::new(),
            focused_pane: None,
            zoomed_pane: None,
            grid_x: 1,
            grid_y: 0,
            window_id: win_id,
            context_id: ctx_id,
        });
        let win_idx = h.app.windows.len() - 1;
        add_focused_app_pane(h, win_idx, root);
        h.app.router.push(Context {
            name: name.to_string(),
            path: root.to_path_buf(),
            root: None, // no root: tick_scheduler must not load routines from here
            description: None,
            context_id: ctx_id,
            parent_id: None,
            depth: 0,
            parked: false,
        });
        win_idx
    }

    fn terminal_pane_ids(h: &HostHarness, win_idx: usize) -> Vec<PaneId> {
        h.app.windows[win_idx]
            .panes
            .iter()
            .filter_map(|(id, p)| p.as_terminal().map(|_| *id))
            .collect()
    }

    fn routine_notification_count(h: &HostHarness, needle: &str) -> usize {
        h.app
            .pending_notifications
            .iter()
            .filter(|n| n.title.contains(needle))
            .count()
    }

    /// Rewind a routine's last_fire so it is due again without sleeping.
    fn rewind_last_fire(h: &mut HostHarness, name: &str, hours: i64) {
        for e in &mut h.app.scheduler.entries {
            if e.routine.name == name {
                e.last_fire = e
                    .last_fire
                    .map(|lf| lf - chrono::Duration::hours(hours))
                    .or_else(|| Some(chrono::Local::now() - chrono::Duration::hours(hours)));
            }
        }
    }

    #[test]
    fn routine_fires_into_named_context_and_restores_focus() {
        let mut h = HostHarness::new();
        let root = tempfile::tempdir().expect("tempdir");
        root_default_context(&mut h, root.path());
        add_focused_app_pane(&mut h, 0, root.path());
        let work_root = tempfile::tempdir().expect("tempdir");
        let work_win = add_secondary_context(&mut h, "work", work_root.path());

        write_routines(
            root.path(),
            r#"
            [[routine]]
            name = "into-work"
            command = "true"
            schedule = "every 2 hours"
            context = "work"
        "#,
        );

        let active_ctx_before = h.app.router.active_idx();
        let active_win_before = h.app.active_window;
        let main_panes_before = h.app.windows[0].panes.len();
        assert!(terminal_pane_ids(&h, work_win).is_empty());

        h.app.tick_scheduler();

        assert_eq!(
            terminal_pane_ids(&h, work_win).len(),
            1,
            "routine pane must land in the named context's window"
        );
        assert_eq!(
            h.app.windows[0].panes.len(),
            main_panes_before,
            "active context must not receive the pane"
        );
        assert_eq!(h.app.router.active_idx(), active_ctx_before);
        assert_eq!(h.app.active_window, active_win_before);
    }

    /// `plexi routine run` parity (stint 0574): a spawn_pane request carrying
    /// `context_name` lands in that context's window without touching the
    /// active window — the same targeting `fire_routine` uses.
    #[test]
    fn spawn_pane_context_name_lands_in_named_context() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut h = HostHarness::new();
        let root = tempfile::tempdir().expect("tempdir");
        root_default_context(&mut h, root.path());
        add_focused_app_pane(&mut h, 0, root.path());
        let work_root = tempfile::tempdir().expect("tempdir");
        let work_win = add_secondary_context(&mut h, "work", work_root.path());

        let active_win_before = h.app.active_window;
        let main_panes_before = h.app.windows[0].panes.len();
        let response = temp_response(tmp.path(), "ctx-name-spawn");
        h.app.handle_pane_ipc_request(AppRequest::SpawnPane {
            type_id: "terminal".to_string(),
            layout: Some("split_h".to_string()),
            args: vec!["true".to_string()],
            from_pane_id: None,
            request_id: None,
            response_file: Some(response.clone()),
            ephemeral: false,
            cwd: None,
            no_focus: false,
            path: None,
            workspace_root: None,
            target_context: None,
            context_name: Some("work".to_string()),
            name: None,
            agent_cmd: None,
            boot_timeout_secs: None,
        });

        let reply = read_json_response(&response);
        let pane_id = reply["pane_id"]
            .as_u64()
            .expect("spawn must return pane_id");
        assert!(
            h.app.windows[work_win].panes.contains_key(&pane_id),
            "pane must land in the named context's window"
        );
        assert_eq!(
            h.app.windows[0].panes.len(),
            main_panes_before,
            "active context must not receive the pane"
        );
        assert_eq!(h.app.active_window, active_win_before);
    }

    /// A `context_name` naming no live context is a hard error back to the
    /// caller — never a silent fallback into the active context.
    #[test]
    fn spawn_pane_unknown_context_name_errors() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut h = HostHarness::new();
        let root = tempfile::tempdir().expect("tempdir");
        root_default_context(&mut h, root.path());
        add_focused_app_pane(&mut h, 0, root.path());
        let panes_before = h.app.windows[0].panes.len();

        let response = temp_response(tmp.path(), "ctx-name-missing");
        h.app.handle_pane_ipc_request(AppRequest::SpawnPane {
            type_id: "terminal".to_string(),
            layout: Some("split_h".to_string()),
            args: vec!["true".to_string()],
            from_pane_id: None,
            request_id: None,
            response_file: Some(response.clone()),
            ephemeral: false,
            cwd: None,
            no_focus: false,
            path: None,
            workspace_root: None,
            target_context: None,
            context_name: Some("ghost".to_string()),
            name: None,
            agent_cmd: None,
            boot_timeout_secs: None,
        });

        let reply = read_json_response(&response);
        let err = reply["error"].as_str().expect("must reply with an error");
        assert!(err.contains("ghost"), "error must name the context: {err}");
        assert_eq!(
            h.app.windows[0].panes.len(),
            panes_before,
            "no pane may be spawned anywhere on a missing context"
        );
    }

    #[test]
    fn routine_empty_context_fires_into_active_context() {
        let mut h = HostHarness::new();
        let root = tempfile::tempdir().expect("tempdir");
        root_default_context(&mut h, root.path());
        add_focused_app_pane(&mut h, 0, root.path());

        write_routines(
            root.path(),
            r#"
            [[routine]]
            name = "here"
            command = "true"
            schedule = "every 2 hours"
        "#,
        );

        h.app.tick_scheduler();
        assert_eq!(
            terminal_pane_ids(&h, 0).len(),
            1,
            "empty context field fires into the active context"
        );
    }

    #[test]
    fn routine_missing_context_skips_notifies_once() {
        let mut h = HostHarness::new();
        h.app.notifications_enabled = true; // harness default is off
        let root = tempfile::tempdir().expect("tempdir");
        root_default_context(&mut h, root.path());
        add_focused_app_pane(&mut h, 0, root.path());

        write_routines(
            root.path(),
            r#"
            [[routine]]
            name = "ghost-run"
            command = "true"
            schedule = "every 2 hours"
            context = "no-such-context"
        "#,
        );

        h.app.tick_scheduler();
        assert!(
            terminal_pane_ids(&h, 0).is_empty(),
            "missing context must not spawn a pane"
        );
        assert_eq!(
            routine_notification_count(&h, "ghost-run"),
            1,
            "missing context surfaces one notification"
        );

        // Still-missing on the next due tick: no duplicate notification.
        rewind_last_fire(&mut h, "ghost-run", 3);
        h.app.tick_scheduler();
        assert!(terminal_pane_ids(&h, 0).is_empty());
        assert_eq!(
            routine_notification_count(&h, "ghost-run"),
            1,
            "repeated observation of the same failure must not re-notify"
        );
    }

    #[test]
    fn startup_does_not_stampede_with_persisted_last_fire() {
        let mut h = HostHarness::new();
        let root = tempfile::tempdir().expect("tempdir");
        root_default_context(&mut h, root.path());
        add_focused_app_pane(&mut h, 0, root.path());

        write_routines(
            root.path(),
            r#"
            [[routine]]
            name = "sync"
            command = "true"
            schedule = "every 2 hours"
        "#,
        );
        let state = format!(
            "[last_fire]\nsync = \"{}\"\n",
            chrono::Local::now().to_rfc3339()
        );
        std::fs::write(
            root.path()
                .join(crate::config::workspace_channel_dir())
                .join("routine-state.toml"),
            state,
        )
        .expect("write state");

        h.app.tick_scheduler();
        assert!(
            terminal_pane_ids(&h, 0).is_empty(),
            "a last_fire inside the interval must suppress the startup fire"
        );
    }

    #[test]
    fn startup_fires_when_persisted_last_fire_is_stale_or_absent() {
        // Stale persisted state → fires once.
        let mut h = HostHarness::new();
        let root = tempfile::tempdir().expect("tempdir");
        root_default_context(&mut h, root.path());
        add_focused_app_pane(&mut h, 0, root.path());
        write_routines(
            root.path(),
            r#"
            [[routine]]
            name = "sync"
            command = "true"
            schedule = "every 2 hours"
        "#,
        );
        let stale = chrono::Local::now() - chrono::Duration::hours(5);
        std::fs::write(
            root.path()
                .join(crate::config::workspace_channel_dir())
                .join("routine-state.toml"),
            format!("[last_fire]\nsync = \"{}\"\n", stale.to_rfc3339()),
        )
        .expect("write state");
        h.app.tick_scheduler();
        assert_eq!(
            terminal_pane_ids(&h, 0).len(),
            1,
            "an overdue routine fires once on launch"
        );

        // No state at all → fires (today's first-run behavior).
        let mut h2 = HostHarness::new();
        let root2 = tempfile::tempdir().expect("tempdir");
        root_default_context(&mut h2, root2.path());
        add_focused_app_pane(&mut h2, 0, root2.path());
        write_routines(
            root2.path(),
            r#"
            [[routine]]
            name = "sync"
            command = "true"
            schedule = "every 2 hours"
        "#,
        );
        h2.app.tick_scheduler();
        assert_eq!(terminal_pane_ids(&h2, 0).len(), 1);
    }

    #[test]
    fn overlap_guard_skips_streaks_and_reaps_on_close() {
        let mut h = HostHarness::new();
        h.app.notifications_enabled = true; // harness default is off
        let root = tempfile::tempdir().expect("tempdir");
        root_default_context(&mut h, root.path());
        add_focused_app_pane(&mut h, 0, root.path());

        write_routines(
            root.path(),
            r#"
            [[routine]]
            name = "long-job"
            command = "true"
            schedule = "every 2 hours"
        "#,
        );

        h.app.tick_scheduler();
        let run_panes = terminal_pane_ids(&h, 0);
        assert_eq!(run_panes.len(), 1, "first tick fires");
        let run_pane = run_panes[0];
        assert_eq!(h.app.scheduler.live_run_count(), 1);

        // No state is faked here: a freshly spawned pane whose PTY child has
        // not exited IS a live run — liveness is pane-existence + !exited,
        // never the `activity` sampler (tester13, PR #2508: sampling read
        // exec-optimised ephemeral runs as finished and stacked panes).
        let last_fire_before_skip = h.app.scheduler.entries[0].last_fire;
        rewind_last_fire(&mut h, "long-job", 3);
        let rewound = h.app.scheduler.entries[0].last_fire;
        assert_ne!(last_fire_before_skip, rewound);

        h.app.tick_scheduler();
        assert_eq!(
            terminal_pane_ids(&h, 0).len(),
            1,
            "overlapping run must be skipped, not stacked"
        );
        assert_eq!(
            h.app.scheduler.entries[0].last_fire, rewound,
            "a skipped tick must NOT stamp last_fire"
        );
        assert_eq!(
            routine_notification_count(&h, "long-job"),
            1,
            "one notification per skip streak"
        );

        // Second skip in the same streak: no new notification.
        h.app.tick_scheduler();
        assert_eq!(routine_notification_count(&h, "long-job"), 1);

        // Close the run's pane → the reap hook must free the routine.
        h.app.close_pane_by_id(run_pane);
        assert_eq!(
            h.app.scheduler.live_run_count(),
            0,
            "closing the pane must reap the live-run record"
        );

        // Still due (last_fire untouched) → fires again on the next tick.
        h.app.tick_scheduler();
        assert_eq!(
            terminal_pane_ids(&h, 0).len(),
            1,
            "routine fires again after the previous run is reaped"
        );

        // A new run means a new streak: a fresh skip notifies again.
        rewind_last_fire(&mut h, "long-job", 3);
        h.app.tick_scheduler();
        assert_eq!(
            routine_notification_count(&h, "long-job"),
            2,
            "a new skip streak notifies once more"
        );
    }

    /// Regression guard for tester13's Bug 1 on PR #2508, at the exact spawn
    /// shape that escaped the original 0575 suite: `ephemeral = true` passes
    /// the bare command to the shell, zsh exec-optimises `zsh -c "sleep 30"`
    /// into the sleep itself (no children, fg pgid == shell pid), so the
    /// `activity` sampler never leaves `None`. Liveness keyed on that sampler
    /// read the run as finished and stacked a pane every tick. Liveness must
    /// come from pane-existence + !exited, so this pane — activity untouched,
    /// exactly as a real idle-sampled host would see it — blocks the refire.
    #[test]
    fn ephemeral_bare_command_run_blocks_refire_without_activity_sample() {
        let mut h = HostHarness::new();
        let root = tempfile::tempdir().expect("tempdir");
        root_default_context(&mut h, root.path());
        add_focused_app_pane(&mut h, 0, root.path());

        write_routines(
            root.path(),
            r#"
            [[routine]]
            name = "eph"
            command = "sleep 30"
            schedule = "every 2 hours"
            ephemeral = true
        "#,
        );

        h.app.tick_scheduler();
        let panes = terminal_pane_ids(&h, 0);
        assert_eq!(panes.len(), 1, "first tick fires");
        let run_pane = panes[0];
        assert_eq!(
            h.app.windows[0]
                .panes
                .get(&run_pane)
                .and_then(|p| p.as_terminal())
                .expect("terminal")
                .activity,
            None,
            "precondition: the sampler has never seen this pane — the shape the bug lived in"
        );

        rewind_last_fire(&mut h, "eph", 3);
        h.app.tick_scheduler();
        assert_eq!(
            terminal_pane_ids(&h, 0).len(),
            1,
            "a live ephemeral run with no activity sample must skip, not stack"
        );
        assert_eq!(h.app.scheduler.live_run_count(), 1);
    }

    /// A non-ephemeral pane whose PTY child exited (shows "[process exited]"
    /// but stays open until a keypress) is a finished run: the next due tick
    /// reaps it and fires a fresh pane alongside it.
    #[test]
    fn nonephemeral_exited_shell_reaps_and_refires() {
        let mut h = HostHarness::new();
        let root = tempfile::tempdir().expect("tempdir");
        root_default_context(&mut h, root.path());
        add_focused_app_pane(&mut h, 0, root.path());

        write_routines(
            root.path(),
            r#"
            [[routine]]
            name = "keeper"
            command = "true"
            schedule = "every 2 hours"
        "#,
        );

        h.app.tick_scheduler();
        let panes = terminal_pane_ids(&h, 0);
        assert_eq!(panes.len(), 1, "first tick fires");
        let first_pane = panes[0];

        // The field PtyEvent::Exit sets when the PTY child dies.
        h.app.windows[0]
            .panes
            .get_mut(&first_pane)
            .and_then(|p| p.as_terminal_mut())
            .expect("terminal")
            .exited = true;

        rewind_last_fire(&mut h, "keeper", 3);
        h.app.tick_scheduler();
        let panes = terminal_pane_ids(&h, 0);
        assert_eq!(
            panes.len(),
            2,
            "an exited run must be reaped and the routine refired"
        );
        assert_eq!(h.app.scheduler.live_run_count(), 1, "only the new run is registered");
        let new_run = h
            .app
            .scheduler
            .live_run(root.path(), "keeper")
            .expect("new live run")
            .pane_id;
        assert_ne!(new_run, first_pane, "the live run is the fresh pane");
    }

    #[test]
    fn ephemeral_routine_pane_closes_on_exit_and_nonephemeral_stays() {
        let mut h = HostHarness::new();
        let root = tempfile::tempdir().expect("tempdir");
        root_default_context(&mut h, root.path());
        add_focused_app_pane(&mut h, 0, root.path());

        write_routines(
            root.path(),
            r#"
            [[routine]]
            name = "flash"
            command = "true"
            schedule = "every 2 hours"
            ephemeral = true

            [[routine]]
            name = "keeper"
            command = "true"
            schedule = "every 2 hours"
        "#,
        );

        h.app.tick_scheduler();
        assert_eq!(terminal_pane_ids(&h, 0).len(), 2, "both routines fire");

        // The ephemeral pane closes when `true` exits; the other stays. PTY
        // exit events are drained every frame, so pump frames until settled.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while terminal_pane_ids(&h, 0).len() > 1 && std::time::Instant::now() < deadline {
            h.run_frames(1);
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert_eq!(
            terminal_pane_ids(&h, 0).len(),
            1,
            "ephemeral routine pane must close when its command exits"
        );
        // Closing the ephemeral pane also reaped its live-run record.
        assert!(
            h.app.scheduler.live_run_count() <= 1,
            "ephemeral run must be reaped on close"
        );
    }
}

// -- `pane new --agent`: spawn replies are held until the agent boots -------

mod agent_boot {
    use super::*;
    use crate::app_protocol::{AgentState, AppRequest};

    /// Spawn a terminal with `--agent` semantics and return its response file.
    fn spawn_agent_pane(
        h: &mut HostHarness,
        dir: &std::path::Path,
        agent_cmd: &str,
        boot_timeout_secs: Option<f64>,
    ) -> String {
        let response = temp_response(dir, "agent-spawn");
        h.inject_ipc(AppRequest::SpawnPane {
            type_id: "terminal".to_string(),
            layout: Some("split_h".to_string()),
            args: vec![],
            from_pane_id: None,
            request_id: None,
            response_file: Some(response.clone()),
            ephemeral: false,
            cwd: Some(dir.to_string_lossy().into_owned()),
            no_focus: false,
            path: None,
            workspace_root: None,
            target_context: None,
            context_name: None,
            name: Some("agent".to_string()),
            agent_cmd: Some(agent_cmd.to_string()),
            boot_timeout_secs,
        });
        response
    }

    /// The id of the terminal the spawn just created. The spawn response is
    /// deliberately withheld, so tests cannot read the id back from it.
    fn spawned_terminal(h: &HostHarness) -> u64 {
        h.app
            .windows
            .iter()
            .flat_map(|win| win.panes.iter())
            .find(|(_, pane)| pane.as_terminal().is_some())
            .map(|(id, _)| *id)
            .expect("spawn must have created a terminal pane")
    }

    fn report_agent(h: &mut HostHarness, pane_id: u64, state: AgentState) {
        h.inject_ipc(AppRequest::SetAgentState {
            pane_id,
            state,
            agent: "claude-code".to_string(),
            detail: None,
            session_id: None,
        });
    }

    /// The core contract: a pane id on stdout means "ready for a brief". The
    /// host must therefore withhold the spawn reply through every state that
    /// is not the agent reporting itself idle.
    #[test]
    fn agent_spawn_defers_reply_until_agent_reports_idle() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut h = HostHarness::new();
        h.app.set_context_root(tmp.path().to_path_buf(), None);
        h.add_test_pane();

        let response = spawn_agent_pane(&mut h, tmp.path(), "true", Some(120.0));
        h.run_frames(1);
        assert!(
            !std::path::Path::new(&response).exists(),
            "the spawn must not answer before the agent has booted"
        );

        let pane_id = spawned_terminal(&h);
        report_agent(&mut h, pane_id, AgentState::Working);
        h.run_frames(1);
        assert!(
            !std::path::Path::new(&response).exists(),
            "a working report is not a boot signal"
        );

        report_agent(&mut h, pane_id, AgentState::Idle);
        h.run_frames(1);
        assert_eq!(
            read_json_response(&response)["pane_id"].as_u64(),
            Some(pane_id),
            "the idle report must release the spawn reply with the real pane id"
        );
    }

    /// The report arrives over IPC, which `App::logic` drains — so an occluded
    /// host, which never runs `App::ui`, must still complete the boot.
    #[test]
    fn agent_spawn_completes_while_window_hidden() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut h = HostHarness::new();
        h.app.set_context_root(tmp.path().to_path_buf(), None);
        h.add_test_pane();

        let response = spawn_agent_pane(&mut h, tmp.path(), "true", Some(120.0));
        h.run_hidden_frames(1);
        let pane_id = spawned_terminal(&h);

        report_agent(&mut h, pane_id, AgentState::Idle);
        h.run_hidden_frames(1);

        assert_eq!(
            read_json_response(&response)["pane_id"].as_u64(),
            Some(pane_id),
            "agent boots must be registered and completed from App::logic"
        );
    }

    /// A boot timeout leaves a live pane behind, so the reply has to name it:
    /// the caller's only way to clean up is the id it never got on stdout.
    #[test]
    fn agent_spawn_boot_timeout_reports_pane_id() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut h = HostHarness::new();
        h.app.set_context_root(tmp.path().to_path_buf(), None);
        h.add_test_pane();

        let response = spawn_agent_pane(&mut h, tmp.path(), "sleep 30", Some(0.05));
        h.run_hidden_frames(1);
        let pane_id = spawned_terminal(&h);

        for _ in 0..40 {
            if std::path::Path::new(&response).exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
            h.run_hidden_frames(1);
        }

        let reply = read_json_response(&response);
        assert_eq!(reply["ok"].as_bool(), Some(false));
        assert_eq!(reply["timeout"].as_bool(), Some(true));
        assert_eq!(reply["pane_id"].as_u64(), Some(pane_id));
        assert!(
            reply["error"].as_str().expect("error").contains("sleep 30"),
            "the reason must name what failed to boot: {reply}"
        );
    }

    /// A pane that closes can never report, so waiting out the full boot
    /// window would strand the caller for nothing.
    #[test]
    fn agent_spawn_errors_when_pane_closes_before_boot() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut h = HostHarness::new();
        h.app.set_context_root(tmp.path().to_path_buf(), None);
        h.add_test_pane();

        // A long boot window: only the close, not the deadline, can end this.
        let response = spawn_agent_pane(&mut h, tmp.path(), "sleep 30", Some(600.0));
        h.run_frames(1);
        let pane_id = spawned_terminal(&h);

        h.inject_ipc(AppRequest::ClosePane { pane_id });
        h.run_frames(1);

        let reply = read_json_response(&response);
        assert_eq!(reply["ok"].as_bool(), Some(false));
        assert_eq!(
            reply["timeout"].as_bool(),
            Some(false),
            "a vanished pane is not a timeout — the caller has nothing to close"
        );
        assert_eq!(reply["pane_id"].as_u64(), Some(pane_id));
        assert!(
            reply["error"].as_str().expect("error").contains("closed"),
            "unexpected error: {reply}"
        );
    }

    /// The alias reaches the pane's shell as typed input, which is what makes
    /// the agent start at all. Proven through the PTY, not through host state.
    #[test]
    fn agent_spawn_types_the_alias_into_the_pane() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut h = HostHarness::new();
        h.app.set_context_root(tmp.path().to_path_buf(), None);
        h.add_test_pane();

        let _response = spawn_agent_pane(&mut h, tmp.path(), "echo plexi-boot-marker", Some(600.0));
        h.run_frames(1);
        let pane_id = spawned_terminal(&h);

        let mut captured = String::new();
        for _ in 0..100 {
            std::thread::sleep(std::time::Duration::from_millis(20));
            h.run_frames(1);
            let capture = temp_response(tmp.path(), "agent-capture");
            h.inject_ipc(AppRequest::CapturePane {
                pane_id,
                lines: 50,
                response_file: capture.clone(),
                full_output: false,
                from_cursor: None,
            });
            h.run_frames(1);
            captured = read_json_response(&capture).to_string();
            if captured.contains("plexi-boot-marker") {
                break;
            }
        }
        assert!(
            captured.contains("plexi-boot-marker"),
            "the alias must be written to the new pane's PTY; captured: {captured}"
        );
    }

    /// An agent is a terminal affordance. Asking for one on an app pane is a
    /// caller mistake and must fail loudly rather than spawn a silent app.
    #[test]
    fn agent_spawn_rejected_for_non_terminal_target() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut h = HostHarness::new();
        h.app.set_context_root(tmp.path().to_path_buf(), None);

        let response = temp_response(tmp.path(), "agent-app-spawn");
        h.inject_ipc(AppRequest::SpawnPane {
            type_id: "text-editor".to_string(),
            layout: Some("split_h".to_string()),
            args: vec![],
            from_pane_id: None,
            request_id: None,
            response_file: Some(response.clone()),
            ephemeral: false,
            cwd: Some(tmp.path().to_string_lossy().into_owned()),
            no_focus: false,
            path: None,
            workspace_root: None,
            target_context: None,
            context_name: None,
            name: None,
            agent_cmd: Some("c-large".to_string()),
            boot_timeout_secs: None,
        });
        h.run_frames(1);

        assert_eq!(
            read_json_response(&response)["error"].as_str(),
            Some("agent_cmd is only supported for terminal spawns")
        );
    }

    // -- host-observed boot detection ---------------------------------------
    //
    // Codex v0.145.0 defers its `session_start` hook until the first user
    // prompt submission, so a freshly booted Codex never self-reports. The
    // shared detector therefore also recognizes a known agent binary in the
    // PTY foreground and derives idle from the output settle. These tests
    // reproduce the real Homebrew distribution shape — a payload binary
    // reached through a `codex` symlink — because macOS names a process
    // exec'd through a symlink after the RESOLVED target, and a fake that
    // guarantees the name match cannot catch identity bugs (PR #2510 round 2).

    /// A quiescent stand-in for a booted agent TUI sitting at its prompt,
    /// shaped like the Homebrew cask: `codex` is a symlink to a payload
    /// binary named `codex-aarch64-apple-darwin` (a copy of `sleep`), and the
    /// pane execs the symlink path.
    fn fake_idle_codex(dir: &std::path::Path) -> String {
        let payload = dir.join("codex-aarch64-apple-darwin");
        std::fs::copy("/bin/sleep", &payload).expect("copy sleep");
        let link = dir.join("codex");
        std::os::unix::fs::symlink(&payload, &link).expect("symlink codex");
        format!("{} 60", link.display())
    }

    fn pane_info(h: &mut HostHarness, dir: &std::path::Path, pane_id: u64) -> serde_json::Value {
        let response = temp_response(dir, "agent-info");
        h.inject_ipc(AppRequest::GetPaneInfo {
            pane_id,
            response_file: response.clone(),
        });
        h.run_frames(1);
        read_json_response(&response)
    }

    /// Pump visible or hidden frames until the response file exists, waiting
    /// generously — the observed path needs real seconds: shell boot, the
    /// typed command, a full settle window, and a 1 Hz detector tick.
    fn pump_until_response(h: &mut HostHarness, response: &str, hidden: bool) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while !std::path::Path::new(response).exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(50));
            if hidden {
                h.run_hidden_frames(1);
            } else {
                h.run_frames(1);
            }
        }
    }

    /// The tester's exact failure: a booted Codex never fires a hook, so the
    /// spawn must complete from the host-observed detector alone.
    #[test]
    fn agent_spawn_completes_from_observed_codex_boot() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut h = HostHarness::new();
        h.app.set_context_root(tmp.path().to_path_buf(), None);
        h.add_test_pane();

        let cmd = fake_idle_codex(tmp.path());
        let response = spawn_agent_pane(&mut h, tmp.path(), &cmd, Some(120.0));
        h.run_frames(1);
        let pane_id = spawned_terminal(&h);

        pump_until_response(&mut h, &response, false);
        assert_eq!(
            read_json_response(&response)["pane_id"].as_u64(),
            Some(pane_id),
            "a settled foreground agent binary must complete the boot without any hook report"
        );

        // The same detector feeds `pane list`/`pane state`: the pane must
        // report the observed agent, not `agent: null`.
        let info = pane_info(&mut h, tmp.path(), pane_id);
        assert_eq!(
            info["agent"]["agent"].as_str(),
            Some("codex"),
            "observed identity must surface on the pane info: {info}"
        );
        assert_eq!(
            info["agent"]["state"].as_str(),
            Some("idle"),
            "a settled agent must read idle: {info}"
        );
    }

    /// The observed path lives in `App::logic` end to end (activity tick,
    /// boot service) — an occluded host must still complete the boot.
    #[test]
    fn agent_spawn_observed_boot_completes_while_window_hidden() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut h = HostHarness::new();
        h.app.set_context_root(tmp.path().to_path_buf(), None);
        h.add_test_pane();

        let cmd = fake_idle_codex(tmp.path());
        let response = spawn_agent_pane(&mut h, tmp.path(), &cmd, Some(120.0));
        h.run_hidden_frames(1);
        let pane_id = spawned_terminal(&h);

        pump_until_response(&mut h, &response, true);
        assert_eq!(
            read_json_response(&response)["pane_id"].as_u64(),
            Some(pane_id),
            "the observed boot path must not depend on App::ui running"
        );
    }

    /// A foreground agent that is still streaming output is booting or busy,
    /// never idle — the settle window is what separates the two. A fake codex
    /// that redraws continuously must ride out the whole boot window.
    #[test]
    fn agent_spawn_streaming_agent_is_not_reported_idle() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut h = HostHarness::new();
        h.app.set_context_root(tmp.path().to_path_buf(), None);
        h.add_test_pane();

        let fake = tmp.path().join("codex");
        std::fs::copy("/bin/zsh", &fake).expect("copy zsh");
        let cmd = format!(
            "{} -c 'while :; do echo streaming-banner; sleep 0.05; done'",
            fake.display()
        );
        let response = spawn_agent_pane(&mut h, tmp.path(), &cmd, Some(4.0));
        h.run_frames(1);
        let pane_id = spawned_terminal(&h);

        pump_until_response(&mut h, &response, false);
        let reply = read_json_response(&response);
        assert_eq!(
            reply["timeout"].as_bool(),
            Some(true),
            "a continuously streaming agent must never satisfy the boot predicate: {reply}"
        );

        h.inject_ipc(AppRequest::ClosePane { pane_id });
        h.run_frames(1);
    }

    /// Hook reports are authoritative: once one lands, it both overrides the
    /// observed state and permanently disables synthesis for the pane.
    #[test]
    fn hook_report_overrides_observed_agent() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut h = HostHarness::new();
        h.app.set_context_root(tmp.path().to_path_buf(), None);
        h.add_test_pane();

        let cmd = fake_idle_codex(tmp.path());
        let response = spawn_agent_pane(&mut h, tmp.path(), &cmd, Some(120.0));
        h.run_frames(1);
        let pane_id = spawned_terminal(&h);
        pump_until_response(&mut h, &response, false);

        report_agent(&mut h, pane_id, AgentState::Working);
        h.run_frames(1);
        let info = pane_info(&mut h, tmp.path(), pane_id);
        assert_eq!(info["agent"]["agent"].as_str(), Some("claude-code"));
        assert_eq!(info["agent"]["state"].as_str(), Some("working"));

        // Ride past the next detector ticks: the settled fake codex is still
        // in the foreground, but synthesis must stay off once hooks own the
        // pane — the hook state must not flap back to observed idle.
        for _ in 0..30 {
            std::thread::sleep(std::time::Duration::from_millis(50));
            h.run_frames(1);
        }
        let info = pane_info(&mut h, tmp.path(), pane_id);
        assert_eq!(
            info["agent"]["agent"].as_str(),
            Some("claude-code"),
            "a hook report must permanently supersede the observed agent: {info}"
        );
        assert_eq!(info["agent"]["state"].as_str(), Some("working"));
    }
}

/// `plexi pane send --submit` (stint 0583): the host owns the whole
/// type → settle → Enter → confirm sequence, so an agent driving a TUI pane
/// gets one verb with an exit code it can branch on instead of a sleep and a
/// hope. These run against a real PTY because the signals under test are real
/// PTY signals: the quiet window comes from the pane's event stream, and the
/// confirmation comes from the rendered grid.
#[cfg(test)]
mod pane_send_submit_tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// How long a test will wait for a submit to resolve. Comfortably above the
    /// host's own ceiling (settle + two confirm windows) so a failure here means
    /// the host never answered, not that the test was impatient.
    const RESOLVE_TIMEOUT: Duration = Duration::from_secs(25);

    fn last_pty_output_at(h: &HostHarness, pane: PaneId) -> Option<Instant> {
        h.app.windows[0]
            .panes
            .get(&pane)
            .and_then(Pane::as_terminal)
            .and_then(|term| term.last_pty_output_at)
    }

    fn tail(h: &mut HostHarness, pane: PaneId) -> Vec<String> {
        h.terminal_backend(pane).capture_lines(40)
    }

    /// Pump frames until the pane stops producing, so a submit's settle window
    /// measures the text we typed rather than the shell still printing its
    /// prompt. Bails once the pane has produced nothing at all for a while —
    /// a shell that never wrote anything is already as settled as it gets.
    fn settle_terminal(h: &mut HostHarness, pane: PaneId) {
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(10) {
            h.run_frames(1);
            match last_pty_output_at(h, pane) {
                Some(at) if at.elapsed() >= Duration::from_millis(600) => return,
                None if started.elapsed() >= Duration::from_secs(2) => return,
                _ => {}
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("pane {pane} never stopped producing PTY output");
    }

    /// Pump frames until the pane's tail contains `needle`.
    fn wait_for_output(h: &mut HostHarness, pane: PaneId, needle: &str) {
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(10) {
            h.run_frames(1);
            if tail(h, pane).iter().any(|line| line.contains(needle)) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!(
            "pane {pane} never printed {needle:?}; tail was {:?}",
            tail(h, pane)
        );
    }

    /// Pump frames until the host writes `path`. Frames are the clock: nothing
    /// the host owes is answered outside a pass, so a fixed sleep would only
    /// make this slower without making it more reliable. `hidden` runs
    /// logic-only passes, the ones an occluded host is limited to.
    fn resolve(h: &mut HostHarness, path: &str, hidden: bool) -> serde_json::Value {
        let started = Instant::now();
        while started.elapsed() < RESOLVE_TIMEOUT {
            if hidden {
                h.run_hidden_frames(1);
            } else {
                h.run_frames(1);
            }
            // `write_response` renames into place, so existence means complete.
            if std::path::Path::new(path).exists() {
                return read_json_response(path);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("submit at {path} was never answered within {RESOLVE_TIMEOUT:?}");
    }

    fn submit(h: &HostHarness, pane: PaneId, text: &str, response_file: Option<String>) {
        h.inject_ipc(AppRequest::SendToPane {
            pane_id: pane,
            text: text.to_string(),
            submit: true,
            response_file,
        });
    }

    fn type_only(h: &HostHarness, pane: PaneId, text: &str, response_file: Option<String>) {
        h.inject_ipc(AppRequest::SendToPane {
            pane_id: pane,
            text: text.to_string(),
            submit: false,
            response_file,
        });
    }

    /// The happy path end to end: the command runs, the host confirms it, and
    /// exactly one Enter reaches the PTY. The byte-level assertion is the point
    /// — a second Enter on a shell pane is a blank command, but on an agent TUI
    /// it is a second turn.
    #[test]
    fn pane_send_submit_confirms_real_shell_command() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut h = HostHarness::new();
        let term = h.add_focused_terminal();
        settle_terminal(&mut h, term);

        h.terminal_backend(term).enable_input_tap();
        let response = temp_response(tmp.path(), "submit-confirm");
        submit(&h, term, "echo submit-proof", Some(response.clone()));

        let reply = resolve(&mut h, &response, false);
        assert_eq!(reply["ok"].as_bool(), Some(true), "reply was {reply}");
        assert_eq!(
            reply["retried"].as_bool(),
            Some(false),
            "a shell that submits on the first Enter must not report a retry: {reply}"
        );

        let written = h.terminal_backend(term).take_input_tap();
        assert_eq!(
            written,
            b"echo submit-proof\r",
            "the text must be followed by exactly one Enter, got {:?}",
            String::from_utf8_lossy(&written)
        );
        wait_for_output(&mut h, term, "submit-proof");
    }

    /// The collapsed-paste heal, bounded. A pane whose tail keeps showing the
    /// hint gets exactly one extra Enter — never a second retry, never a loop —
    /// and then reports the submission unconfirmed with what it observed.
    #[test]
    fn pane_send_submit_retries_exactly_once_on_collapsed_paste_hint() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut h = HostHarness::new();
        let term = h.add_focused_terminal();
        settle_terminal(&mut h, term);

        // Park the hint on screen for good. This is what an agent TUI holding a
        // collapsed paste looks like to the host: the marker never leaves the
        // tail no matter how many enters arrive.
        type_only(&h, term, "echo paste again to expand\n", None);
        wait_for_output(&mut h, term, "paste again to expand");
        settle_terminal(&mut h, term);

        h.terminal_backend(term).enable_input_tap();
        let response = temp_response(tmp.path(), "submit-collapsed");
        submit(&h, term, "echo second", Some(response.clone()));

        let reply = resolve(&mut h, &response, false);
        assert!(
            reply["ok"].as_bool().is_none(),
            "a tail that never loses the collapsed-paste hint must not confirm: {reply}"
        );
        assert_eq!(
            reply["retried"].as_bool(),
            Some(true),
            "the one retry must be reported: {reply}"
        );
        assert!(
            !reply["input_line"]
                .as_array()
                .expect("unconfirmed reply must carry input_line")
                .is_empty(),
            "the caller needs the observed input line to branch on: {reply}"
        );

        let written = h.terminal_backend(term).take_input_tap();
        let enters = written.iter().filter(|byte| **byte == b'\r').count();
        assert_eq!(
            enters,
            2,
            "the submit plus exactly one retry is two enters, never three; got {:?}",
            String::from_utf8_lossy(&written)
        );
        assert_eq!(written, b"echo second\r\r");
    }

    /// A pane that shows nothing at all after Enter is the unconfirmed case,
    /// and the caller is told what the input line actually held. `stty -echo`
    /// plus a foreground `cat` gives a pane that swallows both the text and the
    /// Enter without redrawing, which is precisely "typed but not submitted".
    #[test]
    fn pane_send_submit_unconfirmed_reports_observed_input_line() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut h = HostHarness::new();
        let term = h.add_focused_terminal();
        settle_terminal(&mut h, term);

        type_only(&h, term, "stty -echo; cat > /dev/null\n", None);
        wait_for_output(&mut h, term, "cat > /dev/null");
        settle_terminal(&mut h, term);
        // The same read the host confirms against, so the assertion below is
        // about the host's own view rather than a parallel reimplementation.
        let before = h.app.pane_tail(term);

        let response = temp_response(tmp.path(), "submit-unconfirmed");
        submit(&h, term, "never-submitted", Some(response.clone()));

        let reply = resolve(&mut h, &response, false);
        assert!(
            reply["ok"].as_bool().is_none(),
            "an unchanged tail must not confirm: {reply}"
        );
        assert_eq!(
            reply["retried"].as_bool(),
            Some(false),
            "the retry is scoped to the collapsed-paste hint, which never appeared: {reply}"
        );
        let observed: Vec<String> = reply["input_line"]
            .as_array()
            .expect("unconfirmed reply must carry input_line")
            .iter()
            .map(|line| line.as_str().expect("input_line entries are strings").to_string())
            .collect();
        assert!(
            !observed.is_empty() && before.ends_with(observed.as_slice()),
            "input_line must be the pane's own observed tail; observed={observed:?} tail={before:?}"
        );
    }

    /// `--submit` is a terminal-pane verb. An app pane has no input line to
    /// confirm, so it is refused with a typed error rather than silently
    /// degrading to a plain text injection.
    #[test]
    fn pane_send_submit_rejects_app_pane() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut h = HostHarness::new();
        let pane = h.add_test_pane();

        let response = temp_response(tmp.path(), "submit-app-pane");
        submit(&h, pane, "hello", Some(response.clone()));
        h.run_frames(1);

        assert_eq!(
            read_json_response(&response)["error"].as_str(),
            Some(
                format!("pane {pane} is an app pane; --submit only applies to terminal panes")
                    .as_str()
            )
        );
        assert!(h.app.pending_submits.is_empty());
    }

    /// Two submits racing on one input line interleave their enters, so the
    /// second is refused before it types anything.
    #[test]
    fn pane_send_submit_rejects_second_inflight_submit() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut h = HostHarness::new();
        let term = h.add_focused_terminal();
        settle_terminal(&mut h, term);

        let first = temp_response(tmp.path(), "submit-first");
        let second = temp_response(tmp.path(), "submit-second");
        // Injected back to back so both drain in one pass, with the first still
        // parked when the second is handled.
        submit(&h, term, "echo first", Some(first));
        submit(&h, term, "echo second", Some(second.clone()));
        h.run_frames(1);

        assert_eq!(
            read_json_response(&second)["error"].as_str(),
            Some(
                format!("pane {term} already has a `pane send --submit` in flight").as_str()
            ),
            "a second submit must be refused, not queued"
        );
        assert_eq!(h.app.pending_submits.len(), 1);
    }

    /// The occluded-host guard. `App::ui` never runs while the window is hidden,
    /// so a submit serviced from there would type the text and then never press
    /// Enter — the caller blocking until its own timeout with the prompt sitting
    /// unsent in the pane.
    #[test]
    fn pane_send_submit_completes_while_window_hidden() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut h = HostHarness::new();
        let term = h.add_focused_terminal();
        settle_terminal(&mut h, term);

        let response = temp_response(tmp.path(), "submit-hidden");
        submit(&h, term, "echo hidden-proof", Some(response.clone()));

        let reply = resolve(&mut h, &response, true);
        assert_eq!(
            reply["ok"].as_bool(),
            Some(true),
            "submits must settle, submit and confirm on logic-only passes: {reply}"
        );
    }

    /// The default verb is unchanged: no Enter, no waiting, an answer on the
    /// same pass.
    #[test]
    fn pane_send_without_submit_unchanged() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut h = HostHarness::new();
        let term = h.add_focused_terminal();
        settle_terminal(&mut h, term);

        h.terminal_backend(term).enable_input_tap();
        let response = temp_response(tmp.path(), "send-plain");
        type_only(&h, term, "echo plain", Some(response.clone()));
        h.run_frames(1);

        assert_eq!(read_json_response(&response)["ok"].as_bool(), Some(true));
        assert!(
            h.app.pending_submits.is_empty(),
            "a plain send must never park a reply"
        );
        assert_eq!(
            h.terminal_backend(term).take_input_tap(),
            b"echo plain",
            "a plain send must type the text and nothing else"
        );
    }
}

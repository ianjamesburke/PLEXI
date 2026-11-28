//! Stint 1538: Dock Quit, a quit Apple event, and logout, restart or
//! shutdown take the Cmd+Q decision. The requests are posted to the same
//! inbox `applicationShouldTerminate:` posts to, and serviced on hidden
//! (logic-only) passes, since the window may be hidden when they arrive.

use crate::app::quit::ExitReason;
use crate::platform::system_quit::{take_cancels, SystemQuitReason};
use plexi_host_model::model::PaneId;
use crate::testing::HostHarness;
use std::path::Path;

fn unsaved_editor() -> (
    tempfile::TempDir,
    std::path::PathBuf,
    HostHarness,
    plexi_host_model::model::PaneId,
) {
    let dir = tempfile::tempdir().unwrap();
    let mut h = HostHarness::new();
    let path = dir.path().join("document.txt");
    let pane = open_unsaved_editor(&mut h, &path);
    (dir, path, h, pane)
}

fn open_unsaved_editor(h: &mut HostHarness, path: &Path) -> PaneId {
    let pane = open_editor(h, path);
    h.text_editor_mut(pane).test_set_unsaved("unsaved edits");
    pane
}

fn open_editor(h: &mut HostHarness, path: &Path) -> PaneId {
    std::fs::write(path, b"original bytes\n").unwrap();
    let existing_panes = h.state().open_panes;
    h.app.open_builtin_view_pane(
        Box::new(crate::app::text_editor_view::TextEditorView::new(
            path.to_path_buf(),
        )),
        crate::app::permissions::PluginPermissions::builtin(),
        path.parent().unwrap().to_path_buf(),
        None,
        Some("split_h"),
        None,
    );
    let pane = h
        .state()
        .open_panes
        .into_iter()
        .find(|id| !existing_panes.contains(id))
        .expect("the editor opens a new pane");
    h.focus_pane(pane);
    h.run_frames(2);
    pane
}

fn inject_host_stop(h: &HostHarness, reply: &Path) {
    h.inject_ipc(plexi_protocol::HostRequest::Shutdown {
        response_file: Some(reply.to_str().unwrap().to_owned()),
    });
}

fn read_stop_reply(path: &Path) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

#[test]
fn host_stop_preserves_unsaved_editor_text() {
    let root = tempfile::tempdir().unwrap();
    let profile = root.path().join("profile");
    let document = root.path().join("document.txt");
    let reply_path = root.path().join("host-stop.json");
    let duplicate_reply = root.path().join("host-stop-duplicate.json");
    let mut h = HostHarness::from_profile_dir(&profile);
    let pane = open_unsaved_editor(&mut h, &document);
    assert!(h.text_editor_mut(pane).open_buffer().dirty);

    inject_host_stop(&h, &reply_path);
    inject_host_stop(&h, &duplicate_reply);
    h.hidden_frame();

    assert_eq!(
        std::fs::read_to_string(&document).unwrap(),
        "original bytes\n"
    );
    if h.app.quit_coordinator.is_active() {
        // The first hidden pass saves the snapshot. The real exit skips Drop,
        // so suppress the editor's drop-time save before reloading the snapshot.
        h.text_editor_mut(pane).test_discard_unsaved();
        drop(h);
        let mut restarted = HostHarness::from_profile_dir(&profile);
        let restored = restarted.text_editor_mut(pane).open_buffer();
        assert_eq!(restored.text, "unsaved edits", "host stop lost the edit");
        assert!(restored.dirty);
    } else {
        let reply = read_stop_reply(&reply_path);
        assert_eq!(reply["ok"], false);
        assert_eq!(reply["error_code"], "unsaved_editor_changes");
        assert!(reply["error"].as_str().unwrap().contains("document.txt"));
        assert_eq!(
            read_stop_reply(&duplicate_reply)["error_code"],
            "host_stop_in_progress"
        );
        let editor = h.text_editor_mut(pane);
        let buffer = editor.open_buffer();
        assert_eq!(buffer.text, "unsaved edits");
        assert!(buffer.dirty);
        assert!(crate::app::view_trait::View::unsaved_changes_dialog(editor).is_none());
        assert!(!h.app.pending_close);

        h.press_key(egui::Key::S, egui::Modifiers::COMMAND);
        assert_eq!(std::fs::read_to_string(document).unwrap(), "unsaved edits");
        inject_host_stop(&h, &root.path().join("host-stop-after-save.json"));
        h.hidden_frame();
        assert_eq!(h.app.quit_coordinator.reason, Some(ExitReason::HostStop));
        assert!(h.app.quit_coordinator.is_active());
    }
}

#[test]
fn restart_shutdown_refusal_names_every_unsaved_editor() {
    let (root, document, mut h, first) = unsaved_editor();
    let notes = root.path().join("notes.txt");
    let second = open_unsaved_editor(&mut h, &notes);
    let reply_path = root.path().join("restart-stop.json");
    h.inject_ipc(plexi_protocol::HostRequest::ShutdownForRestart {
        response_file: reply_path.to_str().unwrap().to_owned(),
    });
    h.hidden_frame();

    let reply = read_stop_reply(&reply_path);
    assert_eq!(reply["error_code"], "unsaved_editor_changes");
    let message = reply["error"].as_str().unwrap();
    assert!(message.contains("Save or discard"), "{message}");
    for (path, pane) in [(document, first), (notes, second)] {
        assert!(message.contains(path.to_str().unwrap()), "{message}");
        assert!(h.text_editor_mut(pane).open_buffer().dirty);
    }
    assert!(!h.app.quit_coordinator.is_active());
}

#[test]
fn host_stop_refuses_an_edit_made_during_workspace_save_retry() {
    let mut h = HostHarness::new();
    let document = h.profile_dir().join("document.txt");
    let pane = open_editor(&mut h, &document);
    let workspace = crate::workspace::workspace_path_in(h.profile_dir());
    std::fs::create_dir_all(workspace.parent().unwrap()).unwrap();
    std::fs::write(&workspace, b"invalid workspace").unwrap();
    let reply_path = h.profile_dir().join("host-stop.json");
    inject_host_stop(&h, &reply_path);
    h.hidden_frame();
    assert!(h.app.quit_coordinator.retry_pending());

    h.text_editor_mut(pane)
        .test_set_unsaved("edit during retry");
    assert!(!h.app.prepare_user_quit());
    assert!(h.app.quit_coordinator.retry_pending());
    std::fs::remove_file(workspace).unwrap();
    h.app.quit_coordinator.make_retry_due();
    h.hidden_frame();

    assert!(!h.app.quit_coordinator.is_active());
    assert!(!h.app.pending_close);
    assert_eq!(
        read_stop_reply(&reply_path)["error_code"],
        "unsaved_editor_changes"
    );
    let buffer = h.text_editor_mut(pane).open_buffer();
    assert_eq!(buffer.text, "edit during retry");
    assert!(buffer.dirty);
    assert_eq!(
        std::fs::read_to_string(document).unwrap(),
        "original bytes\n"
    );
}

fn tap_cmd_q(h: &mut HostHarness) {
    h.frame(crate::app::key_str_to_egui_raw_input("cmd+q").expect("Cmd+Q maps to key events"));
}

fn cmd_q(h: &mut HostHarness) {
    tap_cmd_q(h);
    assert!(!h.app.quit_coordinator.is_active());
    h.run_frames(2);
    assert_unsaved_prompt(h);
}

fn assert_unsaved_prompt(h: &HostHarness) {
    assert_eq!(
        h.app.focus_stack.top().as_ref().map(|k| k.name()),
        Some("UnsavedChanges")
    );
    assert!(!h.app.quit_coordinator.blocks_input());
}

fn discard(h: &mut HostHarness) {
    h.app.arm_destructive_dialogs_for_test();
    h.press_key(egui::Key::D, egui::Modifiers::NONE);
}

fn confirm_cmd_q(h: &mut HostHarness) {
    assert!(!h.app.quit_coordinator.is_active());
    assert_eq!(h.app.quit_press_count, 1);
    tap_cmd_q(h);
    assert!(!h.app.quit_coordinator.is_active());
    assert_eq!(h.app.quit_press_count, 2);
    tap_cmd_q(h);
    assert!(h.app.quit_coordinator.is_active());
}

/// The editor answer and the confirmed Cmd+Q must reach the process exit through App::logic.
#[test]
fn cmd_q_unsaved_answers_resume_quit() {
    const CHILD: &str = "PLEXI_TEST_CMD_Q_UNSAVED_CHILD";
    const ROOT: &str = "PLEXI_TEST_CMD_Q_UNSAVED_ROOT";
    if let Ok(case) = std::env::var(CHILD) {
        let root = std::path::PathBuf::from(std::env::var_os(ROOT).unwrap());
        let mut h = HostHarness::from_profile_dir(root.join("profile"));
        let document = root.join("document.txt");
        let first = open_unsaved_editor(&mut h, &document);
        let other = root.join("other.txt");
        let second = (case == "two").then(|| open_unsaved_editor(&mut h, &other));

        cmd_q(&mut h);
        let mut saved = None;
        if case == "discard" {
            discard(&mut h);
        } else {
            saved = h.app.focused_pane_id();
            h.press_key(egui::Key::S, egui::Modifiers::COMMAND);
        }
        if let Some(second) = second {
            h.run_frames(2);
            assert_unsaved_prompt(&h);
            let next = if saved == Some(first) { second } else { first };
            assert_eq!(h.app.focused_pane_id(), Some(next));
            discard(&mut h);
        }
        confirm_cmd_q(&mut h);
        assert_eq!(
            h.app.quit_coordinator.reason,
            Some(ExitReason::WindowClosed)
        );
        assert!(h.app.windows[0].panes.contains_key(&first));
        if let Some(second) = second {
            assert!(h.app.windows[0].panes.contains_key(&second));
        }
        for (pane, path) in [(Some(first), document), (second, other)] {
            if pane.is_some() {
                let expected = if pane == saved {
                    "unsaved edits"
                } else {
                    "original bytes\n"
                };
                assert_eq!(std::fs::read_to_string(path).unwrap(), expected);
            }
        }
        h.hidden_frame().hidden_frame();
        panic!("the editor answer never reached exit_host");
    }

    let mut failures = Vec::new();
    for case in ["save", "discard", "two"] {
        let root = tempfile::tempdir().unwrap();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "app::system_quit_tests::cmd_q_unsaved_answers_resume_quit",
                "--nocapture",
            ])
            .env(CHILD, case)
            .env(ROOT, root.path())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now()
            + crate::testing::load_aware_timeout(std::time::Duration::from_secs(20));
        while child.try_wait().unwrap().is_none() {
            if std::time::Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("{case}: the quit hung");
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let output = child.wait_with_output().unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !output.status.success() || !stderr.contains("phase=exit_call") {
            failures.push(format!("{case}: {:?}\n{stderr}", output.status));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn cmd_q_unsaved_cancel_aborts_with_one_or_two_editors() {
    for (two, accept_first) in [(false, false), (true, false), (true, true)] {
        let (dir, _path, mut h, first) = unsaved_editor();
        let second = two.then(|| open_unsaved_editor(&mut h, &dir.path().join("other.txt")));
        cmd_q(&mut h);
        let discarded = h.app.focused_pane_id().filter(|_| accept_first);
        if let Some(pane) = discarded {
            discard(&mut h);
            assert!(!h.app.quit_coordinator.is_active());
            h.run_frames(2);
            assert_unsaved_prompt(&h);
            assert_ne!(h.app.focused_pane_id(), Some(pane));
            let restored = h.text_editor_mut(pane).open_buffer();
            assert_eq!(restored.text, "original bytes\n");
            assert!(!restored.dirty);
        }
        let cancelled = h.app.focused_pane_id().unwrap();
        h.press_key(egui::Key::Escape, egui::Modifiers::NONE);
        assert!(!h.app.quitting && !h.app.quit_coordinator.is_active());
        assert_eq!(h.app.quit_coordinator.reason, None);
        h.run_frames(2);
        assert!(h.app.focus_stack.is_empty());
        for pane in [Some(first), second].into_iter().flatten() {
            assert!(h.app.windows[0].panes.contains_key(&pane));
            assert_eq!(
                h.text_editor_mut(pane).open_buffer().dirty,
                Some(pane) != discarded
            );
            h.text_editor_mut(pane).test_discard_unsaved();
        }
        assert_ne!(Some(cancelled), discarded);
    }
}

#[test]
fn cmd_q_unsaved_plain_pane_close_never_quits() {
    let (_dir, path, mut h, pane) = unsaved_editor();
    let other = h.add_test_pane();
    h.focus_pane(pane).run_frames(2);
    h.press_key(egui::Key::W, egui::Modifiers::COMMAND);
    h.run_frames(2);
    assert_unsaved_prompt(&h);
    discard(&mut h);
    h.run_frames(2);
    assert!(!h.app.windows[0].panes.contains_key(&pane));
    assert!(h.app.windows[0].panes.contains_key(&other));
    assert!(!h.app.quit_coordinator.is_active());
    assert_eq!(std::fs::read_to_string(path).unwrap(), "original bytes\n");
}

#[test]
fn cmd_q_unsaved_failed_save_keeps_the_decision_pending() {
    let (_dir, path, mut h, pane) = unsaved_editor();
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    cmd_q(&mut h);
    h.press_key(egui::Key::S, egui::Modifiers::COMMAND);
    assert!(!h.app.quit_coordinator.is_active());
    h.run_frames(2);
    assert_unsaved_prompt(&h);
    assert!(h.text_editor_mut(pane).open_buffer().dirty);
    discard(&mut h);
    confirm_cmd_q(&mut h);
    assert_eq!(
        h.app.quit_coordinator.reason,
        Some(ExitReason::WindowClosed)
    );
    assert!(path.is_dir());
}

#[test]
fn restart_to_update_keeps_its_reason_after_the_editor_answer() {
    let (_dir, path, mut h, _pane) = unsaved_editor();
    h.app
        .update_status
        .apply(crate::host::update_status::UpdateEvent::Ready {
            version: "9.9.9".into(),
        });
    h.app.update_quit_pending = true;
    h.run_frames(1);
    assert!(!h.app.quit_coordinator.is_active());
    h.run_frames(1);
    assert_unsaved_prompt(&h);
    h.press_key(egui::Key::S, egui::Modifiers::COMMAND);
    assert!(
        h.app.quit_coordinator.is_active(),
        "Save did not resume restart"
    );
    assert_eq!(
        h.app.quit_coordinator.reason,
        Some(ExitReason::UpdateRestart)
    );
    assert_eq!(std::fs::read_to_string(path).unwrap(), "unsaved edits");
}

fn post(h: &mut HostHarness, reason: SystemQuitReason) {
    take_cancels();
    h.app.system_quit.tx.send(reason).unwrap();
    h.hidden_frame();
}

/// With unsaved text: the Cmd+Q prompt opens, the system request is
/// cancelled, nothing quits and nothing is lost. Escape keeps the host.
fn asks_about_unsaved_text(reason: SystemQuitReason) {
    let (_dir, path, mut h, pane) = unsaved_editor();
    post(&mut h, reason);
    assert_eq!(
        take_cancels(),
        1,
        "{reason:?}: the system request is cancelled"
    );
    assert!(
        !h.app.quit_coordinator.is_active(),
        "{reason:?}: no quit started"
    );
    h.run_frames(2);
    assert_eq!(
        h.app.focus_stack.top().as_ref().map(|k| k.name()),
        Some("UnsavedChanges"),
        "{reason:?}: the Cmd+Q unsaved-changes prompt is shown"
    );
    h.press_key(egui::Key::Escape, egui::Modifiers::NONE);
    h.run_frames(2);
    assert!(!h.app.quitting && !h.app.quit_coordinator.is_active());
    assert!(h.app.windows[0].panes.contains_key(&pane));
    assert_eq!(h.text_editor_mut(pane).open_buffer().text, "unsaved edits");
    assert!(h.text_editor_mut(pane).open_buffer().dirty);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "original bytes\n");
    assert_eq!(take_cancels(), 0, "{reason:?}: answered exactly once");
    h.text_editor_mut(pane).test_discard_unsaved();
}

#[test]
fn dock_quit_asks_about_unsaved_text() {
    asks_about_unsaved_text(SystemQuitReason::Quit);
}

#[test]
fn logout_asks_about_unsaved_text() {
    asks_about_unsaved_text(SystemQuitReason::LogOut);
}

#[test]
fn restart_and_shutdown_ask_about_unsaved_text() {
    asks_about_unsaved_text(SystemQuitReason::Restart);
    asks_about_unsaved_text(SystemQuitReason::ShutDown);
}

/// Nothing unsaved: no prompt, no cancel; the coordinated quit is accepted
/// and its save and window hide run in the same hidden pass (the next pass
/// would exit the test process through `exit_host`).
#[test]
fn system_quit_with_nothing_unsaved_quits_without_asking() {
    for reason in [
        SystemQuitReason::Quit,
        SystemQuitReason::LogOut,
        SystemQuitReason::ShutDown,
    ] {
        let mut h = HostHarness::new();
        h.add_test_pane();
        post(&mut h, reason);
        assert_eq!(
            take_cancels(),
            0,
            "{reason:?}: the system request is not cancelled"
        );
        assert_eq!(
            h.app.quit_coordinator.reason,
            Some(ExitReason::WindowClosed)
        );
        assert!(
            h.app.quit_coordinator.blocks_input(),
            "{reason:?}: exiting, not waiting"
        );
        assert!(h.app.system_quit.pending);
        assert!(h.app.focus_stack.is_empty(), "{reason:?}: no prompt");
    }
}

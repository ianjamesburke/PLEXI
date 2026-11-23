//! Stint 1538: Dock Quit, a quit Apple event, and logout, restart or
//! shutdown take the Cmd+Q decision. The requests are posted to the same
//! inbox `applicationShouldTerminate:` posts to, and serviced on hidden
//! (logic-only) passes, since the window may be hidden when they arrive.

use crate::app::quit::ExitReason;
use crate::platform::system_quit::{take_cancels, SystemQuitReason};
use crate::testing::HostHarness;

fn unsaved_editor() -> (
    tempfile::TempDir,
    std::path::PathBuf,
    HostHarness,
    crate::spatial::tiling::PaneId,
) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("document.txt");
    std::fs::write(&path, b"original bytes\n").unwrap();
    let mut h = HostHarness::new();
    h.app.open_builtin_view_pane(
        Box::new(crate::app::text_editor_view::TextEditorView::new(
            path.clone(),
        )),
        crate::app::permissions::PluginPermissions::builtin(),
        dir.path().to_path_buf(),
        None,
        Some("split_h"),
        None,
    );
    let pane = *h.state().open_panes.last().unwrap();
    h.focus_pane(pane);
    h.run_frames(2);
    h.text_editor_mut(pane).test_set_unsaved("unsaved edits");
    (dir, path, h, pane)
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
        h.app.focus_stack.last().map(|k| k.name()),
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

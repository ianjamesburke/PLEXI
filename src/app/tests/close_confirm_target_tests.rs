//! Stint 1491: the "Close pane?" confirmation closes the pane that asked,
//! never whatever is focused when the user confirms.

use crate::protocol::HostRequest;
use crate::testing::HostHarness;

/// A terminal shell that never writes a title escape, held for the life of
/// the returned guard on this thread (stint 1530). A real login shell sets
/// the title from its prompt, and under load that arrives within the test's
/// few frames, so the fixture's titles would depend on shell timing.
#[cfg(unix)]
fn quiet_shell() -> (tempfile::TempDir, crate::host::shell::TestShellOverrideGuard) {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = tempfile::tempdir().expect("shell dir");
    let script = dir.path().join("quiet-shell.sh");
    std::fs::write(&script, "#!/bin/sh\nexec cat\n").expect("write shell");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    let guard = crate::host::shell::set_test_shell_override(script.to_string_lossy());
    (dir, guard)
}

fn command() -> egui::Modifiers {
    egui::Modifiers::MAC_CMD | egui::Modifiers::COMMAND
}

/// Two side-by-side panes, `a` focused, with close confirmation on.
fn two_panes(h: &mut HostHarness) -> (u64, u64) {
    let a = h.add_test_pane();
    let b = h.add_test_pane();
    let context = h.app.router.active().context_id;
    h.app.set_context_columns(context, vec![vec![a], vec![b]]);
    h.app.focus_pane_in_strip(a);
    h.app.config.confirm_close = Some(true);
    h.run_frames(2);
    (a, b)
}

fn ask_to_close(h: &mut HostHarness) {
    h.press_key(egui::Key::W, command());
    h.run_frames(2);
    assert!(h.app.pending_close, "the close asks first");
}

fn confirm(h: &mut HostHarness) {
    h.press_key(egui::Key::Enter, egui::Modifiers::NONE);
    h.run_frames(2);
}

fn open(h: &HostHarness) -> Vec<u64> {
    let mut panes: Vec<u64> = h
        .app
        .windows
        .iter()
        .flat_map(|w| w.panes.keys().copied())
        .collect();
    panes.sort_unstable();
    panes
}

fn notice_titles(h: &HostHarness) -> Vec<String> {
    h.app
        .pending_notifications
        .iter()
        .map(|n| n.title.clone())
        .collect()
}

/// Focus moves to `b` through the command a CLI `pane focus` sends while
/// the dialog for `a` is up: confirming closes nothing and says why.
#[test]
fn confirm_after_focus_moved_closes_nothing_and_says_so() {
    let mut h = HostHarness::new();
    let (a, b) = two_panes(&mut h);
    ask_to_close(&mut h);
    h.inject_ipc(HostRequest::FocusPane { pane_id: b });
    h.run_frames(2);
    assert_eq!(h.app.active_focused_pane_id(), Some(b));

    confirm(&mut h);

    let mut both = vec![a, b];
    both.sort_unstable();
    assert_eq!(open(&h), both, "neither pane was closed");
    assert!(!h.app.pending_close, "the dialog is gone");
    assert!(
        notice_titles(&h).iter().any(|t| t.contains("not closed")),
        "the user is told: {:?}",
        notice_titles(&h)
    );
}

/// `a` is closed by another route while its dialog is up: confirming
/// closes nothing, in particular not the newly focused `b`.
#[test]
fn confirm_after_the_pane_is_gone_closes_nothing() {
    let mut h = HostHarness::new();
    let (a, b) = two_panes(&mut h);
    ask_to_close(&mut h);
    h.inject_ipc(HostRequest::ClosePane {
        pane_id: a,
        response_file: None,
    });
    h.run_frames(2);
    assert!(!open(&h).contains(&a));

    confirm(&mut h);

    assert_eq!(open(&h), vec![b], "b survives the confirm");
}

/// No focus change: the dialog still closes the pane it asked about, and
/// names it.
#[test]
fn confirm_without_focus_change_closes_the_asking_pane() {
    let mut h = HostHarness::new();
    let (a, b) = two_panes(&mut h);
    ask_to_close(&mut h);
    let target = h
        .app
        .close_confirm_target
        .clone()
        .expect("dialog records its pane");
    assert_eq!(target.pane, a);
    assert!(!target.label.is_empty());

    confirm(&mut h);

    assert_eq!(open(&h), vec![b], "a closed");
    assert!(h.app.close_confirm_target.is_none());
}

/// Critic follow-up 1: on an empty extra page there is no pane to name, so
/// Cmd+W keeps doing the no-pane cleanup at once (the page is removed) and no
/// dialog opens, with close confirmation on.
#[test]
fn close_on_an_empty_extra_page_removes_it_without_a_dialog() {
    let mut h = HostHarness::new();
    h.add_test_pane();
    h.app.config.confirm_close = Some(true);
    let ctx_id = h.app.router.active().context_id;
    let root = h.app.router.active().root.clone();
    let window_id = h.app.next_window_id;
    h.app.next_window_id += 1;
    h.app.windows.push(crate::host::context::Window {
        name: "empty page".to_string(),
        path: root,
        tree: egui_tiles::Tree::empty("plexi"),
        panes: std::collections::HashMap::new(),
        focused_pane: None,
        zoomed_pane: None,
        grid_x: 0,
        grid_y: 1,
        window_id,
        context_id: ctx_id,
    });
    h.app.active_window = h.app.windows.len() - 1;
    h.run_frames(2);
    let pages = |h: &HostHarness| {
        h.app
            .windows
            .iter()
            .filter(|w| w.context_id == ctx_id)
            .count()
    };
    assert_eq!(pages(&h), 2);

    h.press_key(egui::Key::W, command());
    h.run_frames(2);

    assert!(!h.app.pending_close, "no dialog without a pane to name");
    assert!(h.app.close_confirm_target.is_none());
    assert_eq!(pages(&h), 1, "the empty page was removed");
    assert!(h.app.windows.iter().all(|w| w.window_id != window_id));
}

/// Critic follow-up 2: resolving a quit dialog drops a recorded close
/// target with `pending_close`, so it cannot outlive the dialog.
#[test]
fn quit_dialog_resolution_clears_the_close_target() {
    let mut h = HostHarness::new();
    let (a, _b) = two_panes(&mut h);
    for resolve in [
        (|h: &mut HostHarness| h.app.resolve_quit_confirmation(false)) as fn(&mut HostHarness),
        |h: &mut HostHarness| h.app.resolve_quit_save_failure(false),
    ] {
        h.app.close_confirm_target = Some(h.app.close_confirm_target_for(a));
        h.app.pending_close = true;
        resolve(&mut h);
        assert!(!h.app.pending_close);
        assert!(h.app.close_confirm_target.is_none());
    }
}

/// Stint 1500: the asking pane is closed another way while its dialog is up.
/// The dialog closes at once, a notice names the pane, and nothing else
/// closes.
#[test]
fn pane_gone_while_its_dialog_is_up_closes_the_dialog_and_says_so() {
    let mut h = HostHarness::new();
    let (a, b) = two_panes(&mut h);
    ask_to_close(&mut h);
    let label = h.app.close_confirm_target.clone().unwrap().label;
    h.app.notifications_enabled = true;
    h.inject_ipc(HostRequest::ClosePane {
        pane_id: a,
        response_file: None,
    });
    h.run_frames(3);
    assert!(!h.app.pending_close, "the dialog closed with its pane");
    assert!(h.app.close_confirm_target.is_none());
    assert_eq!(open(&h), vec![b], "nothing else closed");
    assert!(
        notice_titles(&h)
            .iter()
            .any(|t| t.contains(&label) && t.contains("already closed")),
        "the user is told: {:?}",
        notice_titles(&h)
    );
}

/// Stint 1500: an unnamed terminal's dialog names it with the title its
/// header paints.
#[test]
#[cfg(unix)]
fn unnamed_terminal_dialog_label_matches_its_header_title() {
    let _shell = quiet_shell();
    let mut h = HostHarness::new();
    let term = h.add_focused_terminal();
    h.run_frames(2);
    let terminal = h.app.windows[0].panes[&term].as_terminal().unwrap();
    assert!(terminal.name.is_none(), "the terminal is unnamed");
    let label = h.app.close_confirm_target_for(term).label;
    let header_titles: Vec<String> = crate::ui::layout_report::snapshot(&h.app.ctx)
        .into_iter()
        .filter(|r| r.surface == "pane_header")
        .flat_map(|r| {
            r.texts
                .into_iter()
                .filter(|t| t.id == "title")
                .map(|t| t.text)
        })
        .collect();
    assert!(
        header_titles.contains(&label),
        "dialog label {label:?} is not a header title {header_titles:?}"
    );
}

/// Stint 1500: two terminals in one folder share a title; their dialogs
/// still say which one closes.
#[test]
#[cfg(unix)]
fn same_folder_terminals_get_distinct_dialog_titles() {
    let _shell = quiet_shell();
    let mut h = HostHarness::new();
    let a = h.add_focused_terminal();
    h.run_frames(2);
    let b = h
        .app
        .split_focused(true, None, false, false, None)
        .expect("a second terminal");
    h.run_frames(3);
    let title = |p: u64| {
        h.app.windows[0].panes[&p]
            .as_terminal()
            .unwrap()
            .display_name()
            .map(str::to_string)
    };
    assert_eq!(title(a), title(b), "fixture: both terminals share a title");
    let la = h.app.close_confirm_target_for(a).label;
    let lb = h.app.close_confirm_target_for(b).label;
    assert_ne!(la, lb, "both dialogs would read the same");

    h.app.config.confirm_close = Some(true);
    h.focus_pane(b);
    h.run_frames(2);
    h.press_key(egui::Key::W, command());
    h.run_frames(3);
    assert!(h.app.pending_close);
    let png = std::env::temp_dir().join("plexi_same_folder_terminals_close_dialog.png");
    h.render_png(&png).expect("render the dialog");
}

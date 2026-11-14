//! Stint 1491: the "Close pane?" confirmation closes the pane that asked,
//! never whatever is focused when the user confirms.

use crate::protocol::HostRequest;
use crate::testing::HostHarness;

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

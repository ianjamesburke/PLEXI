//! Stint 1458: the close shortcut closes every selected pane, one after
//! another, each through its own close veto and dialog.

use crate::app::FocusKind;
use crate::testing::HostHarness;

fn command() -> egui::Modifiers {
    egui::Modifiers::MAC_CMD | egui::Modifiers::COMMAND
}

fn close_shortcut(h: &mut HostHarness) {
    h.press_key(egui::Key::W, command());
    h.run_frames(2);
}

/// Two columns: `a` over `b`, `c` over `d`. Focus on `a`.
fn grid(h: &mut HostHarness) -> [u64; 4] {
    let panes = [
        h.add_test_pane(),
        h.add_test_pane(),
        h.add_test_pane(),
        h.add_test_pane(),
    ];
    let context = h.app.router.active().context_id;
    h.app.set_context_columns(
        context,
        vec![vec![panes[0], panes[1]], vec![panes[2], panes[3]]],
    );
    h.app.focus_pane_in_strip(panes[0]);
    h.run_frames(2);
    panes
}

fn select(h: &mut HostHarness, panes: &[u64]) {
    let window = h.app.windows[h.app.active_window].window_id;
    for pair in panes.windows(2) {
        h.app.host.extend_pane_selection(window, pair[0], pair[1]);
    }
}

fn open(h: &HostHarness) -> Vec<u64> {
    let mut panes: Vec<u64> = h.app.windows[h.app.active_window]
        .panes
        .keys()
        .copied()
        .collect();
    panes.sort_unstable();
    panes
}

#[test]
fn close_shortcut_closes_every_selected_pane_and_clears_the_selection() {
    let mut h = HostHarness::new();
    let [a, b, c, d] = grid(&mut h);
    let window = h.app.windows[0].window_id;
    let select_key = command() | egui::Modifiers::SHIFT;
    h.press_key(egui::Key::J, select_key);
    h.press_key(egui::Key::L, select_key);
    assert_eq!(
        h.app.host.pane_selection(window).unwrap().panes,
        vec![a, b, d]
    );
    close_shortcut(&mut h);
    assert_eq!(open(&h), vec![c], "all three selected panes closed");
    assert!(h.app.host.pane_selection(window).is_none());
    assert!(h.app.host.pane_close_batch().is_none());
}

/// Focus is chosen once, against the final layout, from the anchor's place:
/// the spatial-neighbour rule skips the closed panes and lands on `c`, the
/// only pane left, never on a pane the batch is about to close.
#[test]
fn close_selected_focuses_the_spatial_neighbour_of_the_anchor() {
    let mut h = HostHarness::new();
    let [a, b, c, d] = grid(&mut h);
    let e = h.add_test_pane();
    let context = h.app.router.active().context_id;
    h.app
        .set_context_columns(context, vec![vec![a, b], vec![c, d], vec![e]]);
    h.app.focus_pane_in_strip(a);
    h.run_frames(2);
    select(&mut h, &[a, b, c]);
    close_shortcut(&mut h);
    assert_eq!(open(&h), vec![d, e]);
    assert_eq!(
        h.app.focused_pane_id(),
        Some(d),
        "rule=spatial_neighbour: the right column's surviving pane beside the anchor"
    );
}

#[test]
fn close_shortcut_without_a_selection_closes_only_the_focused_pane() {
    let mut h = HostHarness::new();
    let [a, b, c, d] = grid(&mut h);
    close_shortcut(&mut h);
    assert_eq!(open(&h), vec![b, c, d]);
    assert!(!open(&h).contains(&a));
}

#[test]
fn close_shortcut_with_a_selection_of_one_closes_the_focused_pane_as_before() {
    let mut h = HostHarness::new();
    let [a, b, c, d] = grid(&mut h);
    h.press_key(egui::Key::H, command() | egui::Modifiers::SHIFT);
    let window = h.app.windows[0].window_id;
    assert_eq!(h.app.host.pane_selection(window).unwrap().panes, vec![a]);
    close_shortcut(&mut h);
    assert_eq!(open(&h), vec![b, c, d]);
}

#[test]
fn palette_close_pane_closes_the_selection_too() {
    let mut h = HostHarness::new();
    let [a, b, c, d] = grid(&mut h);
    select(&mut h, &[a, c]);
    let ctx = h.app.ctx.clone();
    h.app
        .dispatch_host_action(crate::host::keys::Action::ClosePane, &ctx);
    h.run_frames(2);
    assert_eq!(open(&h), vec![b, d]);
}

/// `x`, an unsaved editor, then `y`, all selected in that order.
fn editor_batch() -> (
    tempfile::TempDir,
    std::path::PathBuf,
    HostHarness,
    [u64; 3],
    u64,
) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("document.txt");
    std::fs::write(&path, b"original bytes\n").unwrap();
    let mut h = HostHarness::new();
    let x = h.add_test_pane();
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
    let editor = h
        .state()
        .open_panes
        .into_iter()
        .find(|id| *id != x)
        .expect("editor opened");
    let y = h.add_test_pane();
    let keep = h.add_test_pane();
    let context = h.app.router.active().context_id;
    h.app
        .set_context_columns(context, vec![vec![x], vec![editor], vec![y], vec![keep]]);
    h.app.focus_pane_in_strip(x);
    h.run_frames(2);
    h.text_editor_mut(editor).test_set_unsaved("unsaved edits");
    select(&mut h, &[x, editor, y]);
    close_shortcut(&mut h);
    assert!(!open(&h).contains(&x), "x closes before the editor asks");
    assert_eq!(
        h.app.focus_stack.last(),
        Some(&FocusKind::UnsavedChanges),
        "the batch pauses on the editor's own dialog"
    );
    assert_eq!(h.app.focused_pane_id(), Some(editor));
    assert!(open(&h).contains(&y), "y waits behind the dialog");
    (dir, path, h, [x, editor, y], keep)
}

#[test]
fn close_selected_cancel_on_unsaved_editor_stops_and_keeps_the_rest_selected() {
    let (_dir, path, mut h, [_, editor, y], keep) = editor_batch();
    h.press_key(egui::Key::Escape, egui::Modifiers::NONE);
    h.run_frames(2);
    assert_eq!(open(&h), {
        let mut v = vec![editor, y, keep];
        v.sort_unstable();
        v
    });
    let window = h.app.windows[h.app.active_window].window_id;
    assert_eq!(
        h.app.host.pane_selection(window).map(|s| s.panes.clone()),
        Some(vec![editor, y]),
        "Cancel leaves the remaining panes selected"
    );
    assert!(h.app.host.pane_close_batch().is_none());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "original bytes\n");
    h.text_editor_mut(editor).test_discard_unsaved();
}

#[test]
fn close_selected_discard_on_unsaved_editor_continues_the_batch() {
    let (_dir, path, mut h, _, keep) = editor_batch();
    h.press_key(egui::Key::D, egui::Modifiers::NONE);
    h.run_frames(2);
    assert_eq!(open(&h), vec![keep]);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "original bytes\n");
    assert!(h.app.host.pane_close_batch().is_none());
    assert_eq!(h.app.focused_pane_id(), Some(keep));
}

#[test]
fn close_selected_save_on_unsaved_editor_writes_and_continues() {
    let (_dir, path, mut h, _, keep) = editor_batch();
    h.press_key(egui::Key::S, command());
    h.run_frames(2);
    assert_eq!(open(&h), vec![keep]);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "unsaved edits");
    assert!(h.app.host.pane_close_batch().is_none());
}

/// With `confirm_close` on, each selected pane asks its own "Close pane?".
#[test]
fn close_selected_asks_confirm_close_once_per_pane() {
    let mut h = HostHarness::new();
    let [a, b, c, d] = grid(&mut h);
    h.app.config.confirm_close = Some(true);
    select(&mut h, &[a, b]);
    close_shortcut(&mut h);
    assert!(h.app.pending_close, "first pane asks");
    assert_eq!(open(&h), vec![a, b, c, d]);
    h.press_key(egui::Key::Enter, egui::Modifiers::NONE);
    h.run_frames(2);
    assert!(!open(&h).contains(&a));
    assert!(h.app.pending_close, "second pane asks");
    h.press_key(egui::Key::Escape, egui::Modifiers::NONE);
    h.run_frames(2);
    assert_eq!(open(&h), vec![b, c, d], "Cancel keeps b");
    let window = h.app.windows[0].window_id;
    assert_eq!(
        h.app.host.pane_selection(window).map(|s| s.panes.clone()),
        Some(vec![b])
    );
}

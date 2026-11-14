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

// ── Critic blockers on PR #3136 ─────────────────────────────────────────────

fn dispatch(h: &mut HostHarness, action: crate::host::keys::Action) {
    let ctx = h.app.ctx.clone();
    h.app.dispatch_host_action(action, &ctx);
    h.run_frames(2);
}

fn selection(h: &HostHarness) -> Option<Vec<u64>> {
    let window = h.app.windows[h.app.active_window].window_id;
    h.app.host.pane_selection(window).map(|s| s.panes.clone())
}

/// Blocker 1: a selected view opened as an overlay over another pane closes
/// the way a single close does: the covered pane comes back on the same id.
#[test]
fn close_selected_restores_the_pane_under_a_selected_overlay() {
    let mut h = HostHarness::new();
    let [a, b, c, d] = grid(&mut h);
    let covered = h.app.windows[0].panes.remove(&a).unwrap();
    let cwd = h.workspace_root();
    let overlay = crate::host::pane::Pane::View(Box::new(crate::host::pane::ViewPane {
        pip_status: None,
        id: a,
        permissions: crate::app::permissions::PluginPermissions::builtin(),
        runtime: crate::host::pane::ViewRuntime::Builtin(Box::new(
            crate::file_browser::FileBrowserView::new(cwd.clone()),
        )),
        workspace_root: cwd,
        manifest_id: "overlay".to_string(),
        name: "overlay".to_string(),
        pane_group: None,
        linked_pane_id: None,
        overlay_replaced: Some(Box::new(covered)),
        agent: None,
        slots: std::collections::HashMap::new(),
        semantic_state: Default::default(),
    }));
    h.app.windows[0].panes.insert(a, overlay);
    select(&mut h, &[a, b]);
    close_shortcut(&mut h);
    assert_eq!(open(&h), vec![a, c, d], "the covered pane survives on its id");
    assert_eq!(
        h.app.windows[0].panes[&a].as_view().map(|v| v.manifest_id.as_str()),
        Some("test"),
        "the overlay closed and uncovered the original pane"
    );
}

/// Blocker 2: a selection that is not drawn (a pane is zoomed) is not acted
/// on: zoom clears it, and Cmd+W closes the focused pane as before.
#[test]
fn zoom_clears_the_selection_and_close_acts_on_the_focused_pane() {
    let mut h = HostHarness::new();
    let [a, b, c, d] = grid(&mut h);
    let select_key = command() | egui::Modifiers::SHIFT;
    h.press_key(egui::Key::J, select_key);
    assert_eq!(selection(&h), Some(vec![a, b]));
    dispatch(&mut h, crate::host::keys::Action::ToggleZoom);
    assert_eq!(selection(&h), None, "zoom hides the selection, so it clears");
    dispatch(&mut h, crate::host::keys::Action::ToggleZoom);
    close_shortcut(&mut h);
    assert_eq!(open(&h), vec![a, c, d], "only the focused pane closes");
}

/// Blocker 2: a focus jump that is not a selection step (palette, CLI,
/// history) ends the selection.
#[test]
fn a_focus_jump_clears_the_selection() {
    let mut h = HostHarness::new();
    let [a, b, c, d] = grid(&mut h);
    h.press_key(egui::Key::J, command() | egui::Modifiers::SHIFT);
    assert_eq!(selection(&h), Some(vec![a, b]));
    h.app.focus_pane_in_strip(d);
    h.run_frames(2);
    assert_eq!(selection(&h), None);
    close_shortcut(&mut h);
    assert_eq!(open(&h), vec![a, b, c]);
}

/// Blocker 2: a new split moves focus to the new pane and ends the selection.
#[test]
fn a_split_clears_the_selection() {
    let mut h = HostHarness::new();
    let [a, b, ..] = grid(&mut h);
    h.press_key(egui::Key::J, command() | egui::Modifiers::SHIFT);
    assert_eq!(selection(&h), Some(vec![a, b]));
    dispatch(&mut h, crate::host::keys::Action::SplitRight);
    assert_eq!(selection(&h), None);
}

/// Blocker 3: while paused on a dialog, the panes still to close stay drawn
/// as the selection.
#[test]
fn a_paused_batch_keeps_the_remaining_panes_selected() {
    let (_dir, _path, mut h, [_, editor, y], _) = editor_batch();
    assert_eq!(selection(&h), Some(vec![editor, y]));
    h.text_editor_mut(editor).test_discard_unsaved();
}

/// Blocker 3: when focus leaves the asking pane the batch ends; a later
/// answer to the editor's dialog closes only the editor, and Cmd+W elsewhere
/// closes the focused pane.
#[test]
fn focus_leaving_the_asking_pane_cancels_the_batch() {
    let (_dir, _path, mut h, [_, editor, y], keep) = editor_batch();
    h.app.focus_pane_in_strip(keep);
    h.run_frames(2);
    assert!(h.app.host.pane_close_batch().is_none(), "the batch ended");
    assert_eq!(selection(&h), None);
    close_shortcut(&mut h);
    assert!(!open(&h).contains(&keep), "Cmd+W works on the focused pane");
    assert!(open(&h).contains(&y), "y was never closed");
    assert!(open(&h).contains(&editor));
    h.text_editor_mut(editor).test_discard_unsaved();
}

/// Follow-up: Cmd+Shift+click on a pane already selected takes it out.
#[test]
fn reselecting_a_selected_pane_removes_it() {
    let mut h = HostHarness::new();
    let [a, b, c, _] = grid(&mut h);
    let window = h.app.windows[0].window_id;
    h.app.host.extend_pane_selection(window, a, b);
    h.app.host.extend_pane_selection(window, b, c);
    h.app.host.toggle_pane_selection(window, a, b);
    assert_eq!(selection(&h), Some(vec![a, c]));
}

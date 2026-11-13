//! Legacy workspace metadata compatibility and persistence regressions.

use crate::strip::ColumnWidth;
use crate::testing::HostHarness;

#[test]
fn regression_authoritative_strip_model_matches_host() {
    let mut h = HostHarness::new();
    let p1 = h.add_test_pane();
    let p2 = h.add_test_pane();
    let p3 = h.add_test_pane();
    let ctx_id = h.app.router.active().context_id;

    h.app
        .set_context_columns(ctx_id, vec![vec![p1], vec![p2, p3]]);
    h.app.focus_pane_in_strip(p2);
    h.app.set_column_width_ratio(ctx_id, 0, 0.40);
    h.run_frames(2);

    let strip = h.app.strip_for_context(ctx_id);
    assert_eq!(strip.context_id, ctx_id);
    assert_eq!(strip.column_count(), 2);
    assert_eq!(strip.pane_count(), 3);
    assert_eq!(strip.columns[0].panes, vec![p1]);
    assert_eq!(strip.columns[1].panes, vec![p2, p3]);
    assert_eq!(strip.columns[0].width, ColumnWidth::CustomRatio(0.40));
    assert_eq!(strip.focused_pane, Some(p2));

    let layout = h.app.active_strip_layout();
    assert_eq!(layout.panes.len(), 3);
    assert!(layout.panes.iter().all(|pane| pane.is_visible));
}

/// Stint 1039 baseline-red regression:
/// In an ordinary multi-column context (where `set_context_columns` was never called)
/// and after workspace save/reload, every visible pane (leftmost, stacked top/bottom,
/// rightmost) must accept pointer focus and unaddressed host keyboard input; `Cmd+D`
/// from the leftmost or stacked pane must insert an immediate right neighbor column
/// while preserving prior columns' `ColumnId`, order, and remembered focus; `Cmd+H/L`
/// and `Cmd+K/J` must navigate and reveal without wrapping; and `Cmd+Ctrl+H/L`
/// (`swap_pane`) must exchange adjacent pane positions while keeping focus on the
/// moved pane identity.

#[test]
fn custom_width_survives_focus_split_close_and_zoom() {
    let mut h = HostHarness::new();
    let first = h.add_test_pane();
    let second = h.add_test_pane();
    let context = h.app.router.active().context_id;
    h.app
        .set_context_columns(context, vec![vec![first], vec![second]]);
    h.app.focus_pane_in_strip(second);
    h.run_frames(2);
    h.app.ensure_authoritative_strip(context).columns[0].width = ColumnWidth::CustomPixels(333.0);
    h.run_frames(3);
    h.app.focus_pane_in_strip(first);
    h.run_frames(2);
    h.app.focus_pane_in_strip(second);
    let inserted = h
        .app
        .split_focused(false, None, false, false, None)
        .expect("split");
    h.run_frames(2);
    assert_eq!(h.app.focused_pane_id(), Some(inserted));
    h.app.close_focused();
    h.run_frames(2);
    h.app.focus_pane_in_strip(first);
    h.app.toggle_pane_zoom();
    h.run_frames(2);
    h.app.toggle_pane_zoom();
    h.run_frames(2);
    let strip = h.app.strip_for_context(context);
    assert_eq!(
        strip.columns[strip.find_column_for_pane(first).unwrap()].width,
        ColumnWidth::CustomPixels(333.0)
    );
}

#[test]
fn stack_shares_survive_split_and_close_by_pane_identity() {
    let mut h = HostHarness::new();
    let first = h.add_test_pane();
    let second = h.add_test_pane();
    let context = h.app.router.active().context_id;
    h.app
        .set_context_columns(context, vec![vec![first, second]]);
    h.app.focus_pane_in_strip(second);
    h.run_frames(2);
    h.app.ensure_authoritative_strip(context).columns[0].heights = vec![2.0, 3.0];
    h.run_frames(2);
    let inserted = h
        .app
        .split_focused(true, None, false, false, None)
        .expect("split stack");
    h.run_frames(2);
    let strip = h.app.strip_for_context(context);
    for (pane, expected) in [(first, 2.0), (second, 3.0)] {
        let col = strip.find_column_for_pane(pane).unwrap();
        let row = strip.columns[col]
            .panes
            .iter()
            .position(|id| *id == pane)
            .unwrap();
        assert_eq!(
            strip.columns[col].heights[row], expected,
            "share after split pane={pane}"
        );
    }
    assert_eq!(h.app.focused_pane_id(), Some(inserted));
    h.app.close_focused();
    h.run_frames(2);
    let strip = h.app.strip_for_context(context);
    for (pane, expected) in [(first, 2.0), (second, 3.0)] {
        let col = strip.find_column_for_pane(pane).unwrap();
        let row = strip.columns[col]
            .panes
            .iter()
            .position(|id| *id == pane)
            .unwrap();
        assert_eq!(
            strip.columns[col].heights[row], expected,
            "share after close pane={pane}"
        );
    }
}

#[test]
fn saved_strip_restores_width_scroll_and_focus() {
    let mut h = HostHarness::new();
    let first = h.add_test_pane();
    let second = h.add_test_pane();
    let context = h.app.router.active().context_id;
    for id in [first, second] {
        h.app.windows[0]
            .panes
            .get_mut(&id)
            .unwrap()
            .as_view_mut()
            .unwrap()
            .manifest_id = "agents".into();
    }
    h.app
        .set_context_columns(context, vec![vec![first], vec![second]]);
    h.app.focus_pane_in_strip(second);
    {
        let strip = h.app.ensure_authoritative_strip(context);
        strip.columns[0].width = ColumnWidth::CustomPixels(500.0);
        strip.columns[1].width = ColumnWidth::CustomPixels(700.0);
        strip.scroll_offset_px = 200.0;
        strip.reference_width_px = Some(600.0);
    }
    let expected = h.app.strip_for_context(context);
    let profile = h.profile_dir().to_path_buf();
    h.app.save_workspace_now();
    let restored = HostHarness::from_profile_dir(profile);
    let actual = restored.app.strip_for_context(context);
    assert_eq!(actual.columns, expected.columns);
    assert_eq!(actual.focused_pane, expected.focused_pane);
    assert_eq!(actual.scroll_offset_px, expected.scroll_offset_px);
}

#[test]
fn saved_strip_missing_visible_pane_is_rejected_without_overwrite() {
    let mut h = HostHarness::new();
    let pane = h.add_test_pane();
    let context = h.app.router.active().context_id;
    h.app.set_context_columns(context, vec![vec![pane]]);
    h.app.save_workspace_now();
    let profile = h.profile_dir().to_path_buf();
    let mut saved = crate::workspace::WorkspaceFile::load_from(&profile).unwrap();
    let strip = saved.strips.get_mut(&context).unwrap();
    strip.columns.clear();
    strip.focused_pane = None;
    let path = crate::workspace::workspace_path_in(&profile);
    let original = serde_json::to_vec(&saved).unwrap();
    std::fs::write(&path, &original).unwrap();
    assert!(crate::workspace::WorkspaceFile::load_from(&profile).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), original);
}

/// Stint 1312: a version 2 layout is no longer converted. The host boots with
/// nothing from it restored, every save refuses to replace it, and nothing is
/// written beside it.
#[test]
fn retired_v2_layout_boots_empty_and_is_never_rewritten() {
    let mut h = HostHarness::new();
    let pane = h.add_test_pane();
    let context = h.app.router.active().context_id;
    h.app.set_context_columns(context, vec![vec![pane]]);
    h.app.save_workspace_now();
    let profile = h.profile_dir().to_path_buf();
    let path = crate::workspace::workspace_path_in(&profile);
    let mut legacy: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    legacy["version"] = 2.into();
    legacy.as_object_mut().unwrap().remove("strips");
    legacy.as_object_mut().unwrap().remove("conjoined_contexts");
    let original = serde_json::to_vec_pretty(&legacy).unwrap();
    std::fs::write(&path, &original).unwrap();

    assert!(
        !h.app.save_workspace_now(),
        "a save must not replace a version 2 layout"
    );
    assert_eq!(std::fs::read(&path).unwrap(), original);

    let restored = HostHarness::from_profile_dir(&profile);
    assert!(
        restored.app.find_pane_in_any_window(pane).is_none(),
        "nothing from a version 2 layout is restored"
    );
    // `h` owns the profile directory, so only the restored harness is dropped.
    drop(restored);
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert!(!path.with_extension("v2.rollback.json").exists());
}

#[test]
fn restored_strip_keeps_scroll_after_first_frame() {
    let mut h = HostHarness::new();
    let panes = [h.add_test_pane(), h.add_test_pane(), h.add_test_pane()];
    let context = h.app.router.active().context_id;
    for id in panes {
        h.app.windows[0]
            .panes
            .get_mut(&id)
            .unwrap()
            .as_view_mut()
            .unwrap()
            .manifest_id = "agents".into();
    }
    h.app
        .set_context_columns(context, panes.into_iter().map(|pane| vec![pane]).collect());
    h.app.focus_pane_in_strip(panes[1]);
    h.run_frames(3);
    let canvas = h.app.active_strip_layout().canvas_rect.width();
    {
        let strip = h.app.ensure_authoritative_strip(context);
        for column in &mut strip.columns {
            column.width = ColumnWidth::CustomPixels(700.0);
        }
        strip.scroll_offset_px = 450.0;
        strip.reference_width_px = Some(canvas);
    }
    let expected = h.app.strip_for_context(context).scroll_offset_px;
    let profile = h.profile_dir().to_path_buf();
    h.app.save_workspace_now();
    let mut restored = HostHarness::from_profile_dir(profile);
    restored.run_frames(3);
    assert!((restored.app.strip_for_context(context).scroll_offset_px - expected).abs() < 0.5);
    assert_eq!(restored.app.focused_pane_id(), Some(panes[1]));
}

#[test]
fn restored_focus_receives_unaddressed_key_without_refocusing() {
    let mut h = HostHarness::new();
    h.add_focused_terminal();
    h.run_frames(2);
    let focused = h
        .app
        .split_focused(false, None, false, false, None)
        .expect("split terminal");
    h.run_frames(2);
    let profile = h.profile_dir().to_path_buf();
    h.app.save_workspace_now();
    let mut restored = HostHarness::from_profile_dir(profile);
    restored.run_frames(3);
    assert_eq!(
        restored.app.host_input_owner(),
        crate::app::input_owner::InputOwner::Pane(focused)
    );
    restored.terminal_backend(focused).enable_input_tap();
    restored.press_key(egui::Key::Tab, egui::Modifiers::NONE);
    assert_eq!(restored.terminal_backend(focused).take_input_tap(), b"\t");
}

#[test]
fn saving_strip_keeps_live_terminal_process() {
    let mut h = HostHarness::new();
    let pane = h.add_focused_terminal();
    let before = h.terminal_backend(pane).child_pid();
    assert!(before > 0, "fixture must start a real terminal process");
    let context = h.app.router.active().context_id;
    h.app.ensure_authoritative_strip(context);
    h.app.save_workspace_now();
    assert_eq!(h.terminal_backend(pane).child_pid(), before);
}

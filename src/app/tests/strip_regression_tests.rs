//! Legacy workspace metadata compatibility and persistence regressions.

use crate::strip::ColumnWidth;
use crate::testing::HostHarness;
use egui_tiles::{Container, Tile};

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
    h.app.set_context_columns(context, vec![vec![first], vec![second]]);
    h.app.focus_pane_in_strip(second);
    h.run_frames(2);
    h.app.ensure_authoritative_strip(context).columns[0].width = ColumnWidth::CustomPixels(333.0);
    h.run_frames(3);
    h.app.focus_pane_in_strip(first);
    h.run_frames(2);
    h.app.focus_pane_in_strip(second);
    let inserted = h.app.split_focused(false, None, false, false, None).expect("split");
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
    assert_eq!(strip.columns[strip.find_column_for_pane(first).unwrap()].width, ColumnWidth::CustomPixels(333.0));
}

#[test]
fn stack_shares_survive_split_and_close_by_pane_identity() {
    let mut h = HostHarness::new();
    let first = h.add_test_pane();
    let second = h.add_test_pane();
    let context = h.app.router.active().context_id;
    h.app.set_context_columns(context, vec![vec![first, second]]);
    h.app.focus_pane_in_strip(second);
    h.run_frames(2);
    h.app.ensure_authoritative_strip(context).columns[0].heights = vec![2.0, 3.0];
    h.run_frames(2);
    let inserted = h.app.split_focused(true, None, false, false, None).expect("split stack");
    h.run_frames(2);
    let strip = h.app.strip_for_context(context);
    for (pane, expected) in [(first, 2.0), (second, 3.0)] {
        let col = strip.find_column_for_pane(pane).unwrap();
        let row = strip.columns[col].panes.iter().position(|id| *id == pane).unwrap();
        assert_eq!(strip.columns[col].heights[row], expected, "share after split pane={pane}");
    }
    assert_eq!(h.app.focused_pane_id(), Some(inserted));
    h.app.close_focused();
    h.run_frames(2);
    let strip = h.app.strip_for_context(context);
    for (pane, expected) in [(first, 2.0), (second, 3.0)] {
        let col = strip.find_column_for_pane(pane).unwrap();
        let row = strip.columns[col].panes.iter().position(|id| *id == pane).unwrap();
        assert_eq!(strip.columns[col].heights[row], expected, "share after close pane={pane}");
    }
}

#[test]
fn saved_strip_restores_width_scroll_and_focus() {
    let mut h = HostHarness::new();
    let first = h.add_test_pane();
    let second = h.add_test_pane();
    let context = h.app.router.active().context_id;
    for id in [first, second] {
        h.app.windows[0].panes.get_mut(&id).unwrap().as_app_mut().unwrap().manifest_id = "agents".into();
    }
    h.app.set_context_columns(context, vec![vec![first], vec![second]]);
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
    assert!(crate::workspace::WorkspaceFile::load_from(&profile).is_none());
    assert_eq!(std::fs::read(&path).unwrap(), original);
}

#[test]
fn invalid_legacy_tree_is_rejected_without_replacing_original() {
    let mut h = HostHarness::new();
    let pane = h.add_test_pane();
    let context = h.app.router.active().context_id;
    h.app.set_context_columns(context, vec![vec![pane]]);
    h.app.save_workspace_now();
    let profile = h.profile_dir().to_path_buf();
    let mut saved = crate::workspace::WorkspaceFile::load_from(&profile).unwrap();
    saved.version = 2;
    saved.strips.clear();
    let window = &mut saved.windows[0];
    let root = window.tree.root().unwrap();
    let missing = window.tree.tiles.insert_pane(u64::MAX);
    window.tree.root = Some(window.tree.tiles.insert_horizontal_tile(vec![root, missing]));
    let path = crate::workspace::workspace_path_in(&profile);
    let original = serde_json::to_vec(&saved).unwrap();
    std::fs::write(&path, &original).unwrap();
    assert!(crate::workspace::WorkspaceFile::load_from(&profile).is_none());
    assert_eq!(std::fs::read(&path).unwrap(), original);
}

#[test]
fn restored_strip_keeps_scroll_after_first_frame() {
    let mut h = HostHarness::new();
    let panes = [h.add_test_pane(), h.add_test_pane(), h.add_test_pane()];
    let context = h.app.router.active().context_id;
    for id in panes {
        h.app.windows[0].panes.get_mut(&id).unwrap().as_app_mut().unwrap().manifest_id = "agents".into();
    }
    h.app.set_context_columns(context, panes.into_iter().map(|pane| vec![pane]).collect());
    h.app.focus_pane_in_strip(panes[1]);
    h.run_frames(3);
    let canvas = h.app.active_strip_layout().canvas_rect.width();
    {
        let strip = h.app.ensure_authoritative_strip(context);
        for column in &mut strip.columns { column.width = ColumnWidth::CustomPixels(700.0); }
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
fn legacy_fixture_migrates_and_retains_later_v3_changes() {
    let mut h = HostHarness::new();
    let panes = [h.add_test_pane(), h.add_test_pane()];
    let context = h.app.router.active().context_id;
    for id in panes {
        let app = h.app.windows[0].panes.get_mut(&id).unwrap().as_app_mut().unwrap();
        app.manifest_id = "agents".into();
        app.name = format!("rollback pane {id}");
    }
    h.app.set_context_columns(context, panes.into_iter().map(|pane| vec![pane]).collect());
    h.app.focus_pane_in_strip(panes[1]);
    let profile = h.profile_dir().to_path_buf();
    h.app.save_workspace_now();
    let path = crate::workspace::workspace_path_in(&profile);
    let mut legacy: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    legacy["version"] = 2.into();
    for pane in legacy["windows"][0]["panes"].as_array_mut().unwrap() {
        pane["app_state"] = serde_json::json!({"proof":"saved content reference"});
    }
    legacy.as_object_mut().unwrap().remove("strips");
    legacy.as_object_mut().unwrap().remove("conjoined_contexts");
    let original = serde_json::to_vec_pretty(&legacy).unwrap();
    std::fs::write(&path, &original).unwrap();
    h.app.save_workspace_now();
    let backup = path.with_extension("v2.rollback.json");
    assert_eq!(std::fs::read(&backup).unwrap(), original);
    h.app.ensure_authoritative_strip(context).columns[0].width = ColumnWidth::CustomPixels(777.0);
    h.app.save_workspace_now();
    let newer = std::fs::read(&path).unwrap();
    assert_eq!(std::fs::read(&backup).unwrap(), original);
    let current: serde_json::Value = serde_json::from_slice(&newer).unwrap();
    assert_eq!(current["version"], 3);
    if let Ok(export) = std::env::var("PLEXI_1000_ROLLBACK_PROOF_DIR") {
        let export = std::path::PathBuf::from(export);
        std::fs::create_dir_all(&export).unwrap();
        let destination = export.join("legacy");
        crate::workspace::export_v2_rollback(&profile, &destination).unwrap();
        std::fs::write(export.join("newer-v3.json"), &newer).unwrap();
        std::fs::write(destination.join("identities.json"), serde_json::to_vec(&serde_json::json!({"context":context, "panes":panes})).unwrap()).unwrap();
    }
}

#[test]
fn legacy_workspace_converts_before_host_restoration() {
    let mut h = HostHarness::new();
    let panes = [h.add_test_pane(), h.add_test_pane()];
    let context = h.app.router.active().context_id;
    h.app.set_context_columns(context, panes.into_iter().map(|pane| vec![pane]).collect());
    h.app.focus_pane_in_strip(panes[1]);
    h.app.save_workspace_now();
    let profile = h.profile_dir().to_path_buf();
    let path = crate::workspace::workspace_path_in(&profile);
    let mut legacy: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    legacy["version"] = 2.into();
    legacy.as_object_mut().unwrap().remove("strips");
    legacy.as_object_mut().unwrap().remove("conjoined_contexts");
    let original = serde_json::to_vec_pretty(&legacy).unwrap();
    std::fs::write(&path, &original).unwrap();
    let migrated = crate::workspace::WorkspaceFile::load_from(&profile).expect("legacy workspace");
    assert_eq!(migrated.version, 3);
    assert_eq!(migrated.strips[&context].columns.iter().flat_map(|column| column.panes.iter().copied()).collect::<Vec<_>>(), panes);
    assert_eq!(migrated.strips[&context].focused_pane, Some(panes[1]));
    assert_eq!(std::fs::read(path.with_extension("v2.rollback.json")).unwrap(), original);
}

#[test]
fn legacy_conversion_preserves_tree_widths_and_stack_shares() {
    let mut h = HostHarness::new();
    let panes = [h.add_test_pane(), h.add_test_pane(), h.add_test_pane()];
    let context = h.app.router.active().context_id;
    h.app.set_context_columns(context, vec![vec![panes[0], panes[1]], vec![panes[2]]]);
    let tree = &mut h.app.windows[0].tree;
    let root = tree.root().unwrap();
    let (left, right) = match tree.tiles.get_mut(root).unwrap() {
        Tile::Container(Container::Linear(linear)) => {
            let left = linear.children[0];
            let right = linear.children[1];
            linear.shares.set_share(left, 480.0);
            linear.shares.set_share(right, 720.0);
            (left, right)
        }
        _ => panic!("expected horizontal root"),
    };
    let _ = right;
    match tree.tiles.get_mut(left).unwrap() {
        Tile::Container(Container::Linear(linear)) => {
            let first = linear.children[0];
            let second = linear.children[1];
            linear.shares.set_share(first, 2.0);
            linear.shares.set_share(second, 3.0);
        }
        _ => panic!("expected vertical stack"),
    }
    h.app.save_workspace_now();
    let profile = h.profile_dir().to_path_buf();
    let path = crate::workspace::workspace_path_in(&profile);
    let mut legacy: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    legacy["version"] = 2.into();
    legacy.as_object_mut().unwrap().remove("strips");
    legacy.as_object_mut().unwrap().remove("conjoined_contexts");
    std::fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
    let migrated = crate::workspace::WorkspaceFile::load_from(&profile).unwrap();
    let strip = &migrated.strips[&context];
    assert_eq!(strip.columns[0].width, ColumnWidth::CustomPixels(480.0));
    assert_eq!(strip.columns[1].width, ColumnWidth::CustomPixels(720.0));
    assert_eq!(strip.columns[0].heights, vec![2.0, 3.0]);
}

#[test]
fn legacy_conversion_preserves_relative_column_shares() {
    for shares in [[1.2, 0.8], [1.0, 1.0]] {
        let mut h = HostHarness::new();
        let panes = [h.add_test_pane(), h.add_test_pane()];
        let context = h.app.router.active().context_id;
        h.app.set_context_columns(context, vec![vec![panes[0]], vec![panes[1]]]);
        let tree = &mut h.app.windows[0].tree;
        let root = tree.root().unwrap();
        match tree.tiles.get_mut(root).unwrap() {
            Tile::Container(Container::Linear(linear)) => {
                for (child, share) in linear.children.clone().into_iter().zip(shares) {
                    linear.shares.set_share(child, share);
                }
            }
            _ => panic!("expected horizontal root"),
        }
        h.app.save_workspace_now();
        let profile = h.profile_dir().to_path_buf();
        let path = crate::workspace::workspace_path_in(&profile);
        let mut legacy: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        legacy["version"] = 2.into();
        legacy.as_object_mut().unwrap().remove("strips");
        legacy.as_object_mut().unwrap().remove("conjoined_contexts");
        std::fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
        let migrated = crate::workspace::WorkspaceFile::load_from(&profile).unwrap();
        for (column, share) in migrated.strips[&context].columns.iter().zip(shares) {
            assert_eq!(column.width, ColumnWidth::CustomRatio(share / 2.0));
        }
    }
}

#[test]
fn restored_focus_receives_unaddressed_key_without_refocusing() {
    let mut h = HostHarness::new();
    h.add_focused_terminal();
    h.run_frames(2);
    let focused = h.app.split_focused(false, None, false, false, None).expect("split terminal");
    h.run_frames(2);
    let profile = h.profile_dir().to_path_buf();
    h.app.save_workspace_now();
    let mut restored = HostHarness::from_profile_dir(profile);
    restored.run_frames(3);
    assert_eq!(restored.app.host_input_owner(), crate::app::input_owner::InputOwner::Pane(focused));
    restored.terminal_backend(focused).enable_input_tap();
    restored.press_key(egui::Key::Tab, egui::Modifiers::NONE);
    assert_eq!(restored.terminal_backend(focused).take_input_tap(), b"\t");
}

#[test]
fn legacy_tab_conversion_retains_inactive_pane_and_active_identity() {
    let mut h = HostHarness::new();
    let panes = [h.add_test_pane(), h.add_test_pane()];
    let context = h.app.router.active().context_id;
    for id in panes {
        h.app.windows[0].panes.get_mut(&id).unwrap().as_app_mut().unwrap().manifest_id = "agents".into();
    }
    let window = &mut h.app.windows[0];
    let first = window.tree.tiles.find_pane(&panes[0]).unwrap();
    let second = window.tree.tiles.find_pane(&panes[1]).unwrap();
    window.tree.root = Some(window.tree.tiles.insert_tab_tile(vec![first, second]));
    window.focused_pane = Some(first);
    h.app.save_workspace_now();
    let profile = h.profile_dir().to_path_buf();
    let path = crate::workspace::workspace_path_in(&profile);
    let mut legacy: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    legacy["version"] = 2.into();
    legacy.as_object_mut().unwrap().remove("strips");
    legacy.as_object_mut().unwrap().remove("conjoined_contexts");
    let original = serde_json::to_vec(&legacy).unwrap();
    std::fs::write(&path, &original).unwrap();
    let migrated = crate::workspace::WorkspaceFile::load_from(&profile).unwrap();
    assert_eq!(migrated.strips[&context].columns[0].panes, vec![panes[0]]);
    assert_eq!(migrated.windows[0].panes.len(), 2);
    assert!(migrated.windows[0].tree.tiles.find_pane(&panes[1]).is_some());
    assert_eq!(std::fs::read(path.with_extension("v2.rollback.json")).unwrap(), original);
    let restored = HostHarness::from_profile_dir(profile);
    for pane in panes { assert!(restored.app.find_pane_in_any_window(pane).is_some()); }
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

#[test]
fn legacy_grid_conversion_reports_unsupported_and_preserves_file() {
    let mut h = HostHarness::new();
    let panes = [h.add_test_pane(), h.add_test_pane()];
    let context = h.app.router.active().context_id;
    h.app.set_context_columns(context, panes.into_iter().map(|pane| vec![pane]).collect());
    h.app.save_workspace_now();
    let profile = h.profile_dir().to_path_buf();
    let path = crate::workspace::workspace_path_in(&profile);
    let mut legacy = crate::workspace::WorkspaceFile::load_from(&profile).unwrap();
    legacy.version = 2;
    legacy.strips.clear();
    let tree = &mut legacy.windows[0].tree;
    let children: Vec<_> = panes.iter().map(|pane| tree.tiles.find_pane(pane).unwrap()).collect();
    tree.root = Some(tree.tiles.insert_grid_tile(children));
    let original = serde_json::to_vec(&legacy).unwrap();
    std::fs::write(&path, &original).unwrap();
    assert!(crate::workspace::WorkspaceFile::load_from(&profile).is_none());
    assert_eq!(std::fs::read(path).unwrap(), original);
}

#[test]
fn legacy_portal_conversion_preserves_child_and_scope() {
    let mut h = HostHarness::new();
    let parent = h.add_test_pane();
    let child = h.add_test_pane();
    let parent_context = h.app.router.active().context_id;
    for id in [parent, child] {
        h.app.windows[0].panes.get_mut(&id).unwrap().as_app_mut().unwrap().manifest_id = "agents".into();
    }
    h.app.set_context_columns(parent_context, vec![vec![parent], vec![child]]);
    let request = serde_json::from_value(serde_json::json!({"type":"push_pane_to_subcontext", "name":"child", "pane_id":child})).unwrap();
    h.inject_ipc(request);
    h.app.drain_pane_cmd_channel();
    let child_context = h.app.windows.iter().find(|window| window.panes.contains_key(&child)).unwrap().context_id;
    let portal = h.app.windows.iter().flat_map(|window| window.panes.iter()).find_map(|(&id, pane)| (pane.portal_target() == Some(child_context)).then_some(id)).unwrap();
    h.app.save_workspace_now();
    let profile = h.profile_dir().to_path_buf();
    let path = crate::workspace::workspace_path_in(&profile);
    let mut legacy: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    legacy["version"] = 2.into();
    legacy.as_object_mut().unwrap().remove("strips");
    legacy.as_object_mut().unwrap().remove("conjoined_contexts");
    let original = serde_json::to_vec(&legacy).unwrap();
    std::fs::write(&path, &original).unwrap();
    let migrated = crate::workspace::WorkspaceFile::load_from(&profile).unwrap();
    assert!(migrated.windows.iter().any(|window| window.context_id == parent_context && window.panes.iter().any(|pane| pane.id == portal && matches!(pane.kind, crate::workspace::SavedPaneKind::Portal { context_id } if context_id == child_context))));
    assert!(migrated.windows.iter().any(|window| window.context_id == child_context && window.panes.iter().any(|pane| pane.id == child)));
    assert_eq!(std::fs::read(path.with_extension("v2.rollback.json")).unwrap(), original);
    let restored = HostHarness::from_profile_dir(profile);
    assert!(restored.app.windows.iter().any(|window| window.context_id == parent_context && window.panes.get(&portal).is_some_and(|pane| pane.portal_target() == Some(child_context))));
    assert!(restored.app.windows.iter().any(|window| window.context_id == child_context && window.panes.contains_key(&child)));
}

#[test]
fn legacy_zero_window_id_receives_stable_identity() {
    let mut h = HostHarness::new();
    let pane = h.add_test_pane();
    let context = h.app.router.active().context_id;
    h.app.set_context_columns(context, vec![vec![pane]]);
    h.app.focus_pane_in_strip(pane);
    h.app.save_workspace_now();
    let profile = h.profile_dir().to_path_buf();
    let path = crate::workspace::workspace_path_in(&profile);
    let mut legacy: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    legacy["version"] = 2.into();
    legacy.as_object_mut().unwrap().remove("strips");
    legacy.as_object_mut().unwrap().remove("conjoined_contexts");
    legacy["windows"][0]["window_id"] = 0.into();
    legacy["context_active_window"][context.to_string()] = 0.into();
    let original = serde_json::to_vec(&legacy).unwrap();
    std::fs::write(&path, &original).unwrap();
    let migrated = crate::workspace::WorkspaceFile::load_from(&profile).unwrap();
    let window_id = migrated.windows[0].window_id;
    assert_ne!(window_id, 0);
    assert_eq!(migrated.context_active_window[&context], window_id);
    assert_eq!(std::fs::read(path.with_extension("v2.rollback.json")).unwrap(), original);
}

#[test]
fn legacy_deleted_context_active_window_is_migrated_without_losing_workspace() {
    let mut h = HostHarness::new();
    let pane = h.add_test_pane();
    let context = h.app.router.active().context_id;
    h.app.set_context_columns(context, vec![vec![pane]]);
    h.app.focus_pane_in_strip(pane);
    h.app.save_workspace_now();
    let profile = h.profile_dir().to_path_buf();
    let path = crate::workspace::workspace_path_in(&profile);
    let mut legacy: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    legacy["version"] = 2.into();
    legacy.as_object_mut().unwrap().remove("strips");
    legacy.as_object_mut().unwrap().remove("conjoined_contexts");
    let mut conflicting = legacy.clone();
    conflicting["context_active_window"]["999999"] = legacy["windows"][0]["window_id"].clone();
    let conflicting_bytes = serde_json::to_vec(&conflicting).unwrap();
    std::fs::write(&path, &conflicting_bytes).unwrap();
    assert!(crate::workspace::WorkspaceFile::load_from(&profile).is_none());
    assert_eq!(std::fs::read(&path).unwrap(), conflicting_bytes);
    legacy["context_active_window"]["999999"] = 999998.into();
    let original = serde_json::to_vec(&legacy).unwrap();
    std::fs::write(&path, &original).unwrap();
    assert!(
        !h.app.save_workspace_now(),
        "ordinary saves cannot overwrite an unmigrated invalid workspace"
    );
    assert_eq!(std::fs::read(&path).unwrap(), original);

    let migrated = crate::workspace::WorkspaceFile::load_from(&profile)
        .expect("a deleted context's obsolete focus entry must not prevent migration");
    assert_eq!(migrated.version, 3);
    assert!(!migrated.context_active_window.contains_key(&999999));
    assert!(migrated.windows.iter().any(|window| {
        window.context_id == context && window.panes.iter().any(|saved| saved.id == pane)
    }));
    assert_eq!(
        std::fs::read(path.with_extension("v2.rollback.json")).unwrap(),
        original
    );
    migrated
        .save_to(&profile)
        .expect("migrated workspace can be saved again");
    let rollback_parent = tempfile::tempdir().unwrap();
    let rollback_profile = rollback_parent.path().join("rollback");
    crate::workspace::export_v2_rollback(&profile, &rollback_profile).unwrap();
    assert_eq!(
        std::fs::read(crate::workspace::workspace_path_in(&rollback_profile)).unwrap(),
        original
    );
    let restored = HostHarness::from_profile_dir(profile);
    assert!(restored.app.windows.iter().any(|window| {
        window.context_id == context && window.panes.contains_key(&pane)
    }));
}

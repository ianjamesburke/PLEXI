//! Regression fixtures capturing strip defects identified in stint 0985.
//!
//! Covers:
//! 1. Resize persistence across frames (plain source)
//! 2. Divider reset to default presets (plain source)
//! 3. Peek strip click activation / navigation (plain & multi-window sources)
//! 4. Cmd+D column creation from a stacked pane (stacked source)
//! 5. Cmd+D column creation from a vertical-first layout (vertical-first source)
//! 6. Cmd+N left peek visibility across multi-window context pages (multi-window source)
//! 7. Authoritative strip model integration with host state

use crate::strip::{layout_strip, ColumnWidth, StripViewport};
use crate::testing::HostHarness;
use egui_tiles::{Container, Tile};

/// Defect 1: Resizing a column divider must persist across subsequent frames,
/// rather than being overwritten by hardcoded presets on every frame.
#[test]
fn regression_resize_persistence_across_frames() {
    let mut h = HostHarness::new();
    let p1 = h.add_test_pane();
    let p2 = h.add_test_pane();
    let ctx_id = h.app.router.active().context_id;

    h.app.set_context_columns(ctx_id, vec![vec![p1], vec![p2]]);
    h.app.focus_pane_in_strip(p1);
    h.run_frames(2);

    let win = &h.app.windows[h.app.active_window];
    let root_id = win.tree.root().unwrap();
    let strip = h.app.active_window_strip().expect("strip must exist");
    let c0 = strip.children[0];
    let c1 = strip.children[1];

    // User / UI sets custom resize width / share (50/50 instead of 35/65).
    let custom_ratio = 0.50_f32;
    h.app.set_column_width_ratio(ctx_id, 0, custom_ratio);

    h.run_frames(3);

    // Verify custom share persisted across frames and was not overwritten by preset 35/65.
    let win = &h.app.windows[h.app.active_window];
    if let Some(Tile::Container(Container::Linear(lin))) = win.tree.tiles.get(root_id) {
        let sizes = lin.shares.split(&[c0, c1], 1000.0);
        let ratio1 = sizes[0] / (sizes[0] + sizes[1]);
        assert!(
            (ratio1 - 0.50).abs() < 0.05,
            "expected custom share ~0.50 after 3 frames, got ratio1={ratio1:.3} (sizes: {:?})",
            sizes
        );
    } else {
        panic!("expected horizontal linear root");
    }
}

/// Defect 2: Resetting dividers restores default 35/65 presets.
#[test]
fn regression_divider_reset_restores_presets() {
    let mut h = HostHarness::new();
    let p1 = h.add_test_pane();
    let p2 = h.add_test_pane();
    let ctx_id = h.app.router.active().context_id;

    h.app.set_context_columns(ctx_id, vec![vec![p1], vec![p2]]);
    h.app.focus_pane_in_strip(p1);
    h.app.set_column_width_ratio(ctx_id, 0, 0.50);
    h.run_frames(2);

    let strip = h.app.active_window_strip().expect("strip must exist");
    let c0 = strip.children[0];
    let c1 = strip.children[1];

    // Reset column widths to default presets
    h.app.reset_column_widths(ctx_id);
    h.run_frames(2);

    let win = &h.app.windows[h.app.active_window];
    let root_id = win.tree.root().unwrap();

    if let Some(Tile::Container(Container::Linear(lin))) = win.tree.tiles.get(root_id) {
        let sizes = lin.shares.split(&[c0, c1], 1000.0);
        let ratio1 = sizes[0] / (sizes[0] + sizes[1]);
        // Default preset for col 0 is 35% (0.35)
        assert!(
            (ratio1 - 0.35).abs() < 0.05,
            "expected reset ratio ~0.35, got {ratio1:.3} (sizes: {:?})",
            sizes
        );
    } else {
        panic!("expected linear container");
    }
}

/// Defect 3: Clicking the peek strip activates navigation / scrolls to the off-screen column.
#[test]
fn regression_peek_activation_navigates_to_offscreen_column() {
    let mut h = HostHarness::new();
    let p1 = h.add_test_pane();
    let p2 = h.add_test_pane();
    let p3 = h.add_test_pane();
    let ctx_id = h.app.router.active().context_id;

    h.app
        .set_context_columns(ctx_id, vec![vec![p1], vec![p2], vec![p3]]);
    h.app.focus_pane_in_strip(p1);
    h.run_frames(2);

    assert_eq!(h.app.strip_offset_for(ctx_id), 0);
    let (has_left, has_right) = h.app.active_strip_clipped_neighbors();
    assert!(!has_left, "expected no left peek at offset 0");
    assert!(has_right, "expected right peek for column 3");

    // Click right peek
    let handled = h.app.handle_peek_click(false);
    assert!(handled, "expected right peek click to be handled");
    h.run_frames(2);

    // After clicking right peek, strip offset or focus moves to the next column
    assert_eq!(
        h.app.strip_offset_for(ctx_id),
        1,
        "clicking right peek should scroll strip to offset 1"
    );

    // Click left peek returns to offset 0
    let (has_left_after, _) = h.app.active_strip_clipped_neighbors();
    assert!(has_left_after, "expected left peek when at offset 1");
    let handled_left = h.app.handle_peek_click(true);
    assert!(handled_left, "expected left peek click to be handled");
    h.run_frames(2);
    assert_eq!(h.app.strip_offset_for(ctx_id), 0);
}

/// Defect 4: Cmd+D from a stacked pane creates a new column to the right,
/// rather than nesting a horizontal container inside the vertical column stack.
#[test]
fn regression_cmdd_from_stacked_pane_creates_column() {
    let mut h = HostHarness::new();
    let col0_top = h.add_test_pane();
    let col0_bot = h.add_test_pane();
    let ctx_id = h.app.router.active().context_id;

    // Stacked column with 2 panes
    h.app
        .set_context_columns(ctx_id, vec![vec![col0_top, col0_bot]]);
    h.app.focus_pane_in_strip(col0_top);
    h.run_frames(2);

    assert!(matches!(
        h.app.windows[h.app.active_window]
            .tree
            .tiles
            .get(h.app.windows[h.app.active_window].tree.root().unwrap()),
        Some(Tile::Container(Container::Linear(linear)))
            if linear.dir == egui_tiles::LinearDir::Vertical
    ));

    // Press Cmd+D (split horizontal)
    let new_pane = h.app.split_focused(false, None, false, false, None);
    assert!(new_pane.is_some(), "split must produce a pane");
    h.run_frames(2);

    // Columns should now be 2: col 0 has [col0_top, col0_bot], col 1 has [new_pane]
    let tree = &h.app.windows[h.app.active_window].tree;
    let root = tree.root().expect("split must retain a root");
    let Some(Tile::Container(Container::Linear(horizontal))) = tree.tiles.get(root) else {
        panic!("Cmd+D from a stacked pane must promote the tree to horizontal columns");
    };
    assert_eq!(horizontal.dir, egui_tiles::LinearDir::Horizontal);
    assert_eq!(horizontal.children.len(), 2);
    assert!(tree.tiles.find_pane(&new_pane.unwrap()).is_some());
}

/// Defect 4b: Cmd+D from a vertical-first layout models column strip promotion.
#[test]
fn regression_cmdd_from_vertical_first_promotes_to_columns() {
    let mut h = HostHarness::new();
    let p1 = h.add_test_pane();
    let p2 = h.add_test_pane();

    // Construct a vertical-first root: LinearDir::Vertical with [p1, p2]
    let win = &mut h.app.windows[0];
    let t1 = win.tree.tiles.find_pane(&p1).unwrap();
    let t2 = win.tree.tiles.find_pane(&p2).unwrap();
    let v_root = win.tree.tiles.insert_vertical_tile(vec![t1, t2]);
    win.tree.root = Some(v_root);
    win.focused_pane = Some(t1);

    h.run_frames(2);

    // Drive the production Cmd+D path. It must promote the vertical-first tree,
    // rather than nesting a horizontal pair inside the vertical root.
    let new_pane = h.app.split_focused(false, None, false, false, None);
    assert!(new_pane.is_some(), "Cmd+D must create a pane from vertical-first root");
    h.run_frames(2);

    let tree = &h.app.windows[h.app.active_window].tree;
    let root = tree.root().expect("split must retain a root");
    let Some(Tile::Container(Container::Linear(horizontal))) = tree.tiles.get(root) else {
        panic!("Cmd+D from vertical-first root must promote to a horizontal strip");
    };
    assert_eq!(horizontal.dir, egui_tiles::LinearDir::Horizontal);
    assert_eq!(horizontal.children.len(), 2);
    assert!(tree.tiles.find_pane(&p2).is_some());
    assert!(tree.tiles.find_pane(&new_pane.unwrap()).is_some());
}

/// Defect 5: Cmd+N in a single-pane window creates a page right,
/// and the left peek indicator must be visible on the new page.
#[test]
fn regression_cmdn_shows_left_peek_across_windows() {
    let mut h = HostHarness::new();
    let p1 = h.add_test_pane();

    h.app.focus_pane_in_strip(p1);
    h.run_frames(2);

    // Context initially has 1 window with 1 pane
    assert_eq!(h.app.windows.len(), 1);

    // Trigger Cmd+N / new_page_right
    h.app.new_page_right();
    h.run_frames(2);

    // There should now be 2 windows in the same context
    assert_eq!(h.app.windows.len(), 2);
    assert_eq!(h.app.active_window, 1);

    // The left peek indicator must be visible because window 0 is to the left in this context!
    let (has_left, has_right) = h.app.active_strip_clipped_neighbors();
    assert!(
        has_left,
        "Cmd+N must show left peek when prior page exists in context (got has_left={has_left}, has_right={has_right})"
    );
    assert!(!has_right, "rightmost page should have no right peek");
}

/// Verification: Authoritative Strip model integration with host state.
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

    let vp = StripViewport::default();
    let layout = layout_strip(&strip, &vp);
    assert_eq!(layout.panes.len(), 3);
    assert_eq!(layout.dividers.len(), 2); // 1 column divider + 1 vertical stack divider
    assert!(layout.focus_frame.is_some());
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
fn continuous_strip_legacy_context_visible_panes_accept_input() {
    use crate::app::input_owner::InputOwner;
    use crate::host::keys::Direction;
    use crate::ui_tests::PlexiUiHarness;

    let mut h = PlexiUiHarness::new_sized(1280.0, 800.0);
    h.step();

    let tmp = tempfile::tempdir().expect("tempdir");
    let left_file = tmp.path().join("left.md");
    let mid_top_file = tmp.path().join("mid_top.md");
    let mid_bot_file = tmp.path().join("mid_bot.md");
    let right_file = tmp.path().join("right.md");
    std::fs::write(&left_file, "").unwrap();
    std::fs::write(&mid_top_file, "").unwrap();
    std::fs::write(&mid_bot_file, "").unwrap();
    std::fs::write(&right_file, "").unwrap();

    // Open 3 columns + 1 vertical stack through ordinary launch paths (never calling set_context_columns):
    // Col 0: [left_pane]
    // Col 1: [mid_top, mid_bot]
    // Col 2: [right_pane]
    let open_editor = |h: &mut PlexiUiHarness, layout: &str, path: &std::path::Path| -> u64 {
        let before: std::collections::HashSet<u64> = h.with_app(|app| {
            app.windows[app.active_window]
                .panes
                .keys()
                .copied()
                .collect()
        });
        h.with_app_mut(|app| {
            app.launch_app_by_id_with_layout_forced(
                "text-editor",
                Some(layout.to_string()),
                &[path.display().to_string()],
                None,
            )
            .expect("launch text-editor");
        });
        h.run_steps(2);
        h.with_app(|app| {
            app.windows[app.active_window]
                .panes
                .keys()
                .find(|id| !before.contains(id))
                .copied()
                .expect("new text-editor pane id")
        })
    };

    let left_pane = open_editor(&mut h, "split_h", &left_file);
    let mid_top = open_editor(&mut h, "split_h", &mid_top_file);
    let mid_bot = open_editor(&mut h, "split_v", &mid_bot_file);
    let right_pane = open_editor(&mut h, "split_h", &right_file);

    let ctx_id = h.with_app(|app| app.windows[app.active_window].context_id);

    // Remember stable column IDs before any later insertion
    let initial_strip = h.with_app(|app| app.strip_for_context(ctx_id));
    assert_eq!(initial_strip.columns.len(), 3);
    assert_eq!(initial_strip.columns[0].panes, vec![left_pane]);
    assert_eq!(initial_strip.columns[1].panes, vec![mid_top, mid_bot]);
    assert_eq!(initial_strip.columns[2].panes, vec![right_pane]);
    let col0_id = initial_strip.columns[0].id;
    let col1_id = initial_strip.columns[1].id;
    let col2_id = initial_strip.columns[2].id;

    // At this point `right_pane` (Col 2) was just opened, so Col 2 is focused on the right
    // and Col 1 (`mid_top` / `mid_bot`) is partially visible on the left.
    // Clicking `mid_top` inside its visible `hit_rect` must focus AND reveal Col 1;
    // then clicking `mid_bot` focuses it; then scrolling slightly left so `left_pane` (Col 0)
    // is partially visible on the left and clicking `left_pane` inside its `hit_rect` must
    // focus and reveal Col 0 and qualify `left_pane` for unaddressed keyboard input!
    for (pane_id, expected_path, typed_token, partial_scroll_before_click) in [
        (mid_top, &mid_top_file, "MID_TOP_OK", None),
        (mid_bot, &mid_bot_file, "MID_BOT_OK", None),
        (left_pane, &left_file, "LEFT_OK", Some(200.0_f32)),
    ] {
        if let Some(offset_px) = partial_scroll_before_click {
            h.with_app_mut(|app| {
                app.ensure_authoritative_strip(ctx_id)
                    .set_scroll_offset_px(offset_px, None);
            });
            h.run_steps(2);
        }

        let click_pos = h.with_app(|app| {
            let layout = app.active_strip_layout();
            let pane_layout = layout
                .pane(pane_id)
                .expect("pane must be present in active_strip_layout");
            assert!(
                pane_layout.is_visible && pane_layout.hit_rect.width() > 10.0,
                "pane {pane_id} must have a live visible hit_rect before click, got {pane_layout:?}"
            );
            pane_layout.hit_rect.center()
        });

        // Pointer click at the center of the visible hit_rect (no explicit focus_pane call!)
        {
            let events = &mut h.harness().input_mut().events;
            events.push(egui::Event::PointerMoved(click_pos));
            events.push(egui::Event::PointerButton {
                pos: click_pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            });
            events.push(egui::Event::PointerButton {
                pos: click_pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            });
        }
        h.run_steps(2);

        h.with_app(|app| {
            assert_eq!(
                app.focused_pane_id(),
                Some(pane_id),
                "clicking visible pane {pane_id} must focus it in host"
            );
            assert!(
                app.pane_qualifies_for_input(pane_id),
                "clicked visible pane {pane_id} must qualify for input"
            );
            assert_eq!(
                app.host_input_owner(),
                InputOwner::Pane(pane_id),
                "clicked visible pane {pane_id} must own unaddressed host keyboard input"
            );
            let layout = app.active_strip_layout();
            let pane_layout = layout
                .pane(pane_id)
                .expect("pane in strip layout");
            assert!(
                !pane_layout.is_partially_visible,
                "clicking a partially visible pane {pane_id} must reveal its column completely"
            );
            let gap = app.config.pane_gap.unwrap_or(4.0).clamp(0.0, 20.0);
            assert_eq!(
                layout.focus_frame,
                Some(pane_layout.rect.expand(gap)),
                "presentation snapshot focus_frame must match focused pane {pane_id}"
            );
        });

        // Deliver unaddressed text + Cmd+S save through the host frame
        h.harness()
            .input_mut()
            .events
            .push(egui::Event::Text(typed_token.to_string()));
        h.run_steps(2);
        h.harness()
            .key_press_modifiers(egui::Modifiers::MAC_CMD | egui::Modifiers::COMMAND, egui::Key::S);
        h.run_steps(2);

        let saved = std::fs::read_to_string(expected_path).expect("read saved editor file");
        assert!(
            saved.contains(typed_token),
            "unaddressed keyboard input after clicking pane {pane_id} must reach editor buffer; got {saved:?}"
        );
    }

    // Focus `mid_bot` in Col 1, then navigate left to `left_pane` in Col 0, and press Cmd+D
    // (`split_focused(false, ...)`). It must insert a new column at index 1 (immediately right of Col 0),
    // preserve Col 1 (`col1_id`) and Col 2 (`col2_id`) identities and order, and preserve Col 1's
    // remembered pane (`mid_bot`) when navigating right with Cmd+L!
    h.with_app_mut(|app| {
        app.pane_navigate(mid_bot);
        assert!(app.navigate_at_current_zoom_level(Direction::Left));
    });
    h.run_steps(2);
    assert_eq!(h.with_app(|app| app.focused_pane_id()), Some(left_pane));

    let inserted_col_pane = h
        .with_app_mut(|app| app.split_focused(false, None, false, false, None))
        .expect("Cmd+D from leftmost pane must create a pane");
    h.run_steps(2);

    let after_insert_strip = h.with_app(|app| app.strip_for_context(ctx_id));
    assert_eq!(
        after_insert_strip.columns.len(),
        4,
        "Cmd+D from leftmost column must add a 4th column, not overwrite or stack"
    );
    assert_eq!(after_insert_strip.columns[0].id, col0_id);
    assert_eq!(after_insert_strip.columns[0].panes, vec![left_pane]);
    assert_eq!(
        after_insert_strip.columns[1].panes,
        vec![inserted_col_pane],
        "new column must be inserted immediately right of the source column (index 1)"
    );
    assert_eq!(
        after_insert_strip.columns[2].id, col1_id,
        "prior column 1 must retain its stable ColumnId after insertion to its left"
    );
    assert_eq!(after_insert_strip.columns[2].panes, vec![mid_top, mid_bot]);
    assert_eq!(
        after_insert_strip.columns[3].id, col2_id,
        "prior column 2 must retain its stable ColumnId after insertion to its left"
    );
    assert_eq!(after_insert_strip.columns[3].panes, vec![right_pane]);
    assert_eq!(
        h.with_app(|app| app.focused_pane_id()),
        Some(inserted_col_pane)
    );

    // Navigating right (Cmd+L) from `inserted_col_pane` (Col 1) into Col 2 (`col1_id`) must restore
    // `mid_bot` (the last focused pane in `col1_id`), NOT reset to `mid_top`!
    h.with_app_mut(|app| {
        assert!(app.navigate_at_current_zoom_level(Direction::Right));
    });
    h.run_steps(2);
    assert_eq!(
        h.with_app(|app| app.focused_pane_id()),
        Some(mid_bot),
        "Cmd+L into shifted column must restore its remembered pane (mid_bot) via stable ColumnId"
    );

    // Cmd+K / Cmd+J vertical navigation within the stack
    h.with_app_mut(|app| {
        assert!(app.navigate_at_current_zoom_level(Direction::Up));
    });
    h.run_steps(1);
    assert_eq!(h.with_app(|app| app.focused_pane_id()), Some(mid_top));
    h.with_app_mut(|app| {
        assert!(!app.navigate_at_current_zoom_level(Direction::Up), "must not wrap at top of stack");
    });

    // Cmd+Ctrl+H (`swap_pane(Direction::Left)`) from `mid_top` (in Col 2) must exchange positions
    // with `inserted_col_pane` (in Col 1) while keeping focus on `mid_top`!
    h.with_app_mut(|app| {
        app.swap_pane(Direction::Left);
    });
    h.run_steps(2);
    assert_eq!(
        h.with_app(|app| app.focused_pane_id()),
        Some(mid_top),
        "focus must follow the moved pane identity after swap_pane"
    );
    let after_swap_strip = h.with_app(|app| app.strip_for_context(ctx_id));
    assert_eq!(
        after_swap_strip.columns[1].panes,
        vec![mid_top],
        "swap_pane(Left) must swap mid_top into the left neighbor slot"
    );
    assert_eq!(
        after_swap_strip.columns[2].panes,
        vec![inserted_col_pane, mid_bot],
        "swap_pane(Left) must place the displaced neighbor into mid_top's stack slot"
    );

    // Verify intrinsic column widths are not normalized to a visible pair across ordinary focus
    // changes or viewport width changes, and round-trip cleanly across SavedWindow tree persistence.
    let widths_before = h.with_app(|app| {
        let strip = app.strip_for_context(ctx_id);
        strip.intrinsic_column_widths(1280.0, 4.0)
    });
    assert!(
        widths_before[1] + widths_before[2] > 1280.0,
        "two 65% child columns must retain intrinsic widths (>100% total) rather than normalizing to a visible pair: {widths_before:?}"
    );
    let widths_after_resize = h.with_app(|app| {
        let strip = app.strip_for_context(ctx_id);
        strip.intrinsic_column_widths(960.0, 4.0)
    });
    assert!(
        widths_after_resize[1] + widths_after_resize[2] > 960.0,
        "viewport resize must not normalize two 65% child columns to a visible pair: {widths_after_resize:?}"
    );

    // Simulate WorkspaceFile / SavedWindow tree round-trip by clearing the cached strip in
    // HostModel and reconstructing from the persisted window tree.
    let expected_cols: Vec<Vec<u64>> = after_swap_strip
        .columns
        .iter()
        .map(|c| c.panes.clone())
        .collect();
    h.with_app_mut(|app| {
        let saved_tree_json =
            serde_json::to_string(&app.windows[app.active_window].tree).expect("serialize tree");
        let restored_tree: egui_tiles::Tree<u64> =
            serde_json::from_str(&saved_tree_json).expect("deserialize tree");
        app.windows[app.active_window].tree = restored_tree;
        app.host.strips.remove(&ctx_id);
        let rebuilt = app.ensure_authoritative_strip(ctx_id).clone();
        let rebuilt_cols: Vec<Vec<u64>> =
            rebuilt.columns.iter().map(|c| c.panes.clone()).collect();
        assert_eq!(
            rebuilt_cols, expected_cols,
            "restored SavedWindow tree must reconstruct identical strip column and stack membership without duplicate pane ownership"
        );
    });
}


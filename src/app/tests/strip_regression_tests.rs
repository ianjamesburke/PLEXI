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


#[test]
fn oversized_custom_split_preserves_unrelated_width_through_host() {
    let mut h = HostHarness::new();
    let other = h.add_test_pane();
    let source = h.add_test_pane();
    let context = h.app.router.active().context_id;
    h.app.set_context_columns(context, vec![vec![other], vec![source]]);
    h.app.focus_pane_in_strip(source);
    h.run_frames(2);
    let viewport = h.app.active_strip_viewport();
    let width = viewport.canvas_rect.width();
    let strip = h.app.ensure_authoritative_strip(context);
    strip.columns[0].width = ColumnWidth::CustomPixels(250.0);
    strip.columns[1].width = ColumnWidth::CustomPixels(width + 100.0);
    h.app.focus_pane_in_strip(other);
    assert!(h.app.navigate_at_current_zoom_level(crate::host::keys::Direction::Right));
    assert_eq!(h.app.strip_for_context(context).columns[1].width, ColumnWidth::CustomPixels(width + 100.0));
    let new = h.app.split_focused(false, None, false, false, None).expect("split");
    h.run_frames(2);
    let strip = h.app.strip_for_context(context);
    assert_eq!(strip.columns.len(), 3);
    assert_eq!(strip.columns[0].width, ColumnWidth::CustomPixels(250.0));
    let source_idx = strip.find_column_for_pane(source).unwrap();
    let new_idx = strip.find_column_for_pane(new).unwrap();
    let widths = strip.intrinsic_column_widths(width, viewport.gap);
    assert!(widths[source_idx] + viewport.gap + widths[new_idx] <= width + 0.5);
    let layout = layout_strip(&strip, &viewport);
    assert!(layout.pane(source).unwrap().is_on_screen);
    assert!(layout.pane(new).unwrap().is_on_screen);
}

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
        let (col, row) = strip.find_pane_position(pane).unwrap();
        assert_eq!(strip.columns[col].heights[row], expected, "share after split pane={pane}");
    }
    assert_eq!(h.app.focused_pane_id(), Some(inserted));
    h.app.close_focused();
    h.run_frames(2);
    let strip = h.app.strip_for_context(context);
    for (pane, expected) in [(first, 2.0), (second, 3.0)] {
        let (col, row) = strip.find_pane_position(pane).unwrap();
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
        strip.set_scroll_offset_px(200.0, Some(600.0));
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
        strip.set_scroll_offset_px(450.0, Some(canvas));
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

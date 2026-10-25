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

    assert_eq!(h.app.zoom.strip_offset_for(ctx_id), 0);
    let (has_left, has_right) = h.app.active_strip_clipped_neighbors();
    assert!(!has_left, "expected no left peek at offset 0");
    assert!(has_right, "expected right peek for column 3");

    // Click right peek
    let handled = h.app.handle_peek_click(false);
    assert!(handled, "expected right peek click to be handled");
    h.run_frames(2);

    // After clicking right peek, strip offset or focus moves to the next column
    assert_eq!(
        h.app.zoom.strip_offset_for(ctx_id),
        1,
        "clicking right peek should scroll strip to offset 1"
    );

    // Click left peek returns to offset 0
    let (has_left_after, _) = h.app.active_strip_clipped_neighbors();
    assert!(has_left_after, "expected left peek when at offset 1");
    let handled_left = h.app.handle_peek_click(true);
    assert!(handled_left, "expected left peek click to be handled");
    h.run_frames(2);
    assert_eq!(h.app.zoom.strip_offset_for(ctx_id), 0);
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

    assert_eq!(h.app.columns_for_context(ctx_id).len(), 1);

    // Press Cmd+D (split horizontal)
    let new_pane = h.app.split_focused(false, None, false, false, None);
    assert!(new_pane.is_some(), "split must produce a pane");
    h.run_frames(2);

    // Columns should now be 2: col 0 has [col0_top, col0_bot], col 1 has [new_pane]
    let cols = h.app.columns_for_context(ctx_id);
    assert_eq!(
        cols.len(),
        2,
        "Cmd+D from stacked pane must create a second column, got {:?}",
        cols
    );
    assert_eq!(cols[0].panes, vec![col0_top, col0_bot]);
    assert_eq!(cols[1].panes, vec![new_pane.unwrap()]);
}

/// Defect 4b: Cmd+D from a vertical-first layout models column strip promotion.
#[test]
fn regression_cmdd_from_vertical_first_promotes_to_columns() {
    let mut h = HostHarness::new();
    let p1 = h.add_test_pane();
    let p2 = h.add_test_pane();
    let ctx_id = h.app.router.active().context_id;

    // Construct a vertical-first root: LinearDir::Vertical with [p1, p2]
    let win = &mut h.app.windows[0];
    let t1 = win.tree.tiles.find_pane(&p1).unwrap();
    let t2 = win.tree.tiles.find_pane(&p2).unwrap();
    let v_root = win.tree.tiles.insert_vertical_tile(vec![t1, t2]);
    win.tree.root = Some(v_root);
    win.focused_pane = Some(t1);

    h.run_frames(2);

    // Initial vertical-first tree forms column 0 with a stacked pair [p1, p2]
    let mut strip = h.app.strip_for_context(ctx_id);
    assert_eq!(strip.column_count(), 1);
    assert_eq!(strip.columns[0].panes, vec![p1, p2]);

    // Split horizontal (Cmd+D) promotes to a new column right
    let new_col = strip.insert_column_right_of(
        0,
        999,
        ColumnWidth::Preset(crate::app::zoom::ColumnWidthPreset::Child65),
    );
    assert_eq!(new_col, 1);
    assert_eq!(strip.column_count(), 2);
    assert_eq!(strip.columns[0].panes, vec![p1, p2]);
    assert_eq!(strip.columns[1].panes, vec![999]);
    assert_eq!(strip.focused_pane, Some(999));
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

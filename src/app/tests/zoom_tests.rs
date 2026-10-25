//! Tests for the Zoom Ladder, Niri Scrolling Columns, Sibling Cards, Tree Atlas,
//! Attention Queue Focus-Lock Rule, Decision Overlay, Context Restore, and Strict Input Ownership
//! (`SPEC.md` R31–R42).

use crate::app::attention_queue::{QueueCategory, QueueOption, OVERLAY_QUESTION};
use crate::app::input_router::PlexiInput;
use crate::app::zoom::{ColumnWidthPreset, ZoomLevel};
use crate::host::context::Window;
use crate::host::keys::Direction;
use crate::testing::HostHarness;
use crate::ui::style;
use egui::{Color32, Event, Key, Modifiers, Pos2, Rect, Vec2};
use std::collections::HashMap;

fn send_key(h: &mut HostHarness, modifiers: Modifiers, key: Key) {
    let ev = Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    };
    let mut input = PlexiInput::synthetic(vec![ev], modifiers);
    if h.app.attention_queue.decision_overlay.is_some() {
        let _ = h.app.handle_decision_overlay_input(&mut input);
        return;
    }
    if h.app.attention_queue.open {
        let _ = h.app.handle_attention_queue_list_input(&mut input);
        return;
    }
    if h.app.zoom.level != ZoomLevel::Columns {
        let _ = h.app.handle_zoom_surface_input(&mut input);
    }
}

/// R31: Zoom ladder round-trip (`Columns -> SiblingCards -> TreeAtlas -> SiblingCards -> Columns`)
/// records the ghost marker on zoom-out and restores the exact origin pane and scroll offset on return.
/// Both ends of the ladder no-op with an edge bump.
#[test]
fn zoom_ladder_round_trip_and_ghost_restore_r31() {
    let mut h = HostHarness::new();
    let p1 = h.add_test_pane();
    let p2 = h.add_test_pane();
    let p3 = h.add_test_pane();
    let ctx_id = h.app.router.active().context_id;

    h.app
        .set_context_columns(ctx_id, vec![vec![p1], vec![p2], vec![p3]]);
    h.app.focus_pane_in_strip(p2);
    assert_eq!(h.app.zoom.level, ZoomLevel::Columns);
    assert_eq!(h.app.active_focused_pane_id(), Some(p2));

    // At Columns with multiple panes, Cmd+Enter toggles focused pane zoom.
    assert!(h.app.zoom_in_level());
    assert_eq!(h.app.zoom.level, ZoomLevel::Columns);
    assert!(h.app.windows[0].zoomed_pane.is_some());
    // Second Cmd+Enter restores prior geometry/unzooms.
    assert!(h.app.zoom_in_level());
    assert_eq!(h.app.zoom.level, ZoomLevel::Columns);
    assert!(h.app.windows[0].zoomed_pane.is_none());

    // Cmd+Esc: Columns -> SiblingCards, recording ghost marker at (ctx_id, col=1, pane=p2).
    assert!(h.app.zoom_out_or_close_surface());
    assert_eq!(h.app.zoom.level, ZoomLevel::SiblingCards);
    let ghost = h.app.zoom.ghost.clone().expect("ghost recorded");
    assert_eq!(ghost.context_id, ctx_id);
    assert_eq!(ghost.column_idx, 1);
    assert_eq!(ghost.pane_id, p2);

    // Cmd+Esc again: SiblingCards -> TreeAtlas.
    assert!(h.app.zoom_out_or_close_surface());
    assert_eq!(h.app.zoom.level, ZoomLevel::TreeAtlas);

    // Cmd+Esc at TreeAtlas -> no-op + edge bump.
    let bumps_atlas = h.app.zoom.edge_bump_count;
    assert!(!h.app.zoom_out_or_close_surface());
    assert_eq!(h.app.zoom.level, ZoomLevel::TreeAtlas);
    assert_eq!(h.app.zoom.edge_bump_count, bumps_atlas + 1);

    // Cmd+Enter: TreeAtlas -> SiblingCards.
    assert!(h.app.zoom_in_level());
    assert_eq!(h.app.zoom.level, ZoomLevel::SiblingCards);

    // Cmd+Enter: SiblingCards -> Columns, restoring exact ghost pane p2.
    assert!(h.app.zoom_in_level());
    assert_eq!(h.app.zoom.level, ZoomLevel::Columns);
    assert_eq!(h.app.active_focused_pane_id(), Some(p2));
    assert!(h.app.zoom.ghost.is_none());
}

/// R31 / Q7: When the ghost pane is closed while zoomed out in the Tree Atlas,
/// pressing `q` falls back cleanly to a surviving column neighbour in that context.
#[test]
fn ghost_restore_falls_back_when_origin_pane_closed_r31() {
    let mut h = HostHarness::new();
    let p1 = h.add_test_pane();
    let p2 = h.add_test_pane();
    let ctx_id = h.app.router.active().context_id;
    h.app.set_context_columns(ctx_id, vec![vec![p1, p2]]);
    h.app.focus_pane_in_strip(p2);

    h.app.zoom_out_or_close_surface();
    h.app.zoom_out_or_close_surface();
    assert_eq!(h.app.zoom.level, ZoomLevel::TreeAtlas);

    // Remove p2 while zoomed out.
    h.app.windows[0].panes.remove(&p2);
    if let Some(tile) = h.app.windows[0].tree.tiles.find_pane(&p2) {
        h.app.windows[0].tree.tiles.remove(tile);
    }

    // Press `q` in TreeAtlas -> restores surviving neighbour p1 at Columns.
    send_key(&mut h, Modifiers::NONE, Key::Q);
    assert_eq!(h.app.zoom.level, ZoomLevel::Columns);
    assert_eq!(h.app.active_focused_pane_id(), Some(p1));
}

/// R32: Niri-style column strip navigation (`Cmd+H/L` across columns, `Cmd+J/K` within a vertical stack),
/// per-column last-focused pane memory, viewport scrolling (`strip_offsets`), and edge bumps without wrapping.
#[test]
fn niri_columns_strip_navigation_memory_and_edge_bumps_r32() {
    let mut h = HostHarness::new();
    let c0_top = h.add_test_pane();
    let c0_bot = h.add_test_pane();
    let c1_top = h.add_test_pane();
    let c1_bot = h.add_test_pane();
    let c2_only = h.add_test_pane();
    let ctx_id = h.app.router.active().context_id;

    h.app.set_context_columns(
        ctx_id,
        vec![vec![c0_top, c0_bot], vec![c1_top, c1_bot], vec![c2_only]],
    );
    h.app.focus_pane_in_strip(c0_top);
    assert_eq!(h.app.zoom.strip_offset_for(ctx_id), 0);

    // Left at leftmost column -> edge bump, no wrap.
    let b0 = h.app.zoom.edge_bump_count;
    assert!(!h.app.navigate_columns_strip(Direction::Left));
    assert_eq!(h.app.zoom.edge_bump_count, b0 + 1);
    assert_eq!(h.app.active_focused_pane_id(), Some(c0_top));

    // Up at top of column 0 -> edge bump, no wrap.
    assert!(!h.app.navigate_columns_strip(Direction::Up));
    assert_eq!(h.app.zoom.edge_bump_count, b0 + 2);

    // Down in column 0 -> c0_bot.
    assert!(h.app.navigate_columns_strip(Direction::Down));
    assert_eq!(h.app.active_focused_pane_id(), Some(c0_bot));

    // Down at bottom of column 0 -> edge bump, no wrap.
    assert!(!h.app.navigate_columns_strip(Direction::Down));
    assert_eq!(h.app.zoom.edge_bump_count, b0 + 3);

    // Right to column 1 -> c1_top; Down in column 1 -> c1_bot.
    assert!(h.app.navigate_columns_strip(Direction::Right));
    assert_eq!(h.app.active_focused_pane_id(), Some(c1_top));
    assert!(h.app.navigate_columns_strip(Direction::Down));
    assert_eq!(h.app.active_focused_pane_id(), Some(c1_bot));

    // Right to column 2 -> scrolls strip offset to 1 (columns 1 and 2 visible, column 0 clipped left).
    assert!(h.app.navigate_columns_strip(Direction::Right));
    assert_eq!(h.app.active_focused_pane_id(), Some(c2_only));
    assert_eq!(h.app.zoom.strip_offset_for(ctx_id), 1);
    assert_eq!(h.app.active_strip_clipped_neighbors(), (true, false));

    // Right at rightmost column -> edge bump, no wrap.
    let b1 = h.app.zoom.edge_bump_count;
    assert!(!h.app.navigate_columns_strip(Direction::Right));
    assert_eq!(h.app.zoom.edge_bump_count, b1 + 1);

    // Left back to column 1 -> remembers c1_bot!
    assert!(h.app.navigate_columns_strip(Direction::Left));
    assert_eq!(h.app.active_focused_pane_id(), Some(c1_bot));

    // Left back to column 0 -> remembers c0_bot and scrolls strip offset back to 0!
    assert!(h.app.navigate_columns_strip(Direction::Left));
    assert_eq!(h.app.active_focused_pane_id(), Some(c0_bot));
    assert_eq!(h.app.zoom.strip_offset_for(ctx_id), 0);
    assert_eq!(h.app.active_strip_clipped_neighbors(), (false, true));
}

#[test]
fn strip_edge_navigation_is_a_render_and_state_noop_across_windows() {
    let mut h = HostHarness::new();
    let first = h.add_test_pane();
    let middle = h.add_test_pane();
    let last = h.add_test_pane();
    let context_id = h.app.router.active().context_id;
    h.app
        .set_context_columns(context_id, vec![vec![first], vec![middle], vec![last]]);
    h.app.windows.push(Window {
        name: "same context".into(),
        path: crate::testing::scratch_context_root("strip-edge-second-window"),
        tree: egui_tiles::Tree::empty("strip_edge_second_window"),
        panes: HashMap::new(),
        focused_pane: None,
        zoomed_pane: None,
        grid_x: 1,
        grid_y: 0,
        window_id: 9_999,
        context_id,
    });

    h.app.focus_pane_in_strip(first);
    h.run_frames(2);
    for direction in [Direction::Left, Direction::Left, Direction::Left] {
        let focused = h.app.active_focused_pane_id();
        let active_window = h.app.active_window;
        let offset = h.app.zoom.strip_offset_for(context_id);
        let signature = h.app.zoom.last_strip_sig.clone();
        let jump = h.app.zoom.strip_layout_jump;
        let transition = h.app.zoom.transition;
        let rect = h.app.windows[0]
            .tree
            .tiles
            .rect(h.app.windows[0].tree.tiles.find_pane(&first).unwrap());
        assert!(!h.app.navigate_columns_strip(direction));
        for _ in 0..6 {
            h.run_frames(1);
            assert_eq!(h.app.active_focused_pane_id(), focused);
            assert_eq!(h.app.active_window, active_window);
            assert_eq!(h.app.zoom.strip_offset_for(context_id), offset);
            assert_eq!(h.app.zoom.last_strip_sig, signature);
            assert_eq!(h.app.zoom.strip_layout_jump, jump);
            assert_eq!(h.app.zoom.transition, transition);
            assert!(h.app.pane_anims.is_empty());
            assert!(h.app.edge_pulse.is_none());
            assert_eq!(
                h.app.windows[0]
                    .tree
                    .tiles
                    .rect(h.app.windows[0].tree.tiles.find_pane(&first).unwrap()),
                rect
            );
        }
    }

    assert!(h.app.navigate_columns_strip(Direction::Right));
    assert!(h.app.navigate_columns_strip(Direction::Right));
    h.run_frames(2);
    for direction in [Direction::Right, Direction::Right, Direction::Right] {
        let focused = h.app.active_focused_pane_id();
        let active_window = h.app.active_window;
        let offset = h.app.zoom.strip_offset_for(context_id);
        let signature = h.app.zoom.last_strip_sig.clone();
        let jump = h.app.zoom.strip_layout_jump;
        let transition = h.app.zoom.transition;
        let rect = h.app.windows[0]
            .tree
            .tiles
            .rect(h.app.windows[0].tree.tiles.find_pane(&last).unwrap());
        assert!(!h.app.navigate_columns_strip(direction));
        for _ in 0..6 {
            h.run_frames(1);
            assert_eq!(h.app.active_focused_pane_id(), focused);
            assert_eq!(h.app.active_window, active_window);
            assert_eq!(h.app.zoom.strip_offset_for(context_id), offset);
            assert_eq!(h.app.zoom.last_strip_sig, signature);
            assert_eq!(h.app.zoom.strip_layout_jump, jump);
            assert_eq!(h.app.zoom.transition, transition);
            assert!(h.app.pane_anims.is_empty());
            assert!(h.app.edge_pulse.is_none());
            assert_eq!(
                h.app.windows[0]
                    .tree
                    .tiles
                    .rect(h.app.windows[0].tree.tiles.find_pane(&last).unwrap()),
                rect
            );
        }
    }
}

/// R33: Fixed column width presets (`35%` parent / `65%` child) and tile content tiers.
#[test]
fn column_width_presets_and_tile_content_tiers_r33() {
    let mut h = HostHarness::new();
    let p1 = h.add_test_pane();
    let p2 = h.add_test_pane();
    let ctx_id = h.app.router.active().context_id;
    h.app.set_context_columns(ctx_id, vec![vec![p1], vec![p2]]);

    let cols = h.app.columns_for_context(ctx_id);
    assert_eq!(cols.len(), 2);
    assert_eq!(cols[0].preset, ColumnWidthPreset::Parent35);
    assert_eq!(cols[0].preset.ratio(), 0.35);
    assert_eq!(cols[1].preset, ColumnWidthPreset::Child65);
    assert_eq!(cols[1].preset.ratio(), 0.65);
}

/// R34 & R36: Sibling Context Cards (`SiblingCards`) traversal with bare `h`/`l`
/// and `Enter` restoring the target context's last active pane.
#[test]
fn sibling_cards_navigation_and_context_restore_r34_r36() {
    let mut h = HostHarness::new();
    let ctx0_p1 = h.add_test_pane();
    let ctx0_p2 = h.add_test_pane();
    let ctx0_id = h.app.router.active().context_id;
    h.app
        .set_context_columns(ctx0_id, vec![vec![ctx0_p1], vec![ctx0_p2]]);
    h.app.focus_pane_in_strip(ctx0_p2);

    // Create a sibling context (ctx1) with two panes and focus its second pane.
    let ctx1_id = h.app.create_context_without_pty("ctx-1", None);
    let ctx1_p1 = h.add_test_pane();
    let ctx1_p2 = h.add_test_pane();
    h.app
        .set_context_columns(ctx1_id, vec![vec![ctx1_p1], vec![ctx1_p2]]);
    h.app.focus_pane_in_strip(ctx1_p2);

    // Switch back to ctx0 -> R36 must restore ctx0_p2!
    h.app.switch_to_context_restoring_last_pane(ctx0_id);
    assert_eq!(h.app.router.active().context_id, ctx0_id);
    assert_eq!(h.app.active_focused_pane_id(), Some(ctx0_p2));

    // Zoom out to SiblingCards.
    h.app.zoom_out_or_close_surface();
    assert_eq!(h.app.zoom.level, ZoomLevel::SiblingCards);
    assert_eq!(h.app.sibling_card_contexts(), vec![ctx0_id, ctx1_id]);
    assert_eq!(h.app.zoom.cards_focused_context, Some(ctx0_id));

    // Bare `h` at leftmost card -> edge bump.
    let b = h.app.zoom.edge_bump_count;
    send_key(&mut h, Modifiers::NONE, Key::H);
    assert_eq!(h.app.zoom.edge_bump_count, b + 1);

    // Bare `l` moves card focus to ctx1_id; bare `Enter` enters ctx1 at ctx1_p2!
    send_key(&mut h, Modifiers::NONE, Key::L);
    assert_eq!(h.app.zoom.cards_focused_context, Some(ctx1_id));
    send_key(&mut h, Modifiers::NONE, Key::Enter);
    assert_eq!(h.app.zoom.level, ZoomLevel::Columns);
    assert_eq!(h.app.router.active().context_id, ctx1_id);
    assert_eq!(h.app.active_focused_pane_id(), Some(ctx1_p2));
}

/// Stint 0988: Sibling cards selection-follow updates viewport offset, and pointer scrolling can be recovered by navigation.
#[test]
fn sibling_cards_selection_follow_and_pointer_scrolling() {
    let mut h = HostHarness::new();
    let ctx0_id = h.app.router.active().context_id;
    let p0 = h.add_test_pane();
    h.app.set_context_columns(ctx0_id, vec![vec![p0]]);

    let mut sibs = vec![ctx0_id];
    for i in 1..5 {
        let cid = h.app.create_context_without_pty(&format!("ctx-{i}"), None);
        let pid = h.add_test_pane();
        h.app.set_context_columns(cid, vec![vec![pid]]);
        sibs.push(cid);
    }

    h.app.switch_to_context_restoring_last_pane(ctx0_id);
    h.app.zoom_out_or_close_surface();
    assert_eq!(h.app.zoom.level, ZoomLevel::SiblingCards);
    assert_eq!(h.app.zoom.cards_focused_context, Some(ctx0_id));

    // Move Right with L to card 1
    assert!(h.app.navigate_sibling_cards(Direction::Right));
    assert_eq!(h.app.zoom.cards_focused_context, Some(sibs[1]));
    let expected_offset_1 = 1.0 * (style::CARD_W + style::CARD_GAP) + style::CARD_W * 0.5;
    assert_eq!(h.app.zoom.cards_viewport_offset, expected_offset_1);

    // Simulate pointer scroll adjusting viewport offset
    h.app.zoom.cards_viewport_offset += 50.0;
    assert_eq!(h.app.zoom.cards_viewport_offset, expected_offset_1 + 50.0);

    // Navigating with keyboard (Left to card 0) triggers selection-follow and snaps back to centered card 0!
    assert!(h.app.navigate_sibling_cards(Direction::Left));
    assert_eq!(h.app.zoom.cards_focused_context, Some(ctx0_id));
    let expected_offset_0 = 0.0 * (style::CARD_W + style::CARD_GAP) + style::CARD_W * 0.5;
    assert_eq!(h.app.zoom.cards_viewport_offset, expected_offset_0);
}

/// R35: Tree Atlas left-to-right tidy tree (`parent.x < child.x`), `h`/`l`/`j`/`k`,
/// `[`/`]` sibling wrap, `n`/`N` needs-you wrap, `p` peek, `q` ghost return,
/// and `Enter` dive (asking node -> asking pane + DecisionOverlay open; calm node -> last active pane).
#[test]
fn tree_atlas_layout_traversal_and_dive_r35() {
    let mut h = HostHarness::new();
    let root_pane = h.add_test_pane();
    let root_ctx_id = h.app.router.active().context_id;
    h.app
        .set_context_columns(root_ctx_id, vec![vec![root_pane]]);
    h.app.focus_pane_in_strip(root_pane);

    // Add two child contexts under root_ctx_id.
    let child_a_id = h
        .app
        .create_context_without_pty("child-a", Some(root_ctx_id));
    let child_a_pane = h.add_test_pane();
    h.app
        .set_context_columns(child_a_id, vec![vec![child_a_pane]]);
    h.app.focus_pane_in_strip(child_a_pane);

    let child_b_id = h
        .app
        .create_context_without_pty("child-b", Some(root_ctx_id));
    let child_b_p1 = h.add_test_pane();
    let child_b_p2 = h.add_test_pane();
    h.app
        .set_context_columns(child_b_id, vec![vec![child_b_p1], vec![child_b_p2]]);
    h.app.focus_pane_in_strip(child_b_p2);

    // Enqueue an Ask on child_b_p2.
    h.app.enqueue_ask(
        "ask-child-b",
        child_b_p2,
        "cargo publish --dry-run",
        1_000,
        Vec::new(),
    );
    // Arrival must NEVER auto-open the DecisionOverlay (R39).
    assert!(h.app.attention_queue.decision_overlay.is_none());

    // Return to root_ctx_id and zoom out twice to TreeAtlas.
    h.app.switch_to_context_restoring_last_pane(root_ctx_id);
    h.app.zoom_out_or_close_surface();
    h.app.zoom_out_or_close_surface();
    assert_eq!(h.app.zoom.level, ZoomLevel::TreeAtlas);
    assert_eq!(h.app.zoom.atlas_focused_context, Some(root_ctx_id));

    // Verify top-down tree layout: single root at top-center, children on row below, and root rollup includes child_b's ask.
    let viewport = Rect::from_min_size(Pos2::ZERO, Vec2::new(1440.0, 900.0));
    let (nodes, edges) = h.app.compute_atlas_graph(viewport);
    let root_node = nodes.iter().find(|n| n.context_id == root_ctx_id).unwrap();
    let child_a_node = nodes.iter().find(|n| n.context_id == child_a_id).unwrap();
    let child_b_node = nodes.iter().find(|n| n.context_id == child_b_id).unwrap();
    assert!(
        (root_node.rect.center().x - viewport.center().x).abs() < 1.0,
        "single root must sit horizontally centered at top-center"
    );
    assert!(
        root_node.rect.max.y < child_a_node.rect.min.y,
        "single root must sit above its children"
    );
    assert!(
        (child_a_node.rect.center().y - child_b_node.rect.center().y).abs() < 1.0,
        "children at same depth must sit on the same row"
    );
    assert!(child_a_node.rect.center().x < child_b_node.rect.center().x);
    assert!(!root_node.rect.intersects(child_a_node.rect));
    assert!(!root_node.rect.intersects(child_b_node.rect));
    assert!(!child_a_node.rect.intersects(child_b_node.rect));
    assert!(edges
        .iter()
        .any(|e| e.from_context_id == root_ctx_id && e.to_context_id == child_a_id));
    assert!(
        !edges.iter().any(|e| e.from_context_id == 0),
        "single root must not have virtual hub edges"
    );
    assert_eq!(root_node.ask_rollup, 1);
    assert_eq!(child_b_node.direct_asks, 1);
    assert!(!edges.is_empty());

    // `j` moves down from root to first child (child_a).
    send_key(&mut h, Modifiers::NONE, Key::J);
    assert_eq!(h.app.zoom.atlas_focused_context, Some(child_a_id));

    // `l` moves right to neighboring sibling (child_b); `l` again bumps edge (no wrap).
    send_key(&mut h, Modifiers::NONE, Key::L);
    assert_eq!(h.app.zoom.atlas_focused_context, Some(child_b_id));
    let b = h.app.zoom.edge_bump_count;
    send_key(&mut h, Modifiers::NONE, Key::L);
    assert_eq!(h.app.zoom.edge_bump_count, b + 1);

    // `]` wraps around siblings from child_b back to child_a.
    send_key(&mut h, Modifiers::NONE, Key::CloseBracket);
    assert_eq!(h.app.zoom.atlas_focused_context, Some(child_a_id));

    // `k` returns to parent (root), and `j` remembers last-visited child (child_a).
    send_key(&mut h, Modifiers::NONE, Key::K);
    assert_eq!(h.app.zoom.atlas_focused_context, Some(root_ctx_id));
    send_key(&mut h, Modifiers::NONE, Key::J);
    assert_eq!(h.app.zoom.atlas_focused_context, Some(child_a_id));

    // `p` opens Peek card; Cmd+Esc closes Peek first without leaving TreeAtlas.
    send_key(&mut h, Modifiers::NONE, Key::P);
    assert!(h.app.zoom.atlas_peek_open);
    assert!(h.app.zoom_out_or_close_surface());
    assert!(!h.app.zoom.atlas_peek_open);
    assert_eq!(h.app.zoom.level, ZoomLevel::TreeAtlas);

    // `n` jumps directly to the node needing you (child_b_id).
    send_key(&mut h, Modifiers::NONE, Key::N);
    assert_eq!(h.app.zoom.atlas_focused_context, Some(child_b_id));

    // `Enter` on asking node child_b dives to Columns, focuses child_b_p2, AND opens DecisionOverlay!
    send_key(&mut h, Modifiers::NONE, Key::Enter);
    assert_eq!(h.app.zoom.level, ZoomLevel::Columns);
    assert_eq!(h.app.router.active().context_id, child_b_id);
    assert_eq!(h.app.active_focused_pane_id(), Some(child_b_p2));
    let ov = h
        .app
        .attention_queue
        .decision_overlay
        .as_ref()
        .expect("DecisionOverlay opened on dive into asking node");
    assert_eq!(ov.ask_id, "ask-child-b");
    assert_eq!(ov.pane_id, child_b_p2);
}

#[test]
fn tree_atlas_layout_multi_root_hub_top_down() {
    let mut h = HostHarness::new();
    let root1_pane = h.add_test_pane();
    let root1_id = h.app.router.active().context_id;
    h.app.set_context_columns(root1_id, vec![vec![root1_pane]]);

    let child1_id = h.app.create_context_without_pty("child-1", Some(root1_id));
    let child1_pane = h.add_test_pane();
    h.app
        .set_context_columns(child1_id, vec![vec![child1_pane]]);

    let root2_id = h.app.create_context_without_pty("root-2", None);
    let root2_pane = h.add_test_pane();
    h.app.set_context_columns(root2_id, vec![vec![root2_pane]]);

    h.app.switch_to_context_restoring_last_pane(root1_id);
    h.app.zoom_out_or_close_surface();
    h.app.zoom_out_or_close_surface();
    assert_eq!(h.app.zoom.level, ZoomLevel::TreeAtlas);

    let viewport = Rect::from_min_size(Pos2::ZERO, Vec2::new(1440.0, 900.0));
    let hub_rect = crate::app::PlexiApp::atlas_hub_rect(viewport);
    let (nodes, edges) = h.app.compute_atlas_graph(viewport);

    let r1 = nodes.iter().find(|n| n.context_id == root1_id).unwrap();
    let r2 = nodes.iter().find(|n| n.context_id == root2_id).unwrap();
    let c1 = nodes.iter().find(|n| n.context_id == child1_id).unwrap();

    // Hub pill sits at top-center above row 1 roots.
    assert!((hub_rect.center().x - viewport.center().x).abs() < 1.0);
    assert!(hub_rect.max.y < r1.rect.min.y);
    assert!(hub_rect.max.y < r2.rect.min.y);

    // Roots sit on row 1 side-by-side; child sits on row 2 below root 1.
    assert!((r1.rect.center().y - r2.rect.center().y).abs() < 1.0);
    assert!(r1.rect.center().x < r2.rect.center().x);
    assert!(r1.rect.max.y < c1.rect.min.y);

    // Hub edges connect hub bottom-center to each root's top-center.
    let hub_bottom = Pos2::new(hub_rect.center().x, hub_rect.max.y);
    for root in [r1, r2] {
        let edge = edges
            .iter()
            .find(|e| e.from_context_id == 0 && e.to_context_id == root.context_id)
            .expect("hub edge to root");
        assert_eq!(edge.from_pos, hub_bottom);
        assert_eq!(
            edge.to_pos,
            Pos2::new(root.rect.center().x, root.rect.min.y)
        );
    }

    // Keyboard nav works across roots (h/l neighboring siblings) and down to child (j).
    assert_eq!(h.app.zoom.atlas_focused_context, Some(root1_id));
    send_key(&mut h, Modifiers::NONE, Key::L);
    assert_eq!(h.app.zoom.atlas_focused_context, Some(root2_id));
    send_key(&mut h, Modifiers::NONE, Key::H);
    assert_eq!(h.app.zoom.atlas_focused_context, Some(root1_id));
    send_key(&mut h, Modifiers::NONE, Key::J);
    assert_eq!(h.app.zoom.atlas_focused_context, Some(child1_id));
    send_key(&mut h, Modifiers::NONE, Key::K);
    assert_eq!(h.app.zoom.atlas_focused_context, Some(root1_id));
}

/// R37 & R38: Attention queue category ordering (`Ask -> Failed -> Result -> Done`, oldest first)
/// and the Focus-Lock Rule (focused pane's unresolved ask stays at index 0 with `locked = true`
/// even when older-timestamp or new asks arrive, until answered or focus moves).
#[test]
fn attention_queue_ordering_and_focus_lock_rule_r37_r38() {
    let mut h = HostHarness::new();
    let pane_a = h.add_test_pane();
    let pane_b = h.add_test_pane();
    let ctx_id = h.app.router.active().context_id;
    h.app
        .set_context_columns(ctx_id, vec![vec![pane_a], vec![pane_b]]);
    h.app.focus_pane_in_strip(pane_a);

    // Enqueue Done, Result, Failed, and an Ask on focused pane_a at t=5000.
    h.app
        .enqueue_queue_item("done-1", QueueCategory::Done, pane_b, "Finished build", 100);
    h.app.enqueue_queue_item(
        "res-1",
        QueueCategory::Result,
        pane_b,
        "Subagent summary ready",
        200,
    );
    h.app.enqueue_queue_item(
        "fail-1",
        QueueCategory::Failed,
        pane_b,
        "Test suite exited 1",
        300,
    );
    h.app
        .enqueue_ask("ask-pane-a", pane_a, "rm -rf target/", 5_000, Vec::new());

    // Focused pane_a's ask is item 0 and locked!
    assert_eq!(h.app.attention_queue.items[0].id, "ask-pane-a");
    assert!(h.app.attention_queue.items[0].locked);

    // Now an ask with an OLDER timestamp (t=1000) arrives from unfocused pane_b.
    // Because pane_a is focused and has a locked ask, `ask-pane-a` MUST stay at index 0!
    h.app.enqueue_ask(
        "ask-pane-b",
        pane_b,
        "git push origin main",
        1_000,
        Vec::new(),
    );
    let ids: Vec<&str> = h
        .app
        .attention_queue
        .items
        .iter()
        .map(|b| b.id.as_str())
        .collect();
    assert_eq!(
        ids,
        vec!["ask-pane-a", "ask-pane-b", "fail-1", "res-1", "done-1"]
    );
    assert!(h.app.attention_queue.items[0].locked);
    assert!(!h.app.attention_queue.items[1].locked);

    // Move focus away from pane_a to pane_b -> lock releases from pane_a and locks pane_b's ask!
    h.app.focus_pane_in_strip(pane_b);
    assert_eq!(h.app.attention_queue.items[0].id, "ask-pane-b");
    assert!(h.app.attention_queue.items[0].locked);
    assert!(!h.app.attention_queue.items[1].locked);

    // Open queue list (`Cmd+I`): `x` cannot dismiss an Ask bar, but CAN dismiss `fail-1`.
    h.app.toggle_attention_queue_list();
    assert!(h.app.attention_queue.open);
    assert!(!h.app.dismiss_queue_bar("ask-pane-b"));
    assert!(h.app.dismiss_queue_bar("fail-1"));
    assert!(!h.app.attention_queue.items.iter().any(|b| b.id == "fail-1"));
}

/// R39: Decision Overlay interaction:
/// - `Cmd+Alt+B` opens focused pane's ask (or oldest ask).
/// - Question is `"What would you do here?"`, recommended option is pre-selected.
/// - Plain `Esc` closes the overlay leaving the ask queued without sending a verdict.
/// - `Cmd+Esc` also closes the overlay leaving the ask queued.
/// - `1–9` or `Enter` picks an option, sends exactly one verdict, and removes the ask from the queue.
#[test]
fn decision_overlay_verdict_and_esc_handling_r39() {
    let mut h = HostHarness::new();
    let pane_a = h.add_test_pane();
    let ctx_id = h.app.router.active().context_id;
    h.app.set_context_columns(ctx_id, vec![vec![pane_a]]);
    h.app.focus_pane_in_strip(pane_a);

    let options = vec![
        QueueOption {
            key: "allow_once".into(),
            label: "Allow once".into(),
            recommended: false,
        },
        QueueOption {
            key: "allow_session".into(),
            label: "Allow for session".into(),
            recommended: true,
        },
        QueueOption {
            key: "deny".into(),
            label: "Deny".into(),
            recommended: false,
        },
    ];
    h.app.enqueue_ask(
        "ask-overlay-1",
        pane_a,
        "curl https://plexiapp.com",
        1_000,
        options,
    );

    // Summon via Cmd+Alt+B.
    h.app.summon_decision_overlay();
    let ov = h
        .app
        .attention_queue
        .decision_overlay
        .as_ref()
        .expect("overlay open");
    assert_eq!(ov.question, OVERLAY_QUESTION);
    assert_eq!(ov.command_text, "curl https://plexiapp.com");
    assert_eq!(
        ov.selected_idx, 1,
        "recommended option must be pre-selected"
    );

    // Plain Esc closes the overlay leaving the ask queued without sending a verdict.
    send_key(&mut h, Modifiers::NONE, Key::Escape);
    assert!(h.app.attention_queue.decision_overlay.is_none());
    assert_eq!(h.app.attention_queue.ask_count(), 1);
    assert!(h.app.attention_queue.verdicts.is_empty());

    // Re-open via Cmd+Alt+B; Cmd+Esc alias also closes the overlay leaving the ask queued.
    h.app.summon_decision_overlay();
    send_key(&mut h, Modifiers::COMMAND, Key::Escape);
    assert!(h.app.attention_queue.decision_overlay.is_none());
    assert_eq!(h.app.attention_queue.ask_count(), 1);
    assert!(h.app.attention_queue.verdicts.is_empty());

    // Re-open via Cmd+Alt+B and press `Enter` to pick the pre-selected recommended option (idx 1).
    h.app.summon_decision_overlay();
    send_key(&mut h, Modifiers::NONE, Key::Enter);
    assert!(h.app.attention_queue.decision_overlay.is_none());
    assert_eq!(h.app.attention_queue.ask_count(), 0);
    assert_eq!(h.app.attention_queue.verdicts.len(), 1);
    assert_eq!(h.app.attention_queue.verdicts[0].ask_id, "ask-overlay-1");
    assert_eq!(
        h.app.attention_queue.verdicts[0].option_key,
        "allow_session"
    );

    // Attempting to pick again on the same ask ID is a no-op (at most one verdict per ask).
    assert!(!h.app.pick_decision_overlay_option(0));
    assert_eq!(h.app.attention_queue.verdicts.len(), 1);
}

/// R40: Strict input ownership across zoom levels and overlays, and R41/R42 checks.
#[test]
fn strict_input_ownership_and_legacy_removal_r40_r41_r42() {
    let mut h = HostHarness::new();
    let p1 = h.add_test_pane();
    let p2 = h.add_test_pane();
    let p3 = h.add_test_pane();
    let ctx_id = h.app.router.active().context_id;
    h.app
        .set_context_columns(ctx_id, vec![vec![p1], vec![p2], vec![p3]]);
    h.app.focus_pane_in_strip(p1);

    // At ZoomLevel::Columns with no overlay open: focused visible pane p1 qualifies for input;
    // unfocused p2 and off-screen clipped p3 do not.
    assert!(h.app.pane_qualifies_for_input(p1));
    assert!(!h.app.pane_qualifies_for_input(p2));
    assert!(!h.app.pane_qualifies_for_input(p3));

    // When AttentionQueue list is open: no pane qualifies for input.
    h.app.toggle_attention_queue_list();
    assert!(!h.app.pane_qualifies_for_input(p1));
    h.app.toggle_attention_queue_list();
    assert!(h.app.pane_qualifies_for_input(p1));

    // At SiblingCards and TreeAtlas: no pane qualifies for input.
    h.app.zoom_out_or_close_surface();
    assert_eq!(h.app.zoom.level, ZoomLevel::SiblingCards);
    assert!(!h.app.pane_qualifies_for_input(p1));

    h.app.zoom_out_or_close_surface();
    assert_eq!(h.app.zoom.level, ZoomLevel::TreeAtlas);
    assert!(!h.app.pane_qualifies_for_input(p1));

    // R41: Cmd+Shift+A is unbound in KeyBindings::default().
    let keymap = crate::host::keys::KeyBindings::default();
    let mut probe_keys = std::collections::HashMap::new();
    probe_keys.insert("Cmd+Shift+A".to_string(), "run:test".to_string());
    assert!(
        crate::host::keys::validate_user_keys(&probe_keys, &keymap).is_empty(),
        "Cmd+Shift+A must remain unbound after notification modal removal"
    );

    // R42: Tokyo Night default palette & Motorways geometry tokens.
    let colors = crate::ui::theme::Colors::from_config(&crate::config::ThemeConfig::default());
    assert_eq!(colors.bg_darkest, Color32::from_rgb(0x16, 0x16, 0x1e));
    assert_eq!(colors.terminal_bg, Color32::from_rgb(0x1a, 0x1b, 0x26));
    assert_eq!(colors.accent, Color32::from_rgb(0x7a, 0xa2, 0xf7));
    assert_eq!(colors.zoom_needs, Color32::from_rgb(0xe0, 0xaf, 0x68));
    assert_eq!(colors.zoom_failed, Color32::from_rgb(0xf7, 0x76, 0x8e));
    assert_eq!(colors.depth_tint(4), colors.depth_tint(99));
    assert_eq!(style::RADIUS_MOTORWAYS, egui::CornerRadius::same(14));
    assert_eq!(style::RADIUS_BAR, egui::CornerRadius::same(10));
    assert_eq!(style::RADIUS_PILL, egui::CornerRadius::same(99));
}

/// Lifecycle/integrity items are one deduped notice per resource, sort below
/// every ask, are never counted as asks, and clear once seen or acknowledged.
#[test]
fn integrity_notice_sorts_below_asks_dedupes_and_acknowledges() {
    let mut h = HostHarness::new();
    let monitor = crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir());
    // Two restarts filed the same kind of integrity record.
    for run_tag in ["a", "b"] {
        monitor
            .file_needs_you(crate::broker::gate::NeedsYouFile {
                kind: crate::broker::gate::NeedsYouKind::Integrity,
                actor: "host".to_string(),
                resource: "permission-profile".to_string(),
                summary: "The host did not shut down cleanly.".to_string(),
                expires_at: None,
                run_tag: Some(run_tag.to_string()),
            })
            .expect("file integrity");
    }

    let pane = h.add_test_pane();
    h.app.enqueue_ask("ask-1", pane, "cargo publish", 1_000, Vec::new());
    h.app.sync_attention_queue();

    let items = &h.app.attention_queue.items;
    assert_eq!(items.iter().filter(|b| b.category == QueueCategory::Notice).count(), 1);
    assert_eq!(items.first().map(|b| b.category), Some(QueueCategory::Ask));
    assert_eq!(items.last().map(|b| b.category), Some(QueueCategory::Notice));
    assert_eq!(h.app.attention_queue.ask_count(), 1);

    let notice_id = items.last().unwrap().id.clone();
    assert!(h.app.open_decision_overlay_for_ask(&notice_id));
    let overlay = h.app.attention_queue.decision_overlay.clone().unwrap();
    assert_eq!(overlay.options.len(), 1);
    assert_eq!(overlay.options[0].label, "Acknowledge");
    assert!(h.app.pick_decision_overlay_option(0));

    h.app.sync_attention_queue();
    assert!(h
        .app
        .attention_queue
        .items
        .iter()
        .all(|b| b.category != QueueCategory::Notice));
    // Acknowledging cleared every duplicate record, so a restart finds none.
    assert!(monitor
        .open_needs_you()
        .iter()
        .all(|r| r.kind != crate::broker::gate::NeedsYouKind::Integrity));
}

#[test]
fn esc_closes_queue_list() {
    let mut h = HostHarness::new();
    let pane = h.add_test_pane();
    h.app
        .enqueue_ask("ask-q-1", pane, "cargo check", 1_000, Vec::new());

    // Cmd+I toggles the queue list open.
    h.app.toggle_attention_queue_list();
    assert!(h.app.attention_queue.open);

    // Plain Esc closes the queue list.
    send_key(&mut h, Modifiers::NONE, Key::Escape);
    assert!(!h.app.attention_queue.open);
    assert_eq!(h.app.attention_queue.ask_count(), 1);

    // Cmd+I still toggles it open and closed.
    h.app.toggle_attention_queue_list();
    assert!(h.app.attention_queue.open);
    send_key(&mut h, Modifiers::COMMAND, Key::I);
    assert!(!h.app.attention_queue.open);
}

#[test]
fn esc_closes_decision_overlay_without_verdict() {
    let mut h = HostHarness::new();
    let pane = h.add_test_pane();
    let ctx_id = h.app.router.active().context_id;
    h.app.set_context_columns(ctx_id, vec![vec![pane]]);
    h.app.focus_pane_in_strip(pane);

    h.app
        .enqueue_ask("ask-ov-esc", pane, "rm -rf /tmp/build", 1_000, Vec::new());
    h.app.summon_decision_overlay();
    assert!(h.app.attention_queue.decision_overlay.is_some());

    send_key(&mut h, Modifiers::NONE, Key::Escape);
    assert!(h.app.attention_queue.decision_overlay.is_none());
    assert_eq!(h.app.attention_queue.ask_count(), 1);
    assert!(h.app.attention_queue.verdicts.is_empty());
}

#[test]
fn esc_swallowed_when_overlay_open() {
    use crate::app::app_trait::KeyDisposition;

    let mut h = HostHarness::new();
    let pane = h.add_test_pane();
    let ctx_id = h.app.router.active().context_id;
    h.app.set_context_columns(ctx_id, vec![vec![pane]]);
    h.app.focus_pane_in_strip(pane);

    h.app.enqueue_ask(
        "ask-swallow-esc",
        pane,
        "git push --force",
        1_000,
        Vec::new(),
    );
    h.app.summon_decision_overlay();
    assert!(h.app.attention_queue.decision_overlay.is_some());

    let mut input = PlexiInput::synthetic(
        vec![Event::Key {
            key: Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }],
        Modifiers::NONE,
    );
    let disp = h.app.handle_decision_overlay_input(&mut input);
    assert_eq!(disp, KeyDisposition::Consumed);
    assert!(
        input.is_empty(),
        "Esc must be cleared from PlexiInput so it is never forwarded to the pane underneath"
    );
    assert!(h.app.attention_queue.decision_overlay.is_none());
    assert_eq!(h.app.attention_queue.ask_count(), 1);
    assert!(h.app.attention_queue.verdicts.is_empty());

    // Also verify via full frame (press_key) that the underlying builtin app pane is NOT closed by Esc.
    h.app.summon_decision_overlay();
    assert!(h.app.attention_queue.decision_overlay.is_some());
    let panes_before = h.state().open_panes.len();
    h.press_key(Key::Escape, Modifiers::NONE);
    assert!(h.app.attention_queue.decision_overlay.is_none());
    assert_eq!(
        h.state().open_panes.len(),
        panes_before,
        "Esc closing decision overlay must not close or reach the underlying app pane"
    );
}

#[test]
fn cmd_esc_still_zooms_out_when_nothing_open() {
    let mut h = HostHarness::new();
    let pane = h.add_test_pane();
    let ctx_id = h.app.router.active().context_id;
    h.app.set_context_columns(ctx_id, vec![vec![pane]]);
    h.app.focus_pane_in_strip(pane);

    assert!(!h.app.attention_queue.open);
    assert!(h.app.attention_queue.decision_overlay.is_none());
    assert_eq!(h.app.zoom.level, ZoomLevel::Columns);

    // Cmd+Esc zooms out from Columns -> SiblingCards -> TreeAtlas when neither queue nor overlay is open.
    h.press_key(Key::Escape, Modifiers::COMMAND);
    assert_eq!(h.app.zoom.level, ZoomLevel::SiblingCards);

    // Release Escape so the next press is not treated by egui as key-repeat.
    h.frame(egui::RawInput {
        events: vec![Event::Key {
            key: Key::Escape,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers: Modifiers::NONE,
        }],
        ..Default::default()
    });

    h.press_key(Key::Escape, Modifiers::COMMAND);
    assert_eq!(h.app.zoom.level, ZoomLevel::TreeAtlas);
}

/// Cmd+Enter in SiblingCards must dive into the highlighted card, exactly like bare Enter,
/// even after the selection moved off the context that was focused when zooming out.
#[test]
fn cmd_enter_zooms_into_selected_card_like_enter() {
    let mut h = HostHarness::new();
    let ctx0_p = h.add_test_pane();
    let ctx0_id = h.app.router.active().context_id;
    h.app.set_context_columns(ctx0_id, vec![vec![ctx0_p]]);
    let ctx1_id = h.app.create_context_without_pty("ctx-1", None);
    let ctx1_p = h.add_test_pane();
    h.app.set_context_columns(ctx1_id, vec![vec![ctx1_p]]);
    let ctx2_id = h.app.create_context_without_pty("ctx-2", None);
    let ctx2_p = h.add_test_pane();
    h.app.set_context_columns(ctx2_id, vec![vec![ctx2_p]]);

    for chord in [Modifiers::NONE, Modifiers::COMMAND] {
        h.app.switch_to_context_restoring_last_pane(ctx0_id);
        h.app.set_zoom_level_with_transition(ZoomLevel::Columns);
        h.app.zoom_out_or_close_surface();
        assert_eq!(h.app.zoom.level, ZoomLevel::SiblingCards);
        send_key(&mut h, Modifiers::NONE, Key::L);
        send_key(&mut h, Modifiers::NONE, Key::L);
        assert_eq!(h.app.zoom.cards_focused_context, Some(ctx2_id));
        if chord == Modifiers::NONE {
            send_key(&mut h, chord, Key::Enter);
        } else {
            assert!(h.app.zoom_in_level());
        }
        assert_eq!(h.app.zoom.level, ZoomLevel::Columns, "chord {chord:?}");
        assert_eq!(h.app.router.active().context_id, ctx2_id, "chord {chord:?}");
    }
}

fn press_and_release_key(h: &mut HostHarness, key: Key, modifiers: Modifiers) {
    h.press_key(key, modifiers);
    h.frame(egui::RawInput {
        events: vec![Event::Key {
            key,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers: Modifiers::NONE,
        }],
        ..Default::default()
    });
}

/// Stint 0986 Done-When:
/// Keyboard-injected Cmd+Enter toggles exactly the focused pane and a second press restores
/// its prior geometry, focus and scroll, for both one-pane and multi-pane columns.
#[test]
fn cmd_enter_toggles_focused_pane_zoom_and_restores_geometry_focus_and_scroll() {
    let mut h = HostHarness::new();
    let col0_p = h.add_test_pane();
    h.focus_pane(col0_p).run_frames(1);

    let col1_top = h
        .app
        .split_focused(true, None, false, false, None)
        .expect("split right for column 1");
    h.run_frames(2).focus_pane(col1_top);

    let col1_bot = h
        .app
        .split_focused(false, None, false, false, None)
        .expect("split down to stack inside column 1");
    h.run_frames(2);

    let ctx = h.app.router.active().context_id;
    h.app
        .set_context_columns(ctx, vec![vec![col0_p], vec![col1_top, col1_bot]]);
    h.run_frames(2);

    let active_win = h.app.active_window;
    let col0_tile = h.app.windows[active_win]
        .tree
        .tiles
        .find_pane(&col0_p)
        .expect("col0 pane tile");
    let col1_top_tile = h.app.windows[active_win]
        .tree
        .tiles
        .find_pane(&col1_top)
        .expect("col1 top pane tile");
    let col1_bot_tile = h.app.windows[active_win]
        .tree
        .tiles
        .find_pane(&col1_bot)
        .expect("col1 bot pane tile");

    let rect_col0_before = h.app.windows[active_win]
        .tree
        .tiles
        .rect(col0_tile)
        .expect("col0 rect");
    let rect_top_before = h.app.windows[active_win]
        .tree
        .tiles
        .rect(col1_top_tile)
        .expect("col1 top rect");
    let rect_bot_before = h.app.windows[active_win]
        .tree
        .tiles
        .rect(col1_bot_tile)
        .expect("col1 bot rect");

    // 1. Focus stacked pane in multi-pane column (col1_bot).
    h.focus_pane(col1_bot).run_frames(2);
    assert_eq!(h.app.active_focused_pane_id(), Some(col1_bot));
    assert!(h.app.windows[active_win].zoomed_pane.is_none());

    // Press Cmd+Enter -> zooms exactly col1_bot.
    press_and_release_key(&mut h, Key::Enter, Modifiers::COMMAND);
    h.run_frames(2);
    assert_eq!(
        h.app.windows[active_win].zoomed_pane,
        Some(col1_bot_tile),
        "Cmd+Enter must zoom the focused multi-pane column pane"
    );

    // Second press Cmd+Enter -> restores prior geometry, focus and scroll.
    press_and_release_key(&mut h, Key::Enter, Modifiers::COMMAND);
    h.run_frames(2);
    assert!(
        h.app.windows[active_win].zoomed_pane.is_none(),
        "second Cmd+Enter must unzoom"
    );
    assert_eq!(
        h.app.active_focused_pane_id(),
        Some(col1_bot),
        "focus must be preserved after unzoom"
    );
    let rect_bot_after = h.app.windows[active_win]
        .tree
        .tiles
        .rect(col1_bot_tile)
        .expect("col1 bot rect after unzoom");
    assert_eq!(
        rect_bot_after, rect_bot_before,
        "multi-pane column geometry must be preserved after unzoom"
    );

    // 2. Focus one-pane column (col0_p).
    h.focus_pane(col0_p).run_frames(2);
    assert_eq!(h.app.active_focused_pane_id(), Some(col0_p));
    assert!(h.app.windows[active_win].zoomed_pane.is_none());

    // Press Cmd+Enter -> zooms col0_p.
    press_and_release_key(&mut h, Key::Enter, Modifiers::COMMAND);
    h.run_frames(2);
    assert_eq!(
        h.app.windows[active_win].zoomed_pane,
        Some(col0_tile),
        "Cmd+Enter must zoom the focused one-pane column pane"
    );

    // Second press Cmd+Enter -> unzooms col0_p and restores geometry.
    press_and_release_key(&mut h, Key::Enter, Modifiers::COMMAND);
    h.run_frames(2);
    assert!(
        h.app.windows[active_win].zoomed_pane.is_none(),
        "second Cmd+Enter on one-pane column must unzoom"
    );
    assert_eq!(
        h.app.active_focused_pane_id(),
        Some(col0_p),
        "one-pane column focus must be preserved after unzoom"
    );
    let rect_col0_after = h.app.windows[active_win]
        .tree
        .tiles
        .rect(col0_tile)
        .expect("col0 rect after unzoom");
    assert_eq!(
        rect_col0_after, rect_col0_before,
        "one-pane column geometry must be preserved after unzoom"
    );
    let rect_top_after = h.app.windows[active_win]
        .tree
        .tiles
        .rect(col1_top_tile)
        .expect("col1 top rect after unzoom");
    assert_eq!(
        rect_top_after, rect_top_before,
        "unrelated pane geometry must remain intact"
    );

    // 3. Single-pane window records an edge bump down when Cmd+Enter is pressed.
    let mut single_h = HostHarness::new();
    let single_p = single_h.add_test_pane();
    single_h.focus_pane(single_p).run_frames(2);
    let bumps_before = single_h.app.zoom.edge_bump_count;
    press_and_release_key(&mut single_h, Key::Enter, Modifiers::COMMAND);
    single_h.run_frames(2);
    assert!(
        single_h.app.windows[0].zoomed_pane.is_none(),
        "single pane window cannot zoom"
    );
    assert_eq!(
        single_h.app.zoom.edge_bump_count,
        bumps_before + 1,
        "single pane Cmd+Enter must record edge bump"
    );
}

#[test]
fn tree_atlas_directional_navigation_uneven_multi_root_hierarchy() {
    let mut h = HostHarness::new();
    // Root 1 (active)
    let r1_pane = h.add_test_pane();
    let r1_id = h.app.router.active().context_id;
    h.app.set_context_columns(r1_id, vec![vec![r1_pane]]);

    // Root 1 -> Child 1A
    let c1a_id = h.app.create_context_without_pty("c1a", Some(r1_id));
    let c1a_pane = h.add_test_pane();
    h.app.set_context_columns(c1a_id, vec![vec![c1a_pane]]);

    // Root 1 -> Child 1A -> Grandchild 1A1
    let g1a1_id = h.app.create_context_without_pty("g1a1", Some(c1a_id));
    let g1a1_pane = h.add_test_pane();
    h.app.set_context_columns(g1a1_id, vec![vec![g1a1_pane]]);

    // Root 1 -> Child 1B (leaf)
    let c1b_id = h.app.create_context_without_pty("c1b", Some(r1_id));
    let c1b_pane = h.add_test_pane();
    h.app.set_context_columns(c1b_id, vec![vec![c1b_pane]]);

    // Root 2
    let r2_id = h.app.create_context_without_pty("r2", None);
    let r2_pane = h.add_test_pane();
    h.app.set_context_columns(r2_id, vec![vec![r2_pane]]);

    // Root 2 -> Child 2A, 2B, 2C
    let c2a_id = h.app.create_context_without_pty("c2a", Some(r2_id));
    let c2a_pane = h.add_test_pane();
    h.app.set_context_columns(c2a_id, vec![vec![c2a_pane]]);
    let c2b_id = h.app.create_context_without_pty("c2b", Some(r2_id));
    let c2b_pane = h.add_test_pane();
    h.app.set_context_columns(c2b_id, vec![vec![c2b_pane]]);
    let c2c_id = h.app.create_context_without_pty("c2c", Some(r2_id));
    let c2c_pane = h.add_test_pane();
    h.app.set_context_columns(c2c_id, vec![vec![c2c_pane]]);

    // Root 3: leaf root (no children)
    let r3_id = h.app.create_context_without_pty("r3", None);
    let r3_pane = h.add_test_pane();
    h.app.set_context_columns(r3_id, vec![vec![r3_pane]]);

    // Root 4: missing parent (parent_id points to nonexistent context)
    let r4_id = h.app.create_context_without_pty("r4-orphan", Some(999999));
    let r4_pane = h.add_test_pane();
    h.app.set_context_columns(r4_id, vec![vec![r4_pane]]);

    // Parked context: marked parked
    let parked_id = h.app.create_context_without_pty("parked-child", Some(r1_id));
    let parked_pane = h.add_test_pane();
    h.app.set_context_columns(parked_id, vec![vec![parked_pane]]);
    if let Some(idx) = h.app.router.as_slice().iter().position(|c| c.context_id == parked_id) {
        h.app.router.get_mut(idx).parked = true;
    }

    // Switch to root 1 and zoom out to TreeAtlas
    h.app.switch_to_context_restoring_last_pane(r1_id);
    h.app.zoom_out_or_close_surface();
    h.app.zoom_out_or_close_surface();
    assert_eq!(h.app.zoom.level, ZoomLevel::TreeAtlas);

    let viewport = Rect::from_min_size(Pos2::ZERO, Vec2::new(1440.0, 900.0));
    let (nodes, _) = h.app.compute_atlas_graph(viewport);

    // Verify parked context is omitted from nodes
    assert!(!nodes.iter().any(|n| n.context_id == parked_id));

    // Verify roots are placed left-to-right: r1, r2, r3, r4
    let r1_node = nodes.iter().find(|n| n.context_id == r1_id).unwrap();
    let r2_node = nodes.iter().find(|n| n.context_id == r2_id).unwrap();
    let r3_node = nodes.iter().find(|n| n.context_id == r3_id).unwrap();
    let r4_node = nodes.iter().find(|n| n.context_id == r4_id).unwrap();
    assert!(r1_node.rect.center().x < r2_node.rect.center().x);
    assert!(r2_node.rect.center().x < r3_node.rect.center().x);
    assert!(r3_node.rect.center().x < r4_node.rect.center().x);

    // Initial focus on r1
    assert_eq!(h.app.zoom.atlas_focused_context, Some(r1_id));

    // 1. Root boundary: Up from root is a no-op with edge bump
    let bumps = h.app.zoom.edge_bump_count;
    send_key(&mut h, Modifiers::NONE, Key::K);
    assert_eq!(h.app.zoom.atlas_focused_context, Some(r1_id));
    assert_eq!(h.app.zoom.edge_bump_count, bumps + 1);
    // ArrowUp parity
    send_key(&mut h, Modifiers::NONE, Key::ArrowUp);
    assert_eq!(h.app.zoom.atlas_focused_context, Some(r1_id));
    assert_eq!(h.app.zoom.edge_bump_count, bumps + 2);

    // 2. Leftmost root boundary: Left from r1 is a no-op with edge bump
    send_key(&mut h, Modifiers::NONE, Key::H);
    assert_eq!(h.app.zoom.atlas_focused_context, Some(r1_id));
    assert_eq!(h.app.zoom.edge_bump_count, bumps + 3);
    send_key(&mut h, Modifiers::NONE, Key::ArrowLeft);
    assert_eq!(h.app.zoom.atlas_focused_context, Some(r1_id));
    assert_eq!(h.app.zoom.edge_bump_count, bumps + 4);

    // 3. Right through roots: r1 -> r2 (L), r2 -> r3 (ArrowRight), r3 -> r4 (L)
    send_key(&mut h, Modifiers::NONE, Key::L);
    assert_eq!(h.app.zoom.atlas_focused_context, Some(r2_id));
    send_key(&mut h, Modifiers::NONE, Key::ArrowRight);
    assert_eq!(h.app.zoom.atlas_focused_context, Some(r3_id));
    send_key(&mut h, Modifiers::NONE, Key::L);
    assert_eq!(h.app.zoom.atlas_focused_context, Some(r4_id));

    // Rightmost root boundary: Right from r4 is a no-op with edge bump
    send_key(&mut h, Modifiers::NONE, Key::L);
    assert_eq!(h.app.zoom.atlas_focused_context, Some(r4_id));
    assert_eq!(h.app.zoom.edge_bump_count, bumps + 5);

    // r4 (missing parent) Up is root boundary:
    send_key(&mut h, Modifiers::NONE, Key::K);
    assert_eq!(h.app.zoom.atlas_focused_context, Some(r4_id));
    assert_eq!(h.app.zoom.edge_bump_count, bumps + 6);

    // Move Left back to r3 (leaf root)
    send_key(&mut h, Modifiers::NONE, Key::H);
    assert_eq!(h.app.zoom.atlas_focused_context, Some(r3_id));

    // Leaf boundary: Down from r3 (leaf root) is a no-op with edge bump
    send_key(&mut h, Modifiers::NONE, Key::J);
    assert_eq!(h.app.zoom.atlas_focused_context, Some(r3_id));
    assert_eq!(h.app.zoom.edge_bump_count, bumps + 7);
    send_key(&mut h, Modifiers::NONE, Key::ArrowDown);
    assert_eq!(h.app.zoom.atlas_focused_context, Some(r3_id));
    assert_eq!(h.app.zoom.edge_bump_count, bumps + 8);

    // Move Left back to r1: r3 -> r2 (ArrowLeft) -> r1 (H)
    send_key(&mut h, Modifiers::NONE, Key::ArrowLeft);
    assert_eq!(h.app.zoom.atlas_focused_context, Some(r2_id));
    send_key(&mut h, Modifiers::NONE, Key::H);
    assert_eq!(h.app.zoom.atlas_focused_context, Some(r1_id));

    // 4. Down from r1 moves to first child c1a
    send_key(&mut h, Modifiers::NONE, Key::J);
    assert_eq!(h.app.zoom.atlas_focused_context, Some(c1a_id));

    // Sibling boundary: Left from c1a is a no-op (leftmost child of r1)
    send_key(&mut h, Modifiers::NONE, Key::H);
    assert_eq!(h.app.zoom.atlas_focused_context, Some(c1a_id));
    assert_eq!(h.app.zoom.edge_bump_count, bumps + 9);

    // Right from c1a moves to sibling c1b (and updates r1's child memory to c1b)
    send_key(&mut h, Modifiers::NONE, Key::L);
    assert_eq!(h.app.zoom.atlas_focused_context, Some(c1b_id));

    // Sibling boundary: Right from c1b is a no-op (rightmost child of r1)
    send_key(&mut h, Modifiers::NONE, Key::L);
    assert_eq!(h.app.zoom.atlas_focused_context, Some(c1b_id));
    assert_eq!(h.app.zoom.edge_bump_count, bumps + 10);

    // Leaf boundary: Down from c1b is a no-op (leaf)
    send_key(&mut h, Modifiers::NONE, Key::J);
    assert_eq!(h.app.zoom.atlas_focused_context, Some(c1b_id));
    assert_eq!(h.app.zoom.edge_bump_count, bumps + 11);

    // Up from c1b returns to parent r1
    send_key(&mut h, Modifiers::NONE, Key::K);
    assert_eq!(h.app.zoom.atlas_focused_context, Some(r1_id));

    // Child memory test: Down from r1 now returns to remembered child c1b!
    send_key(&mut h, Modifiers::NONE, Key::J);
    assert_eq!(h.app.zoom.atlas_focused_context, Some(c1b_id));

    // Left to c1a
    send_key(&mut h, Modifiers::NONE, Key::ArrowLeft);
    assert_eq!(h.app.zoom.atlas_focused_context, Some(c1a_id));

    // Down from c1a moves to g1a1 (depth 2)
    send_key(&mut h, Modifiers::NONE, Key::ArrowDown);
    assert_eq!(h.app.zoom.atlas_focused_context, Some(g1a1_id));

    // Leaf boundary: Down from g1a1 is a no-op
    send_key(&mut h, Modifiers::NONE, Key::J);
    assert_eq!(h.app.zoom.atlas_focused_context, Some(g1a1_id));
    assert_eq!(h.app.zoom.edge_bump_count, bumps + 12);

    // Sibling boundaries: g1a1 is an only child
    send_key(&mut h, Modifiers::NONE, Key::H);
    assert_eq!(h.app.zoom.atlas_focused_context, Some(g1a1_id));
    assert_eq!(h.app.zoom.edge_bump_count, bumps + 13);
    send_key(&mut h, Modifiers::NONE, Key::L);
    assert_eq!(h.app.zoom.atlas_focused_context, Some(g1a1_id));
    assert_eq!(h.app.zoom.edge_bump_count, bumps + 14);

    // Up to c1a (ArrowUp)
    send_key(&mut h, Modifiers::NONE, Key::ArrowUp);
    assert_eq!(h.app.zoom.atlas_focused_context, Some(c1a_id));

    // Up to r1 (K)
    send_key(&mut h, Modifiers::NONE, Key::K);
    assert_eq!(h.app.zoom.atlas_focused_context, Some(r1_id));

    // Child memory: Down from r1 remembers c1a
    send_key(&mut h, Modifiers::NONE, Key::J);
    assert_eq!(h.app.zoom.atlas_focused_context, Some(c1a_id));

    // Down from c1a remembers g1a1
    send_key(&mut h, Modifiers::NONE, Key::J);
    assert_eq!(h.app.zoom.atlas_focused_context, Some(g1a1_id));
}

use super::super::*;
use crate::host::command::{HostAction, OpenPaneRequest, PaneRuntimeKind, Placement, ShareRatio};
use crate::host::effect::HostEffect;

fn make_app() -> PlexiApp {
    let ctx = egui::Context::default();
    let ft = crate::platform::logging::new_frame_tick();
    PlexiApp::new_for_test(ctx, ft).0
}

fn open_req() -> OpenPaneRequest {
    OpenPaneRequest {
        runtime: PaneRuntimeKind::Terminal,
        placement: Placement::Right,
        share: ShareRatio::new(1.0, 1.0).unwrap(),
        group: None,
        declared_capabilities: vec![],
    }
}

// ── ID allocation invariants ──────────────────────────────────────────────────

#[test]
fn alloc_pane_id_is_monotone_across_app() {
    let mut app = make_app();
    let a = app.host.alloc_pane_id();
    let b = app.host.alloc_pane_id();
    let c = app.host.alloc_pane_id();
    assert!(
        a < b && b < c,
        "alloc_pane_id must return strictly increasing IDs"
    );
    assert_eq!(app.host.test_next_pane_id(), c + 1);
}

#[test]
fn all_allocated_ids_strictly_below_next_pane_id() {
    let mut app = make_app();
    let ids: Vec<_> = (0..8).map(|_| app.host.alloc_pane_id()).collect();
    let next = app.host.test_next_pane_id();
    for id in &ids {
        assert!(
            *id < next,
            "allocated id={id} must be < next_pane_id={next}"
        );
    }
    // All IDs are distinct.
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), ids.len(), "all allocated IDs must be unique");
}

// ── handle_command path keeps HostModel in sync ───────────────────────────────

#[test]
fn open_pane_via_handle_command_registers_in_model() {
    let mut app = make_app();
    let pre_count = app.host.test_pane_ids().len();
    let pre_next = app.host.test_next_pane_id();

    let effects = app
        .host
        .handle_command(HostAction::OpenPane(open_req()), &mut app.host_services);

    let opened_id = effects
        .iter()
        .find_map(|e| match e {
            HostEffect::PaneOpened { pane_id, .. } => Some(*pane_id),
            _ => None,
        })
        .expect("OpenPane must emit PaneOpened");

    // Effect pane_id matches what the allocator would have returned.
    assert_eq!(opened_id, pre_next);
    // Model now tracks the new pane.
    assert_eq!(app.host.test_pane_ids().len(), pre_count + 1);
    assert!(app.host.test_pane_ids().contains(&opened_id));
    // Focus moved to the new pane.
    assert_eq!(app.host.test_focused_pane(), Some(opened_id));
}

#[test]
fn close_focused_via_handle_command_removes_pane_from_model() {
    let mut app = make_app();
    let effects = app
        .host
        .handle_command(HostAction::OpenPane(open_req()), &mut app.host_services);
    let opened_id = effects
        .iter()
        .find_map(|e| match e {
            HostEffect::PaneOpened { pane_id, .. } => Some(*pane_id),
            _ => None,
        })
        .unwrap();
    assert!(app.host.test_pane_ids().contains(&opened_id));

    let close_effects = app
        .host
        .handle_command(HostAction::CloseFocusedPane, &mut app.host_services);
    let closed_id = close_effects
        .iter()
        .find_map(|e| match e {
            HostEffect::PaneClosed { pane_id } => Some(*pane_id),
            _ => None,
        })
        .expect("CloseFocusedPane must emit PaneClosed");

    assert_eq!(closed_id, opened_id);
    assert!(
        !app.host.test_pane_ids().contains(&opened_id),
        "closed pane must be removed from HostModel registry"
    );
}

#[test]
fn split_via_handle_command_adds_pane_to_model_and_shifts_focus() {
    let mut app = make_app();
    let pre_next = app.host.test_next_pane_id();

    let effects = app
        .host
        .handle_command(HostAction::SplitVertical, &mut app.host_services);

    let split_id = effects
        .iter()
        .find_map(|e| match e {
            HostEffect::SplitOpened { pane_id, .. } => Some(*pane_id),
            _ => None,
        })
        .expect("SplitVertical must emit SplitOpened");

    assert_eq!(split_id, pre_next);
    assert!(app.host.test_pane_ids().contains(&split_id));
    assert_eq!(app.host.test_focused_pane(), Some(split_id));
}

// ── Workspace restore / seeding invariant ────────────────────────────────────

#[test]
fn seeded_next_pane_id_starts_allocations_above_restored_ids() {
    let mut app = make_app();
    // Simulate restored workspace: pane IDs in range [10, 49].
    let max_restored: u64 = 49;
    app.host.seed_next_pane_id(max_restored + 1);

    assert_eq!(app.host.test_next_pane_id(), max_restored + 1);

    for _ in 0..5 {
        let id = app.host.alloc_pane_id();
        assert!(
            id > max_restored,
            "post-seed allocation id={id} must exceed max restored id={max_restored}"
        );
    }
}

#[test]
fn seed_below_high_water_mark_is_ignored() {
    let mut app = make_app();
    // Advance the counter to 10.
    let before = (0..9).map(|_| app.host.alloc_pane_id()).last().unwrap();
    let high = app.host.test_next_pane_id();
    assert!(high > before);

    // Attempting to seed below the current high-water mark must be rejected.
    app.host.seed_next_pane_id(1);
    assert_eq!(
        app.host.test_next_pane_id(),
        high,
        "seed below high-water mark must not lower the counter"
    );
}

// ── Design invariant documentation ───────────────────────────────────────────

/// Documents the intentional architectural split between HostModel and
/// PlexiApp.windows.
///
/// HostModel.context().panes tracks panes opened via handle_command.
/// Panes created with alloc_pane_id() directly (e.g. create_single_pane_tree,
/// spawn_terminal_pane_at) are NOT registered in this list. The authoritative
/// pane registry for rendering is PlexiApp.windows.panes.
#[test]
fn direct_alloc_does_not_register_in_host_model_panes() {
    let mut app = make_app();
    let pre_count = app.host.test_pane_ids().len();
    let _id = app.host.alloc_pane_id();
    assert_eq!(
        app.host.test_pane_ids().len(),
        pre_count,
        "alloc_pane_id() must not register in HostModel.panes; that is the caller's responsibility"
    );
}

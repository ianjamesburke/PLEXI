use super::super::*;

fn make_app() -> PlexiApp {
    let ctx = egui::Context::default();
    let ft = crate::platform::logging::new_frame_tick();
    PlexiApp::new_for_test(ctx, ft).0
}

// ── ID allocation invariants ──────────────────────────────────────────────────

#[test]
fn alloc_pane_id_is_monotone_across_view() {
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

// ── Workspace restore / seeding invariant ────────────────────────────────────

#[test]
fn seeded_next_pane_id_starts_allocations_above_restored_ids() {
    let mut app = make_app();
    // new_for_test already reserved a private block. A restored workspace sits
    // above that floor; seeding below it is rejected.
    let floor = app.host.test_next_pane_id();
    let max_restored = floor + 39;
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

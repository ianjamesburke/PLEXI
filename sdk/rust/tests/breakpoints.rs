//! Integration tests for the public breakpoint API.
//!
//! These run against the published crate surface (no private access),
//! so they can only exercise the parts of the API that don't require a
//! live `RenderContext`. The in-crate unit tests cover the full dispatch
//! path, including `BreakpointSet::dispatch` firing the right closures.

use plexi_sdk::{pick_breakpoint, BreakpointSet};

#[test]
fn pick_breakpoint_picks_largest_match() {
    // [full, compact, fallback] by area descending.
    let bps: &[(f32, f32)] = &[(800.0, 500.0), (400.0, 0.0), (0.0, 0.0)];

    // Large pane: full wins.
    assert_eq!(pick_breakpoint(1200.0, 800.0, bps), Some(0));

    // Medium pane: full is too big (needs 800x500), compact fits.
    assert_eq!(pick_breakpoint(600.0, 600.0, bps), Some(1));

    // Small pane: only the (0, 0) fallback fits.
    assert_eq!(pick_breakpoint(200.0, 200.0, bps), Some(2));
}

#[test]
fn pick_breakpoint_respects_per_axis_constraints() {
    // Compact requires 400 wide but no height constraint; full requires
    // both. A wide-but-short pane should land on compact, not full.
    let bps: &[(f32, f32)] = &[(800.0, 500.0), (400.0, 0.0), (0.0, 0.0)];
    assert_eq!(pick_breakpoint(900.0, 300.0, bps), Some(1));
}

#[test]
fn pick_breakpoint_returns_none_when_nothing_fits() {
    let bps: &[(f32, f32)] = &[(1000.0, 1000.0), (800.0, 600.0)];
    assert_eq!(pick_breakpoint(400.0, 300.0, bps), None);
}

#[test]
fn breakpoint_set_builder_collects_entries() {
    let set = BreakpointSet::new()
        .breakpoint(800.0, 500.0, |_| {})
        .breakpoint(400.0, 0.0, |_| {})
        .fallback(|_| {});
    assert_eq!(set.len(), 3);
    assert!(!set.is_empty());
}

#[test]
fn breakpoint_set_starts_empty() {
    let set = BreakpointSet::new();
    assert!(set.is_empty());
    assert_eq!(set.len(), 0);
}

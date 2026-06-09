use super::*;

#[test]
fn split_vertical_adds_pane() {
    let mut h = HostHarness::new();
    let pane_id = h.add_test_pane();
    h.focus_pane(pane_id);
    h.run_frames(1);

    assert_eq!(h.pane_count(), 1);
    h.app.split_focused(true, None, false, false, None);
    assert_eq!(
        h.pane_count(),
        2,
        "SplitVertical should add a terminal pane"
    );
}

#[test]
fn split_horizontal_adds_pane() {
    let mut h = HostHarness::new();
    let pane_id = h.add_test_pane();
    h.focus_pane(pane_id);
    h.run_frames(1);

    assert_eq!(h.pane_count(), 1);
    h.app.split_focused(false, None, false, false, None);
    assert_eq!(
        h.pane_count(),
        2,
        "SplitHorizontal should add a terminal pane"
    );
}

#[test]
fn new_context_adds_window() {
    let mut h = HostHarness::new();
    assert_eq!(h.window_count(), 1);
    h.app.new_context();
    assert_eq!(
        h.window_count(),
        2,
        "new_context should add a second window"
    );
}

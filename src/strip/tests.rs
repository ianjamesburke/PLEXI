//! Compatibility checks for persisted strip-era workspace metadata.
use super::model::{Column, ColumnId, ColumnWidth, Strip};
use crate::app::zoom::ColumnWidthPreset;

#[test]
fn pure_invariant_stable_column_order() {
    let mut strip = Strip::new(1);
    strip.push_column(Column::new(
        ColumnId(0),
        10,
        ColumnWidth::Preset(ColumnWidthPreset::Parent35),
    ));
    strip.push_column(Column::new(
        ColumnId(1),
        20,
        ColumnWidth::Preset(ColumnWidthPreset::Child65),
    ));
    strip.push_column(Column::new(
        ColumnId(2),
        30,
        ColumnWidth::Preset(ColumnWidthPreset::Child65),
    ));

    let col_ids_before: Vec<_> = strip.columns.iter().map(|c| c.panes.clone()).collect();
    assert_eq!(col_ids_before, vec![vec![10], vec![20], vec![30]]);

    // Focus pane in column 2, then column 0
    strip.focus_pane(30);
    assert_eq!(strip.scroll_offset, 1);
    strip.focus_pane(10);
    assert_eq!(strip.scroll_offset, 0);

    let col_ids_after: Vec<_> = strip.columns.iter().map(|c| c.panes.clone()).collect();
    assert_eq!(
        col_ids_after, col_ids_before,
        "column order must remain strictly stable"
    );
}

#[test]
fn legacy_metadata_roundtrip_keeps_widths_focus_and_scroll() {
    let mut strip = Strip::new(7);
    strip.push_column(Column::new_stacked(
        ColumnId(4),
        vec![10, 11],
        ColumnWidth::CustomPixels(640.0),
    ));
    strip.columns[0].heights = vec![2.0, 3.0];
    assert!(strip.focus_pane(11));
    strip.scroll_offset_px = 140.0;
    strip.reference_width_px = Some(480.0);
    let json = serde_json::to_string(&strip).unwrap();
    let restored: Strip = serde_json::from_str(&json).unwrap();
    assert_eq!(restored, strip);
    assert!(strip.remove_pane(10));
    assert_eq!(strip.columns[0].panes, vec![11]);
    assert_eq!(strip.columns[0].heights, vec![3.0]);
    assert_eq!(strip.focused_pane, Some(11));
}

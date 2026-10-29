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

#[test]
fn extract_panes_prunes_columns_and_preserves_order_shares_and_focus() {
    let mut strip = Strip::new(1);
    for (id, panes) in [(1, vec![10, 11]), (2, vec![20, 21]), (3, vec![30])] {
        strip.push_column(Column::new_stacked(
            ColumnId(id),
            panes,
            ColumnWidth::CustomPixels(400.0),
        ));
    }
    strip.columns[0].heights = vec![2.0, 3.0];
    strip.columns[1].heights = vec![4.0, 5.0];
    strip.focus_pane(11);
    let child = strip.extract_panes(&[10, 11, 20].into_iter().collect(), 2);
    assert_eq!(child.context_id, 2);
    assert_eq!(
        child
            .columns
            .iter()
            .map(|c| c.panes.clone())
            .collect::<Vec<_>>(),
        vec![vec![10, 11], vec![20]]
    );
    assert_eq!(child.columns[0].heights, vec![2.0, 3.0]);
    assert_eq!(child.columns[1].heights, vec![4.0]);
    assert_eq!(child.columns[1].width, ColumnWidth::CustomPixels(400.0));
    assert_eq!(child.focused_pane, Some(11));
    assert_eq!(
        strip
            .columns
            .iter()
            .map(|c| c.panes.clone())
            .collect::<Vec<_>>(),
        vec![vec![21], vec![30]]
    );
    assert_eq!(strip.columns[0].heights, vec![5.0]);
    assert!(strip.focused_pane.is_some_and(|id| strip.contains_pane(id)));
    assert_eq!(child.scroll_offset_px, 0.0);
}

#[test]
fn extract_empty_or_unknown_set_leaves_source_unchanged() {
    let mut strip = Strip::new(1);
    strip.push_column(Column::new(ColumnId(0), 10, ColumnWidth::default()));
    let original = strip.clone();
    assert_eq!(
        strip
            .extract_panes(&[99].into_iter().collect(), 2)
            .pane_count(),
        0
    );
    assert_eq!(strip, original);
    assert_eq!(strip.extract_panes(&Default::default(), 2).pane_count(), 0);
    assert_eq!(strip, original);
}

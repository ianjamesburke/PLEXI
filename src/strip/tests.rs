//! Pure tests for strip model invariants and layout calculations.

use super::layout::{layout_strip, StripViewport};
use super::model::{Column, ColumnId, ColumnWidth, Strip};
use crate::app::zoom::ColumnWidthPreset;
use egui::{Pos2, Rect, Vec2};

#[test]
fn pure_invariant_one_focused_pane() {
    let mut strip = Strip::new(1);
    let mut c0 = Column::new(ColumnId(0), 101, ColumnWidth::Preset(ColumnWidthPreset::Parent35));
    let c1 = Column::new(ColumnId(1), 102, ColumnWidth::Preset(ColumnWidthPreset::Child65));
    assert!(!c0.width.is_custom());
    c0.push_pane(103);
    assert_eq!(c0.panes, vec![101, 103]);
    assert!(c0.contains_pane(103));
    assert!(c0.remove_pane(103));
    assert!(!c0.contains_pane(103));

    strip.push_column(c0);
    strip.push_column(c1);

    assert_eq!(strip.column_count(), 2);
    assert_eq!(strip.pane_count(), 2);
    assert!(strip.contains_pane(101));
    assert!(strip.contains_pane(102));
    assert_eq!(strip.focused_pane, Some(101));

    // Focus pane 102
    assert!(strip.focus_pane(102));
    assert_eq!(strip.focused_pane, Some(102));

    let vp = StripViewport::new(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 800.0)), 4.0);
    let layout = layout_strip(&strip, &vp);

    let focused_panes: Vec<_> = layout.panes.iter().filter(|p| p.is_focused).collect();
    assert_eq!(focused_panes.len(), 1, "must have exactly one focused pane in layout");
    assert_eq!(focused_panes[0].pane_id, 102);
    assert!(layout.focus_frame.is_some(), "focus outline frame must exist");
}

#[test]
fn pure_invariant_stable_column_order() {
    let mut strip = Strip::new(1);
    strip.push_column(Column::new(ColumnId(0), 10, ColumnWidth::Preset(ColumnWidthPreset::Parent35)));
    strip.push_column(Column::new(ColumnId(1), 20, ColumnWidth::Preset(ColumnWidthPreset::Child65)));
    strip.push_column(Column::new(ColumnId(2), 30, ColumnWidth::Preset(ColumnWidthPreset::Child65)));

    let col_ids_before: Vec<_> = strip.columns.iter().map(|c| c.panes.clone()).collect();
    assert_eq!(col_ids_before, vec![vec![10], vec![20], vec![30]]);

    // Focus pane in column 2, then column 0
    strip.focus_pane(30);
    assert_eq!(strip.scroll_offset, 1);
    strip.focus_pane(10);
    assert_eq!(strip.scroll_offset, 0);

    let col_ids_after: Vec<_> = strip.columns.iter().map(|c| c.panes.clone()).collect();
    assert_eq!(col_ids_after, col_ids_before, "column order must remain strictly stable");
}

#[test]
fn pure_invariant_on_screen_pane_rects() {
    let mut strip = Strip::new(1);
    // Stacked column + two plain columns
    strip.push_column(Column::new_stacked(
        ColumnId(0),
        vec![10, 11],
        ColumnWidth::Preset(ColumnWidthPreset::Parent35),
    ));
    strip.push_column(Column::new(ColumnId(1), 20, ColumnWidth::Preset(ColumnWidthPreset::Child65)));
    strip.push_column(Column::new(ColumnId(2), 30, ColumnWidth::Preset(ColumnWidthPreset::Child65)));
    strip.push_column(Column::new(ColumnId(3), 40, ColumnWidth::Preset(ColumnWidthPreset::Child65)));

    let vp = StripViewport {
        canvas_rect: Rect::from_min_size(Pos2::new(50.0, 100.0), Vec2::new(1000.0, 600.0)),
        gap: 6.0,
        peek_width: 28.0,
    };

    // At scroll_offset 0: columns 0 and 1 are visible on-screen
    let layout0 = layout_strip(&strip, &vp);
    assert!(layout0.peek_left.is_none(), "offset 0 has no left peek");
    assert!(layout0.peek_right.is_some(), "offset 0 with 4 columns has right peek");

    for p in &layout0.panes {
        if p.is_on_screen {
            assert!(
                vp.canvas_rect.contains_rect(p.rect),
                "visible pane {:?} rect {:?} must be within viewport {:?}",
                p.pane_id,
                p.rect,
                vp.canvas_rect
            );
        }
    }

    // Scroll to offset 2: columns 2 and 3 visible, left peek present, no right peek
    strip.scroll_offset = 2;
    let layout2 = layout_strip(&strip, &vp);
    assert!(layout2.peek_left.is_some(), "offset 2 has left peek");
    assert!(layout2.peek_right.is_none(), "last columns have no right peek");

    for p in &layout2.panes {
        if p.is_on_screen {
            assert!(
                vp.canvas_rect.contains_rect(p.rect),
                "visible pane {:?} rect {:?} must be within viewport {:?}",
                p.pane_id,
                p.rect,
                vp.canvas_rect
            );
        }
    }
}

#[test]
fn pure_resize_persistence_and_divider_reset() {
    let mut strip = Strip::new(1);
    strip.push_column(Column::new(ColumnId(0), 10, ColumnWidth::Preset(ColumnWidthPreset::Parent35)));
    strip.push_column(Column::new(ColumnId(1), 20, ColumnWidth::Preset(ColumnWidthPreset::Child65)));

    let vp = StripViewport::default();
    let l_preset = layout_strip(&strip, &vp);
    let w0_preset = l_preset.panes[0].rect.width();
    let w1_preset = l_preset.panes[1].rect.width();
    let ratio_preset = w0_preset / (w0_preset + w1_preset);
    assert!((ratio_preset - 0.35).abs() < 0.05);

    // Apply custom 50/50 resize
    strip.set_column_width(0, ColumnWidth::CustomRatio(0.50));
    strip.set_column_width(1, ColumnWidth::CustomRatio(0.50));
    assert!(strip.columns[0].width.is_custom());

    let l_custom = layout_strip(&strip, &vp);
    let w0_custom = l_custom.panes[0].rect.width();
    let w1_custom = l_custom.panes[1].rect.width();
    let ratio_custom = w0_custom / (w0_custom + w1_custom);
    assert!((ratio_custom - 0.50).abs() < 0.05, "custom ratio must be ~0.50, got {ratio_custom}");

    // Reset column widths
    strip.reset_column_widths();
    let l_reset = layout_strip(&strip, &vp);
    let w0_reset = l_reset.panes[0].rect.width();
    let w1_reset = l_reset.panes[1].rect.width();
    let ratio_reset = w0_reset / (w0_reset + w1_reset);
    assert!((ratio_reset - 0.35).abs() < 0.05, "reset must restore ~0.35 ratio, got {ratio_reset}");
}

#[test]
fn pure_insert_column_right_maintains_invariants() {
    let mut strip = Strip::new(1);
    strip.push_column(Column::new_stacked(
        ColumnId(0),
        vec![1, 2],
        ColumnWidth::Preset(ColumnWidthPreset::Parent35),
    ));

    assert_eq!(strip.columns.len(), 1);
    assert_eq!(strip.columns[0].panes, vec![1, 2]);

    // Insert new column to the right of column 0
    let new_idx = strip.insert_column_right_of(0, 3, ColumnWidth::Preset(ColumnWidthPreset::Child65));
    assert_eq!(new_idx, 1);
    assert_eq!(strip.columns.len(), 2);
    assert_eq!(strip.columns[0].panes, vec![1, 2]);
    assert_eq!(strip.columns[1].panes, vec![3]);
    assert_eq!(strip.focused_pane, Some(3));
}

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

#[test]
fn pure_representative_focus_sequence_keeps_order_and_visible_rects_sound() {
    let mut strip = Strip::new(1);
    strip.push_column(Column::new_stacked(
        ColumnId(0),
        vec![10, 11],
        ColumnWidth::Preset(ColumnWidthPreset::Parent35),
    ));
    strip.push_column(Column::new(ColumnId(1), 20, ColumnWidth::Preset(ColumnWidthPreset::Child65)));
    strip.push_column(Column::new(ColumnId(2), 30, ColumnWidth::Preset(ColumnWidthPreset::Child65)));
    strip.push_column(Column::new(ColumnId(3), 40, ColumnWidth::Preset(ColumnWidthPreset::Child65)));
    let before = strip.columns.iter().map(|column| column.panes.clone()).collect::<Vec<_>>();
    let viewport = StripViewport::new(Rect::from_min_size(Pos2::ZERO, Vec2::new(960.0, 640.0)), 6.0);

    for focused in [11, 30, 40, 20, 10] {
        assert!(strip.focus_pane(focused));
        let layout = layout_strip(&strip, &viewport);
        let focused_panes = layout.panes.iter().filter(|pane| pane.is_focused).collect::<Vec<_>>();
        assert_eq!(focused_panes.len(), 1, "exactly one pane stays focused after {focused}");
        assert_eq!(focused_panes[0].pane_id, focused);
        for pane in layout.panes.iter().filter(|pane| pane.is_on_screen) {
            assert!(
                viewport.canvas_rect.contains_rect(pane.rect),
                "visible pane {} must remain on-screen after focus sequence",
                pane.pane_id
            );
        }
    }

    assert_eq!(
        strip.columns.iter().map(|column| column.panes.clone()).collect::<Vec<_>>(),
        before,
        "focus-only commands must not reorder columns or stack members"
    );
}

#[test]
fn pure_stable_column_ids_and_swap_panes_preserve_focus_and_structure() {
    let mut strip = Strip::new(1);
    strip.push_column(Column::new(
        ColumnId(0),
        10,
        ColumnWidth::Preset(ColumnWidthPreset::Parent35),
    ));
    strip.push_column(Column::new_stacked(
        ColumnId(1),
        vec![20, 21],
        ColumnWidth::Preset(ColumnWidthPreset::Child65),
    ));
    strip.push_column(Column::new(
        ColumnId(2),
        30,
        ColumnWidth::Preset(ColumnWidthPreset::Child65),
    ));

    let id_col1_before = strip.columns[1].id;
    let id_col2_before = strip.columns[2].id;

    // Remember bottom pane 21 in column 1, then focus column 0 and insert a new column right of 0.
    assert!(strip.focus_pane(21));
    assert!(strip.focus_pane(10));
    let inserted_idx =
        strip.insert_column_right_of(0, 15, ColumnWidth::Preset(ColumnWidthPreset::Child65));
    assert_eq!(inserted_idx, 1);
    assert_eq!(strip.columns[2].id, id_col1_before);
    assert_eq!(strip.columns[3].id, id_col2_before);
    assert_eq!(strip.columns[2].remembered_or_first_pane(), Some(21));

    // Insert a pane below 20 in shifted column 2.
    assert!(strip.insert_pane_below_in_column(20, 22));
    assert_eq!(strip.columns[2].panes, vec![20, 22, 21]);
    assert_eq!(strip.focused_pane, Some(22));

    // Swap bottom pane 21 horizontally with column 3's pane 30.
    assert!(strip.focus_pane(21));
    assert!(strip.swap_panes(21, 30));
    assert_eq!(strip.columns[2].panes, vec![20, 22, 30]);
    assert_eq!(strip.columns[3].panes, vec![21]);
    assert_eq!(strip.focused_pane, Some(21));
    assert_eq!(strip.find_column_by_id(id_col2_before), Some(3));
}


#[test]
fn oversized_custom_split_fits_pair_without_resizing_other_columns() {
    let mut strip = Strip::new(1);
    strip.reference_width_px = Some(800.0);
    strip.push_column(Column::new(ColumnId(0), 1, ColumnWidth::CustomPixels(333.0)));
    strip.push_column(Column::new(ColumnId(1), 2, ColumnWidth::CustomPixels(900.0)));
    strip.focus_pane(2);
    assert_eq!(strip.columns[1].width, ColumnWidth::CustomPixels(900.0));
    strip.insert_column_right_of(1, 3, ColumnWidth::Preset(ColumnWidthPreset::Child65));
    let widths = strip.intrinsic_column_widths(800.0, 4.0);
    assert_eq!(widths[0], 333.0);
    assert!(widths[1] + 4.0 + widths[2] <= 800.0);
    let intervals = strip.column_intervals(800.0, 4.0);
    assert!(intervals[1].0 >= strip.scroll_offset_px - 0.5);
    assert!(intervals[2].0 + intervals[2].1 <= strip.scroll_offset_px + 800.5);
    let before = strip.columns.iter().map(|c| c.width).collect::<Vec<_>>();
    strip.focus_pane(1);
    strip.focus_pane(2);
    assert_eq!(before, strip.columns.iter().map(|c| c.width).collect::<Vec<_>>());
}

#[test]
fn fitting_custom_split_and_navigation_preserve_widths() {
    let mut strip = Strip::new(1);
    strip.reference_width_px = Some(800.0);
    strip.push_column(Column::new(ColumnId(0), 1, ColumnWidth::CustomPixels(200.0)));
    strip.push_column(Column::new(ColumnId(1), 2, ColumnWidth::CustomPixels(1000.0)));
    strip.focus_pane(2);
    strip.focus_pane(1);
    strip.focus_pane(2);
    assert_eq!(strip.columns[1].width, ColumnWidth::CustomPixels(1000.0));
    strip.insert_column_right_of(0, 3, ColumnWidth::Preset(ColumnWidthPreset::Child65));
    assert_eq!(strip.columns[0].width, ColumnWidth::CustomPixels(200.0));
    assert_eq!(strip.columns[2].width, ColumnWidth::CustomPixels(1000.0));
    let layout = layout_strip(&strip, &StripViewport::new(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0)), 4.0));
    assert!(layout.pane(1).unwrap().is_on_screen);
    assert!(layout.pane(3).unwrap().is_on_screen);
}

#[test]
fn restored_scroll_does_not_jump_when_revealing_visible_column() {
    let mut original = Strip::new(1);
    for (index, pane) in [10, 20, 30].into_iter().enumerate() {
        original.push_column(Column::new(ColumnId(index), pane, ColumnWidth::CustomPixels(400.0)));
    }
    original.set_scroll_offset_px(300.0, Some(600.0));
    assert_eq!(original.scroll_offset, 1);
    let mut restored: Strip = serde_json::from_str(&serde_json::to_string(&original).unwrap()).unwrap();
    original.reveal_column_with_viewport(1, 600.0, 4.0);
    restored.reveal_column_with_viewport(1, 600.0, 4.0);
    assert_eq!(restored.scroll_offset_px, original.scroll_offset_px);
}

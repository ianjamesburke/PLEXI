//! Finite row composition. Measure with the painting font before allocating.
//! Required slots never shrink. Optional slots degrade in declaration order;
//! the text lane yields only after optional full/compact variants are tried.

use egui::{Rect, pos2};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Presentation {
    Full,
    Compact,
    Hidden,
}

pub(crate) struct Slot<'a> {
    pub id: &'a str,
    pub width: f32,
    pub compact: Option<f32>,
    pub required: bool,
}

#[derive(Debug)]
pub(crate) struct AllocatedSlot<'a> {
    pub id: &'a str,
    pub rect: Rect,
    pub presentation: Presentation,
}

#[derive(Debug)]
pub(crate) struct RowLayout<'a> {
    pub text: Rect,
    pub slots: Vec<AllocatedSlot<'a>>,
}

#[derive(Debug, PartialEq)]
pub(crate) struct InsufficientSpace {
    pub minimum_width: f32,
}

/// `bounds` already excludes padding and platform-owned areas. Slots are
/// placed from right to left in priority order. A zero-width text lane means
/// deliberate hiding; it is never evidence that a label fitted.
pub(crate) fn allocate<'a>(
    bounds: Rect,
    text_minimum: f32,
    gap: f32,
    slots: &[Slot<'a>],
) -> Result<RowLayout<'a>, InsufficientSpace> {
    let required: f32 = slots
        .iter()
        .filter(|s| s.required)
        .map(|s| s.width + gap)
        .sum();
    if !bounds.is_finite()
        || bounds.is_negative()
        || !gap.is_finite()
        || gap < 0.0
        || !text_minimum.is_finite()
        || text_minimum < 0.0
        || slots.iter().any(|s| {
            !s.width.is_finite()
                || s.width < 0.0
                || s.compact
                    .is_some_and(|w| !w.is_finite() || w < 0.0 || w > s.width)
        })
        || required > bounds.width()
    {
        return Err(InsufficientSpace {
            minimum_width: required,
        });
    }
    let mut optional_budget = (bounds.width() - required - text_minimum).max(0.0);
    let mut right = bounds.right();
    let mut allocated = Vec::with_capacity(slots.len());
    for slot in slots {
        let (width, presentation) = if slot.required || slot.width + gap <= optional_budget {
            (slot.width, Presentation::Full)
        } else if let Some(width) = slot.compact.filter(|w| *w + gap <= optional_budget) {
            (width, Presentation::Compact)
        } else {
            (0.0, Presentation::Hidden)
        };
        let rect = Rect::from_min_max(
            pos2(right - width, bounds.top()),
            pos2(right, bounds.bottom()),
        );
        if presentation != Presentation::Hidden {
            right -= width + gap;
            if !slot.required {
                optional_budget -= width + gap;
            }
        }
        allocated.push(AllocatedSlot {
            id: slot.id,
            rect,
            presentation,
        });
    }
    Ok(RowLayout {
        text: Rect::from_min_max(bounds.min, pos2(right.max(bounds.left()), bounds.bottom())),
        slots: allocated,
    })
}

/// Center within a free text lane, shifting towards its nearest safe edge.
/// The same rectangle is suitable for painting and semantic diagnostics.
pub(crate) fn centered_text(bounds: Rect, center_x: f32, size: egui::Vec2) -> Option<Rect> {
    if !bounds.is_finite()
        || !size.is_finite()
        || size.x <= 0.0
        || size.y <= 0.0
        || bounds.width() < size.x
        || bounds.height() < size.y
    {
        return None;
    }
    let left = (center_x - size.x / 2.0).clamp(bounds.left(), bounds.right() - size.x);
    Some(Rect::from_min_size(
        pos2(left, bounds.center().y - size.y / 2.0),
        size,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_rows_preserve_required_actions_at_every_boundary() {
        let slots = [
            Slot {
                id: "undo",
                width: 56.0,
                compact: None,
                required: true,
            },
            Slot {
                id: "metadata",
                width: 80.0,
                compact: Some(20.0),
                required: false,
            },
        ];
        for quarter in 0..2400 {
            let width = quarter as f32 / 4.0;
            let bounds = Rect::from_min_size(pos2(10.0, 20.0), egui::vec2(width, 32.0));
            match allocate(bounds, 64.0, 8.0, &slots) {
                Err(error) => assert!(width < error.minimum_width),
                Ok(row) => {
                    assert!(bounds.contains_rect(row.text));
                    assert_eq!(row.slots[0].rect.width(), 56.0);
                    for slot in &row.slots {
                        assert!(bounds.contains_rect(slot.rect), "{width}: {slot:?}");
                        if slot.presentation != Presentation::Hidden {
                            assert!(!row.text.intersects(slot.rect));
                        }
                    }
                    assert_eq!(row.slots[0].id, "undo");
                    assert_eq!(row.slots[0].presentation, Presentation::Full);
                }
            }
        }
    }

    #[test]
    fn bounded_rows_reject_nonfinite_input() {
        for width in [f32::NAN, f32::INFINITY, -1.0] {
            assert!(
                allocate(
                    Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 32.0)),
                    0.0,
                    4.0,
                    &[]
                )
                .is_err()
            );
        }
    }
}

//! Plexi design tokens — the single source of truth for spacing, typography,
//! corner radii, modal widths, and overlay styling. All overlays, modals, and
//! chrome should reference these constants rather than hard-coding magic
//! numbers.
//!
//! ## Why this exists
//! Before this module, every overlay picked its own sizes (sometimes 14.0 body,
//! sometimes 13.0, buttons 32px tall in one place and 40 in another). Cohesion
//! broke down as the UI grew. Every new overlay introduced another "how big
//! should this text be?" decision, and re-flowing layouts for readability
//! meant grepping for literals.
//!
//! ## How to use
//! ```ignore
//! use crate::ui::style;
//! ui.set_width(style::MODAL_WIDTH_MD);
//! ui.label(egui::RichText::new("Title").size(style::TEXT_TITLE_XL));
//! ```
//!
//! ## When to add a token
//! When two or more places need the same value. The token set here is
//! intentionally minimal — only the constants actually referenced in the
//! codebase. Add scale holes (e.g. `MODAL_WIDTH_SM`) as soon as a
//! migration needs them, not speculatively.

use egui::CornerRadius;

// ── Spacing scale ──────────────────────────────────────────────────────────
// 4-based scale. Use these for padding, margins, gaps between siblings.
pub const SPACE_XS: f32 = 4.0;
pub const SPACE_SM: f32 = 8.0;
pub const SPACE_MD: f32 = 12.0;
pub const SPACE_XL: f32 = 24.0;

// ── Typography scale ───────────────────────────────────────────────────────
// Point sizes for `RichText::new(...).size(...)`.
pub const TEXT_HINT: f32 = 11.0; // Keyboard hints, meta labels.
pub const TEXT_CAPTION: f32 = 12.0; // Secondary info, small labels.
pub const TEXT_META: f32 = 10.0; // Compact badges and sidebar metadata.
pub const TEXT_SIDEBAR_TITLE: f32 = 14.0; // Sidebar context row titles.
pub const TEXT_BODY: f32 = 14.0; // Default body text, button labels.
pub const TEXT_TITLE: f32 = 16.0; // Modal/section titles — pair with
                                  // `theme::font_medium` for weight contrast.
pub const TEXT_TITLE_XL: f32 = 28.0; // Primary modal title — the thing the
                                     // user reads first.

// ── Corner radii ───────────────────────────────────────────────────────────
// ONE scale: small / medium / large / pill. No literal corner radii outside
// this file (guarded by `radius_guard_tests`). Pick the step by element size.
/// Chips, badges, key chips, inline widgets, inputs, small buttons.
pub const RADIUS_SMALL: CornerRadius = CornerRadius::same(6);
/// Option rows, bars, code blocks, cards inside panels.
pub const RADIUS_MEDIUM: CornerRadius = CornerRadius::same(10);
/// Pane frames, cards, atlas nodes, modals, overlays.
pub const RADIUS_LARGE: CornerRadius = CornerRadius::same(14);
/// Capsules and pills.
pub const RADIUS_PILL: CornerRadius = CornerRadius::same(99);
// Aliases kept for existing call sites; each maps onto one of the four steps.
pub const RADIUS_SM: CornerRadius = RADIUS_SMALL;
pub const RADIUS_MD: CornerRadius = RADIUS_MEDIUM;
pub const RADIUS_LG: CornerRadius = RADIUS_LARGE;
pub const RADIUS_MOTORWAYS: CornerRadius = RADIUS_LARGE;
pub const RADIUS_BAR: CornerRadius = RADIUS_MEDIUM;
/// Badge radius as f32 (`Frame::corner_radius` / `rect_filled` accept it).
/// Keep in sync with RADIUS_BADGE in plexi_sdk/ui.py.
pub const RADIUS_BADGE: f32 = 6.0;

/// Radius that is a fraction of a shape's own size (glyph-like geometry such
/// as icons and logos, where a fixed step would overwhelm the shape).
pub fn radius_frac(size: f32, frac: f32) -> CornerRadius {
    CornerRadius::same((size * frac).round().clamp(0.0, 255.0) as u8)
}

/// The small step, capped to a quarter of the rect's shorter side so tiny
/// shapes (icon bands, minimap items) stay proportionate.
pub fn radius_fit(rect: egui::Rect) -> CornerRadius {
    let cap = (rect.width().min(rect.height()) * 0.25).round().max(0.0);
    CornerRadius::same(cap.min(RADIUS_SMALL.nw as f32) as u8)
}

/// Edge of the square pane-header provenance icon (who spawned the pane).
pub const PROVENANCE_ICON_SIZE: f32 = 14.0;

// ── Zoom / Niri Column / Atlas geometry tokens ─────────────────────────────
pub const TOOLBAR_H: f32 = 28.0;
pub const PATH_BAR_H: f32 = 34.0;
#[allow(dead_code)]
pub const STRIP_GAP: f32 = 10.0;
pub const STRIP_PEEK_W: f32 = 28.0;
/// Narrowest a clipped strip column is laid out, so `egui_tiles` never sees a zero share.
pub const STRIP_MIN_SLIVER_W: f32 = 2.0;
/// Canvas width assumed for strip shares when the real width is not known yet.
pub const STRIP_FALLBACK_CANVAS_W: f32 = 1200.0;
pub const COLUMN_WIDTH_PARENT: f32 = 0.35;
pub const COLUMN_WIDTH_CHILD: f32 = 0.65;
pub const CARD_W: f32 = 340.0;
pub const CARD_H: f32 = 240.0;
pub const CARD_GAP: f32 = 16.0;
pub const ATLAS_NODE_W: f32 = 216.0;
pub const ATLAS_NODE_H: f32 = 84.0;
pub const EDGE_ROAD_W: f32 = 2.5;
pub const EDGE_NEEDS_W: f32 = 3.0;
pub const DECISION_MODAL_W: f32 = 580.0;
pub const QUEUE_PANEL_W: f32 = 640.0;
pub const PEEK_CARD_W: f32 = 380.0;
pub const ZOOM_DURATION_MS: u64 = 160;
#[allow(dead_code)]
pub const CAMERA_EASE_MS: u64 = 120;

// ── Modal widths ───────────────────────────────────────────────────────────
// Pick the smallest width that fits the content without crowding. Bigger is
// NOT better — oversized modals feel empty and force eye travel.
pub const GALLERY_NARROW_W: f32 = 320.0;

pub const MODAL_WIDTH_MD: f32 = 640.0; // Compact modals.
pub const MODAL_WIDTH_PALETTE: f32 = 576.0; // Command palette — a launcher, not a document.
pub const MODAL_WIDTH_NOTES_PICKER: f32 = 1040.0; // Notes picker — list left, wrapped preview right.
pub const NOTES_PICKER_LIST_FRACTION: f32 = 0.5; // Share of the notes picker given to the list half.
/// Fixed two-line caption slot between the notes picker's body and its hint bar
/// so confirm prompts and errors never resize or shift the modal.
pub const NOTES_PICKER_STATUS_HEIGHT: f32 = (TEXT_CAPTION + SPACE_XS) * 2.0;
pub const MODAL_WIDTH_NOTIFY: f32 = 760.0; // Notification modal — wider for breathing room.

// ── Button heights ─────────────────────────────────────────────────────────
pub const BUTTON_H_MD: f32 = 32.0; // Standard form buttons.
/// Floor width for a modal's confirm/cancel pair. Wide enough that a short
/// label ("OK") still reads as a button rather than as a chip, and that the
/// two sit as a matched pair regardless of label length.
pub const BUTTON_MIN_W_MODAL: f32 = 88.0;
pub const BUTTON_H_LG: f32 = 52.0; // Primary action buttons in modals.

// ── List rows ──────────────────────────────────────────────────────────────
// 48 gives a two-line row (12pt medium + 11pt secondary) ~8px of air inside
// the selection highlight; 14 horizontal padding keeps the title clear of
// the highlight's left edge — 10 read as cramped against the outline.
pub const LIST_ROW_H: f32 = 48.0;
// Dense single-line variant for data-dense listings (file browser rows).
// 30 fits one 12pt medium line with ~7px of air — the file-explorer PRD's
// 28-32px density target. The 48px two-line row above is for pickers.
pub const LIST_ROW_DENSE_H: f32 = 30.0;
pub const LIST_ROW_PAD_H: f32 = 14.0;
pub const LIST_ROW_GAP: f32 = 8.0;
/// Monospace line boxes sit optically low inside compact row chips.
pub const LIST_CHIP_TEXT_OFFSET_Y: f32 = -1.0;
pub const LIST_DROPDOWN_HEADER_H: f32 = 28.0;
/// Minimum sidebar width: fits the context row's index gutter, elided title,
/// close action, and pane-state pips without clipping their fixed chrome.
pub const SIDEBAR_MIN_W: f32 = 140.0;
/// Default width leaves room for a readable context headline beside its state.
pub const SIDEBAR_DEFAULT_W: f32 = 220.0;
/// Maximum sidebar width when the window is wide enough to allow it.
pub const SIDEBAR_MAX_W: f32 = 400.0;
/// Minimum usable width reserved for the central content area when the sidebar
/// is open. At the host's 400-point minimum window width (`400 - 260 = 140`),
/// this clamps the sidebar to [`SIDEBAR_MIN_W`] so the content area never drops
/// below 260 points.
pub const SIDEBAR_CONTENT_MIN_W: f32 = 260.0;
pub const LIST_DROPDOWN_CHEVRON_W: f32 = 24.0;
pub const TABLE_HEADER_H: f32 = 28.0;

// ── Overlay / modal chrome ─────────────────────────────────────────────────
/// Alpha (0-255) of the black scrim drawn behind modals. Higher = more of the
/// workspace is dimmed.
pub const SCRIM_ALPHA: u8 = 190;

/// Inner padding of a modal frame (horizontal, vertical). Applied via
/// `egui::Margin::symmetric`. Judged by screenshot at ppp 1.0 and 2.0 across
/// the command palette, the notes picker, and a confirm modal: the frame's
/// own border and shadow already separate it from the scrim, so the padding
/// only has to keep text off the edge. Past ~12/10 the gap reads as dead air
/// rather than as breathing room.
pub const MODAL_PADDING_H: i8 = 12;
pub const MODAL_PADDING_V: i8 = 10;

/// Height of one `ActionModal` action row (shortcut chips + label). Sits
/// between a dense list row and a form button: the chips need breathing room
/// but a confirm modal's action stack must not read as a menu.
pub const MODAL_ACTION_ROW_H: f32 = 30.0;

// ── Pane ID overlay ────────────────────────────────────────────────────────
/// Font size for the ⌘-hold pane ID ghost number (large, centered over pane content).
pub const TEXT_PANE_ID_GHOST: f32 = 64.0;
/// Alpha (0–255) of the ghost pane ID number — dim enough that content reads through.
pub const PANE_ID_GHOST_ALPHA: u8 = 55;

// ── App protocol — Badge geometry ─────────────────────────────────────────
// Padding tokens for every host-painted badge pill. The host measures the
// label and sizes the pill, so the Python SDK never repeats these; it shares
// only RADIUS_BADGE above. `crate::ui::badge` is the one consumer that turns
// them into a rect — go through it rather than reading them directly.
pub const BADGE_PAD_H: f32 = 8.0; // horizontal padding (text-to-edge each side)
pub const BADGE_PAD_V: f32 = 3.0; // vertical padding (text-to-edge each side)
/// Floor width for a badge in the PGAP flex-layout path, where the label font
/// size is the app's choice rather than TEXT_META. Keeps a short status pill
/// from shrink-wrapping tighter than its neighbours in a row.
pub const BADGE_MIN_W: f32 = 32.0;

// ── App protocol — KeyChip geometry ──────────────────────────────────────
// Padding tokens for the host-rendered KeyChip / KeyChipRow DrawCommands.
pub const KEYCHIP_PAD_H: f32 = 5.0; // horizontal padding (text-to-edge each side)
pub const KEYCHIP_PAD_V: f32 = 3.0; // vertical padding
pub const KEYCHIP_MIN_W: f32 = 16.0; // floor width; prevents single-char cramping
pub const KEYCHIP_GAP: f32 = 2.0; // gap between chips in a KeyChipRow
pub const KEYCHIP_DESC_GAP: f32 = 10.0; // gap between last chip and description

// Pointer micro-motion. Keyboard navigation remains instantaneous.
pub const DURATION_PRESS_MS: u64 = 140;
pub const DURATION_MICRO_MS: u64 = 160;
pub const DURATION_TOAST_MS: u64 = 180;
pub const PRESS_SCALE: f32 = 0.97;
const _: () =
    assert!(DURATION_PRESS_MS <= 300 && DURATION_MICRO_MS <= 300 && DURATION_TOAST_MS <= 300);

/// CSS cubic-bezier(0.23, 1, 0.32, 1), evaluated by inverting its x curve.
pub fn ease_out_cubic(t: f32) -> f32 {
    if t <= 0.0 {
        return 0.0;
    }
    if t >= 1.0 {
        return 1.0;
    }
    let (mut lo, mut hi): (f32, f32) = (0.0, 1.0);
    for _ in 0..20 {
        let u = (lo + hi) * 0.5;
        let x = 3.0 * (1.0 - u).powi(2) * u * 0.23 + 3.0 * (1.0 - u) * u * u * 0.32 + u * u * u;
        if x < t {
            lo = u;
        } else {
            hi = u;
        }
    }
    1.0 - (1.0 - (lo + hi) * 0.5).powi(3)
}

/// A zero animation_time is the reduced-motion opt-out. Test builds are
/// deterministic; immediate pointer-down feedback remains enabled in both.
pub fn micro_motion(ctx: &egui::Context, id: egui::Id, target: bool, duration_ms: u64) -> f32 {
    let duration = if cfg!(test) || ctx.global_style().animation_time <= 0.0 {
        0.0
    } else {
        duration_ms as f32 / 1000.0
    };
    ctx.animate_bool_with_time_and_easing(id, target, duration, ease_out_cubic)
}

pub fn press_amount(response: &egui::Response) -> f32 {
    let down = response.enabled() && response.is_pointer_button_down_on();
    // Snap the animation manager to active on the press frame, then ease out
    // only on release. The hit rectangle never shrinks.
    let id = response.id.with("press");
    let was_down = response.ctx.data_mut(|data| {
        let old = data.get_temp::<bool>(id.with("held")).unwrap_or(false);
        data.insert_temp(id.with("held"), down);
        old
    });
    if down && !was_down {
        log::info!(
            "ui button active-press id={:?} scale={PRESS_SCALE}",
            response.id
        );
    }
    if down {
        response.ctx.animate_bool_with_time(id, true, 0.0)
    } else {
        micro_motion(&response.ctx, id, false, DURATION_PRESS_MS)
    }
}

pub fn elevation_shadow() -> egui::Shadow {
    egui::Shadow {
        offset: [0, 12],
        blur: 32,
        spread: 0,
        color: egui::Color32::from_black_alpha(96),
    }
}

/// Motorways flat offset shadow (`2px 2px 0px #0c0e14`, zero blur).
pub fn motorways_shadow(shadow_color: egui::Color32) -> egui::Shadow {
    egui::Shadow {
        offset: [2, 2],
        blur: 0,
        spread: 0,
        color: shadow_color,
    }
}

pub fn hairline_border(color: egui::Color32) -> egui::Stroke {
    egui::Stroke::new(1.0_f32, color.gamma_multiply(0.65))
}

#[cfg(test)]
mod motion_tests {
    use super::*;
    #[test]
    fn ui_has_no_accelerating_entrance_easing() {
        let banned = ["ease", "in"].join("-");
        for entry in std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/src/ui")).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|extension| extension == "rs") {
                assert!(
                    !std::fs::read_to_string(&path).unwrap().contains(&banned),
                    "{}",
                    path.display()
                );
            }
        }
    }

    #[test]
    fn motion_curve_is_fast_monotonic_and_decelerating() {
        assert_eq!(ease_out_cubic(0.0), 0.0);
        assert_eq!(ease_out_cubic(1.0), 1.0);
        assert!(ease_out_cubic(0.25) > 0.5);
        let mut previous = 0.0;
        for step in 1..=100 {
            let value = ease_out_cubic(step as f32 / 100.0);
            assert!(value >= previous);
            previous = value;
        }
        assert!(ease_out_cubic(0.25) > ease_out_cubic(0.5) - ease_out_cubic(0.25));
    }
}

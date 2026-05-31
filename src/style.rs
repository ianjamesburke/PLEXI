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
//! use crate::style;
//! ui.set_width(style::MODAL_WIDTH_MD);
//! ui.label(egui::RichText::new("Title").size(style::TEXT_TITLE_XL));
//! ```
//!
//! ## When to add a token
//! When two or more places need the same value. The token set here is
//! intentionally minimal — only the constants actually referenced in the
//! codebase. Add scale holes (e.g. `SPACE_XS`, `MODAL_WIDTH_SM`) as soon as
//! a migration needs them, not speculatively.

use egui::CornerRadius;

// ── Spacing scale ──────────────────────────────────────────────────────────
// 4-based scale. Use these for padding, margins, gaps between siblings.
pub const SPACE_SM: f32 = 8.0;
pub const SPACE_MD: f32 = 12.0;
pub const SPACE_XL: f32 = 24.0;

// ── Typography scale ───────────────────────────────────────────────────────
// Point sizes for `RichText::new(...).size(...)`.
pub const TEXT_HINT: f32 = 11.0;       // Keyboard hints, meta labels.
pub const TEXT_CAPTION: f32 = 12.0;    // Secondary info, small labels.
pub const TEXT_BODY: f32 = 14.0;       // Default body text, button labels.
pub const TEXT_TITLE_XL: f32 = 28.0;   // Primary modal title — the thing the
                                       // user reads first.

// ── Corner radii ───────────────────────────────────────────────────────────
pub const RADIUS_MD: CornerRadius = CornerRadius::same(8);
pub const RADIUS_LG: CornerRadius = CornerRadius::same(12);
// Badge-specific radius. At TEXT_HINT size the pill height is ~17 px;
// RADIUS_MD (8) is 94% of max-oval — a cliché perfect stadium. 6 gives
// visible corners while staying clearly rounded. Keep in sync with
// RADIUS_BADGE in plexi_sdk/ui.py.
pub const RADIUS_BADGE: f32 = 6.0;

// ── Modal widths ───────────────────────────────────────────────────────────
// Pick the smallest width that fits the content without crowding. Bigger is
// NOT better — oversized modals feel empty and force eye travel.
pub const MODAL_WIDTH_MD: f32 = 640.0;     // Palettes, compact modals.
pub const MODAL_WIDTH_NOTIFY: f32 = 760.0; // Notification modal — wider for breathing room.

// ── Button heights ─────────────────────────────────────────────────────────
pub const BUTTON_H_MD: f32 = 40.0;       // Standard form buttons.
pub const BUTTON_H_LG: f32 = 52.0;       // Primary action buttons in modals.

// ── Overlay / modal chrome ─────────────────────────────────────────────────
/// Alpha (0-255) of the black scrim drawn behind modals. Higher = more of the
/// workspace is dimmed.
pub const SCRIM_ALPHA: u8 = 190;

/// Inner padding of a modal frame (horizontal, vertical). Applied via
/// `egui::Margin::symmetric`.
pub const MODAL_PADDING_H: i8 = 32;
pub const MODAL_PADDING_V: i8 = 28;

// ── Pane ID overlay ────────────────────────────────────────────────────────
/// Font size for the ⌘-hold pane ID ghost number (large, centered over pane content).
pub const TEXT_PANE_ID_GHOST: f32 = 64.0;
/// Alpha (0–255) of the ghost pane ID number — dim enough that content reads through.
pub const PANE_ID_GHOST_ALPHA: u8 = 55;

// ── App protocol — Badge geometry ─────────────────────────────────────────
// Padding tokens for the host-rendered Badge DrawCommand. Shared with the
// Python SDK constants in plexi_sdk/ui.py so both sides agree on pill size.
pub const BADGE_PAD_H: f32 = 8.0;   // horizontal padding (text-to-edge each side)
pub const BADGE_PAD_V: f32 = 3.0;   // vertical padding (text-to-edge each side)
pub const BADGE_MIN_W: f32 = 32.0;  // floor width; prevents single-char scrunch

// ── App protocol — KeyChip geometry ──────────────────────────────────────
// Padding tokens for the host-rendered KeyChip / KeyChipRow DrawCommands.
pub const KEYCHIP_PAD_H: f32 = 5.0;    // horizontal padding (text-to-edge each side)
pub const KEYCHIP_PAD_V: f32 = 1.0;    // vertical padding
pub const KEYCHIP_MIN_W: f32 = 16.0;   // floor width; prevents single-char cramping
pub const KEYCHIP_GAP: f32 = 2.0;      // gap between chips in a KeyChipRow
pub const KEYCHIP_DESC_GAP: f32 = 10.0; // gap between last chip and description

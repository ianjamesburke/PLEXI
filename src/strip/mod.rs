//! Authoritative Niri-style scrolling column strip model and layout engine (`SPEC.md` R31–R36, R40, R42).
//!
//! Provides the tested authority boundary for strip state:
//! - Columns, stacks, widths, and focus
//! - Pure layout calculations for panes, dividers, peeks, and focus frames

pub mod layout;
pub mod model;

#[cfg(test)]
pub mod tests;

pub use layout::{layout_strip, StripViewport};
pub use model::{Column, ColumnId, ColumnWidth, Strip};

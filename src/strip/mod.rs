//! Legacy workspace metadata and the pane geometry snapshot exposed by the host.
//! Live layout is computed by each window's tile tree.

pub mod layout;
pub mod model;

#[cfg(test)]
pub mod tests;

pub use layout::{StripLayout, StripViewport};
pub use model::{Column, ColumnId, ColumnWidth, Strip};

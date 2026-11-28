//! The pane geometry snapshot exposed by the host. The logical strip is in `plexi-host-model`.
//! Live layout is computed by each window's tile tree.

pub mod layout;

pub use layout::{StripLayout, StripViewport};

//! The host's reducer state, free of any UI toolkit: pane identity and
//! selection, the picker reducer and the logical strip. Geometry, PTY
//! conversion, worker I/O and persistence stay in the host adapters.

pub mod model;
pub mod picker;
pub mod strip;

// Vendored crate: excluded from the first-party perf lint gate (`just perf-clippy`).
// Do not refactor vendored code to satisfy these lints — see issue #2026.
#![allow(
    clippy::perf,
    clippy::redundant_clone,
    clippy::needless_collect,
    clippy::large_enum_variant
)]

mod backend;
mod bindings;
mod contrast;
mod diag;
mod font;
mod graphics;
mod keyboard;
mod theme;
mod types;
mod view;

pub use backend::settings::BackendSettings;
pub use backend::tap::{ByteTapHandle, PtyByteTap, TapReader};
pub use backend::{
    BackendCommand, HeldResize, PtyEvent, PtyExitStatus, TerminalBackend,
    resize_wake_delay, TerminalMode, RESIZE_MAX_HOLD, RESIZE_SETTLE,
};
pub use types::Size;
#[cfg(unix)]
pub use backend::{shutdown_panes, ShutdownOutcome, PANE_LIFECYCLE_LOG_TARGET};
#[cfg(all(unix, any(test, feature = "testing")))]
pub use backend::pane_close_testing;
pub use bindings::{Binding, BindingAction, InputKind, KeyboardBinding};
pub use diag::{set_repaint_diag_hook, set_span_hook};
pub use contrast::{MIN_CONTRAST_MAX, MIN_CONTRAST_OFF};
pub use font::{FontSettings, TerminalFont};
pub use theme::{ColorPalette, TerminalTheme};
pub use view::{
    cancel_pointer_gesture, terminal_widget_id, TerminalView, TerminalViewState,
};

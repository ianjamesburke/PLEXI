//! Repaint-cause diagnostics hook.
//!
//! egui_term cannot depend on the host crate, so the host installs a function
//! pointer at startup and each recurring repaint site reports a stable label
//! through it. With no hook installed, `diag_note` is a no-op.

use std::sync::OnceLock;

static REPAINT_DIAG_HOOK: OnceLock<fn(&'static str)> = OnceLock::new();

/// Install the host's repaint diagnostics hook. First call wins; later calls
/// are ignored.
pub fn set_repaint_diag_hook(hook: fn(&'static str)) {
    let _ = REPAINT_DIAG_HOOK.set(hook);
}

/// Report a repaint cause label to the host hook, if one is installed.
/// Callable from any thread (PTY reader threads report output repaints).
pub(crate) fn diag_note(cause: &'static str) {
    if let Some(hook) = REPAINT_DIAG_HOOK.get() {
        hook(cause);
    }
}

/// Per-frame timing span hook (stint 0731 drag profiling), same
/// host-installs-a-function-pointer shape as [`REPAINT_DIAG_HOOK`] since
/// egui_term cannot depend on the host crate directly.
static SPAN_HOOK: OnceLock<fn(&'static str, u64)> = OnceLock::new();

/// Install the host's span-timing hook. First call wins; later calls are
/// ignored.
pub fn set_span_hook(hook: fn(&'static str, u64)) {
    let _ = SPAN_HOOK.set(hook);
}

/// Report `nanos` elapsed against `label` to the host hook, if one is
/// installed. Check [`span_hook_installed`] before timing to avoid an
/// `Instant::now()` call when no hook is present.
pub(crate) fn span_note(label: &'static str, nanos: u64) {
    if let Some(hook) = SPAN_HOOK.get() {
        hook(label, nanos);
    }
}

/// True once the host has installed a span-timing hook. Callers should gate
/// `Instant::now()` calls behind this so an uninstrumented build pays zero
/// timing cost.
pub(crate) fn span_hook_installed() -> bool {
    SPAN_HOOK.get().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static CALLS: AtomicU32 = AtomicU32::new(0);

    fn test_hook(label: &'static str) {
        assert_eq!(label, "terminal_pty_output");
        CALLS.fetch_add(1, Ordering::Relaxed);
    }

    #[test]
    fn installed_hook_receives_diag_note_calls() {
        // No hook installed yet: must be a silent no-op.
        diag_note("terminal_pty_output");
        assert_eq!(CALLS.load(Ordering::Relaxed), 0);

        set_repaint_diag_hook(test_hook);
        diag_note("terminal_pty_output");
        assert_eq!(CALLS.load(Ordering::Relaxed), 1);

        // Second install is ignored; the first hook keeps receiving calls.
        set_repaint_diag_hook(|_| panic!("second hook must not be installed"));
        diag_note("terminal_pty_output");
        assert_eq!(CALLS.load(Ordering::Relaxed), 2);
    }

    static SPAN_CALLS: AtomicU32 = AtomicU32::new(0);

    fn test_span_hook(label: &'static str, nanos: u64) {
        assert_eq!(label, "terminal_resize");
        assert_eq!(nanos, 42);
        SPAN_CALLS.fetch_add(1, Ordering::Relaxed);
    }

    #[test]
    fn installed_span_hook_receives_span_note_calls() {
        assert!(!span_hook_installed());
        // No hook installed yet: must be a silent no-op.
        span_note("terminal_resize", 42);
        assert_eq!(SPAN_CALLS.load(Ordering::Relaxed), 0);

        set_span_hook(test_span_hook);
        assert!(span_hook_installed());
        span_note("terminal_resize", 42);
        assert_eq!(SPAN_CALLS.load(Ordering::Relaxed), 1);

        // Second install is ignored; the first hook keeps receiving calls.
        set_span_hook(|_, _| panic!("second span hook must not be installed"));
        span_note("terminal_resize", 42);
        assert_eq!(SPAN_CALLS.load(Ordering::Relaxed), 2);
    }
}

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
}

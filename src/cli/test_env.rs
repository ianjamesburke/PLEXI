//! Shared test-environment guard for CLI tests that mutate process-wide env vars.
//!
//! `PLEXI_SOCKET`, `PLEXI_PANE_ID` and `PLEXI_CONTEXT_ROOT` are process-global,
//! but `cargo test` runs test functions on parallel threads. Module-local
//! mutexes do not serialise against each other: a `notify` test once replaced
//! the socket while an `open` test was still connecting (stint 0555), and a
//! `plugin_state` lock let its tests repoint the socket and pane id under
//! `pane` tests (stint 1697).
//!
//! Every test that sets or reads any of these variables must acquire
//! [`SocketEnvGuard`]. It is the single lock for all of them. Acquisition saves
//! and clears every owned variable, so a value exported in the caller's shell
//! never leaks into a test; drop restores the saved values, including on unwind.

use std::ffi::OsString;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

/// The process-wide variables this guard owns.
const OWNED: [&str; 3] = ["PLEXI_SOCKET", "PLEXI_PANE_ID", "PLEXI_CONTEXT_ROOT"];

/// The one lock serialising mutation of [`OWNED`] across all test modules.
static SOCKET_ENV_LOCK: Mutex<()> = Mutex::new(());

/// The owned variables' values at capture. Capture clears them; drop restores
/// them. Only constructed while [`SOCKET_ENV_LOCK`] is held.
struct EnvSnapshot([Option<OsString>; 3]);

impl EnvSnapshot {
    fn take() -> Self {
        let previous = OWNED.map(std::env::var_os);
        for key in OWNED {
            std::env::remove_var(key);
        }
        Self(previous)
    }
}

impl Drop for EnvSnapshot {
    fn drop(&mut self) {
        for (key, value) in OWNED.iter().zip(&self.0) {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }
}

/// Exclusive access to the process-wide host-endpoint variables for one test.
///
/// Acquire with [`socket_env_guard`]. Every owned variable starts unset; the
/// values found at acquisition are restored when the guard drops.
pub struct SocketEnvGuard {
    // Field order is drop order: restore the values, then release the lock.
    _snapshot: EnvSnapshot,
    _lock: MutexGuard<'static, ()>,
}

impl SocketEnvGuard {
    /// Point `PLEXI_SOCKET` at `path` for the remainder of this guard's life.
    pub fn set(&self, path: impl AsRef<Path>) {
        std::env::set_var("PLEXI_SOCKET", path.as_ref().as_os_str());
    }

    /// Remove `PLEXI_SOCKET`, simulating a CLI invoked outside a Plexi pane.
    pub fn unset(&self) {
        std::env::remove_var("PLEXI_SOCKET");
    }

    /// Set `PLEXI_PANE_ID`, simulating a CLI invoked inside that pane.
    pub fn set_pane_id(&self, pane_id: &str) {
        std::env::set_var("PLEXI_PANE_ID", pane_id);
    }

    /// Set the spawn-time `PLEXI_CONTEXT_ROOT` a pane inherited.
    pub fn set_context_root(&self, root: impl AsRef<Path>) {
        std::env::set_var("PLEXI_CONTEXT_ROOT", root.as_ref().as_os_str());
    }
}

impl Drop for SocketEnvGuard {
    fn drop(&mut self) {
        super::clear_command_socket_override();
    }
}

/// Acquire the process-wide host-endpoint test lock, with every owned variable unset.
///
/// A poisoned lock is recovered rather than propagated: poisoning means an
/// earlier test panicked while holding it, and that test's own failure is the
/// signal — cascading the poison into every other socket test only hides it.
pub fn socket_env_guard() -> SocketEnvGuard {
    let lock = SOCKET_ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    SocketEnvGuard {
        _snapshot: EnvSnapshot::take(),
        _lock: lock,
    }
}

#[cfg(test)]
mod tests {
    use super::{socket_env_guard, EnvSnapshot, OWNED};

    // The lock is not reentrant, so each test holds one guard and exercises
    // the snapshot it wraps directly, under that guard's lock.

    #[test]
    fn acquisition_clears_every_owned_variable() {
        let _env = socket_env_guard();
        for key in OWNED {
            assert!(std::env::var_os(key).is_none(), "{key} must start unset");
        }
    }

    #[test]
    fn snapshot_hides_then_restores_a_caller_shell_value() {
        let env = socket_env_guard();
        std::env::set_var("PLEXI_PANE_ID", "shell-7");
        std::env::set_var("PLEXI_SOCKET", "/tmp/plexi-shell.sock");
        {
            let _snapshot = EnvSnapshot::take();
            assert!(std::env::var_os("PLEXI_PANE_ID").is_none());
            env.set_pane_id("42");
            env.set("/tmp/plexi-guard-probe.sock");
            env.set_context_root("/tmp/plexi-probe-root");
        }
        assert_eq!(std::env::var("PLEXI_PANE_ID").as_deref(), Ok("shell-7"));
        assert_eq!(
            std::env::var("PLEXI_SOCKET").as_deref(),
            Ok("/tmp/plexi-shell.sock")
        );
        assert!(std::env::var_os("PLEXI_CONTEXT_ROOT").is_none());
    }

    #[test]
    fn snapshot_restores_when_a_test_panics() {
        let env = socket_env_guard();
        std::env::set_var("PLEXI_PANE_ID", "shell-9");
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _snapshot = EnvSnapshot::take();
            env.set_pane_id("42");
            panic!("probe unwind");
        }));
        assert!(result.is_err());
        assert_eq!(std::env::var("PLEXI_PANE_ID").as_deref(), Ok("shell-9"));
    }
}

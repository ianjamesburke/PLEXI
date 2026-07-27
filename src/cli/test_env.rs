//! Shared test-environment guard for CLI tests that mutate process-wide env vars.
//!
//! `PLEXI_SOCKET` is process-global, but `cargo test` runs test functions on
//! parallel threads. Module-local mutexes (one in `open.rs`, one in `notify.rs`)
//! do not serialise against each other, so a `notify` test could replace or
//! remove the socket while an `open` test was still connecting — a
//! nondeterministically red `cargo test --bin plexi` (stint 0555).
//!
//! Every test that touches `PLEXI_SOCKET` must acquire [`SocketEnvGuard`]. It is
//! the single lock for that variable, and it restores the pre-test value on drop
//! so a panicking assertion cannot leak state into the next test.

use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

/// The one lock serialising `PLEXI_SOCKET` mutation across all CLI test modules.
static SOCKET_ENV_LOCK: Mutex<()> = Mutex::new(());

/// Exclusive access to the process-wide `PLEXI_SOCKET` variable for one test.
///
/// Acquire with [`socket_env_guard`]. The previous value is captured at
/// acquisition and restored when the guard drops, including on unwind.
pub struct SocketEnvGuard {
    previous: Option<OsString>,
    // Held for the guard's lifetime; released on drop after the value is restored.
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
}

impl Drop for SocketEnvGuard {
    fn drop(&mut self) {
        match self.previous.as_deref() {
            Some(value) => restore(value),
            None => std::env::remove_var("PLEXI_SOCKET"),
        }
    }
}

fn restore(value: &OsStr) {
    std::env::set_var("PLEXI_SOCKET", value);
}

/// Acquire the process-wide `PLEXI_SOCKET` test lock.
///
/// A poisoned lock is recovered rather than propagated: poisoning means an
/// earlier test panicked while holding it, and that test's own failure is the
/// signal — cascading the poison into every other socket test only hides it.
pub fn socket_env_guard() -> SocketEnvGuard {
    let lock = SOCKET_ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    SocketEnvGuard {
        previous: std::env::var_os("PLEXI_SOCKET"),
        _lock: lock,
    }
}

#[cfg(test)]
mod tests {
    use super::socket_env_guard;

    // The lock is not reentrant, so these tests acquire guards sequentially —
    // never nested — exactly as the CLI test helpers do.

    #[test]
    fn guard_restores_the_value_it_found_on_drop() {
        let before = {
            let env = socket_env_guard();
            let before = std::env::var_os("PLEXI_SOCKET");
            env.set("/tmp/plexi-guard-probe.sock");
            assert_eq!(
                std::env::var("PLEXI_SOCKET").as_deref(),
                Ok("/tmp/plexi-guard-probe.sock")
            );
            before
        };

        let env = socket_env_guard();
        assert_eq!(
            std::env::var_os("PLEXI_SOCKET"),
            before,
            "dropping a guard must restore the value captured at acquisition"
        );
        drop(env);
    }

    #[test]
    fn guard_restores_after_unset() {
        let before = {
            let env = socket_env_guard();
            let before = std::env::var_os("PLEXI_SOCKET");
            env.unset();
            assert!(std::env::var_os("PLEXI_SOCKET").is_none());
            before
        };

        let env = socket_env_guard();
        assert_eq!(
            std::env::var_os("PLEXI_SOCKET"),
            before,
            "an unset inside the guard must not leak past its drop"
        );
        drop(env);
    }

    #[test]
    fn guard_restores_when_a_test_panics() {
        let mut before = None;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let env = socket_env_guard();
            before = Some(std::env::var_os("PLEXI_SOCKET"));
            env.set("/tmp/plexi-panicking-guard.sock");
            panic!("probe unwind");
        }));
        assert!(result.is_err());

        let env = socket_env_guard();
        assert_eq!(
            std::env::var_os("PLEXI_SOCKET"),
            before.expect("captured under the socket lock"),
            "unwinding must restore the value captured at acquisition"
        );
        drop(env);
    }
}

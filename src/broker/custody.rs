//! Host-only custody of sealed permission state (stint 1291).
//!
//! The seal key and every sealed file belong to the host process. The CLI is
//! the same signed binary, so the keychain cannot tell the two apart; this
//! module is what does.
//!
//! What it guarantees, exactly:
//! - Every primitive that reads the seal key or reads or writes sealed state
//!   (`host_key::get`/`add_new`, `seal::*` key, read and write paths,
//!   `GrantStore`/`PermissionStore` load and save, `PermissionMonitor::open`)
//!   takes a [`HostProcess`].
//! - Only the broker and the host start path can obtain one. The constructor
//!   is private to this module. [`install_for_host_start`] mints the only
//!   production token, and `main` calls it once, on the host start path
//!   (`only_host_start_installs_custody` pins both facts). The accessor is
//!   `pub(in crate::broker)`.
//! - A process that never ran host start (every CLI command, including
//!   `plexi host start`, which launches a separate host process) has no token,
//!   so `PermissionMonitor::for_profile` and the store loaders fail loudly at
//!   runtime with [`NotHostProcess`].
//!
//! It does not claim that code under `src/cli` cannot compile a call to
//! `for_profile`: it can, and the call fails at runtime. It does not stop
//! another process that runs this signed binary as a host (stints 1282, 1288).

use std::sync::OnceLock;

/// Proof that this process is the Plexi host. Zero-sized; copying one only
/// copies a token the process already holds.
#[derive(Debug, Clone, Copy)]
pub struct HostProcess {
    _private: (),
}

/// A sealed-state operation ran in a process that is not the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotHostProcess;

impl std::fmt::Display for NotHostProcess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(
            "permission state is held by the running Plexi host, and this process is not the host. \
             Commands cannot read or change permissions themselves; start Plexi and approve in its window",
        )
    }
}

impl std::error::Error for NotHostProcess {}

impl From<NotHostProcess> for String {
    fn from(error: NotHostProcess) -> Self {
        error.to_string()
    }
}

/// Custody was installed twice in one process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CustodyAlreadyInstalled;

impl std::fmt::Display for CustodyAlreadyInstalled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("host custody was already installed in this process")
    }
}

fn installed() -> &'static OnceLock<HostProcess> {
    static INSTALLED: OnceLock<HostProcess> = OnceLock::new();
    &INSTALLED
}

/// Mint this process's host token. Call once, on the host start path, before
/// anything opens permission state. A second call is an error and leaves the
/// first token in place.
pub fn install_for_host_start() -> Result<(), CustodyAlreadyInstalled> {
    installed()
        .set(HostProcess { _private: () })
        .map_err(|_| CustodyAlreadyInstalled)?;
    log::info!("custody: host process token installed; this process seals permission state");
    Ok(())
}

/// The host token, or [`NotHostProcess`] in any other process.
#[cfg(not(test))]
pub(in crate::broker) fn host_process() -> Result<HostProcess, NotHostProcess> {
    installed().get().copied().ok_or(NotHostProcess)
}

/// Tests run as the host unless a test opts into the CLI role with
/// [`as_cli_process`] on its own thread.
#[cfg(test)]
pub(in crate::broker) fn host_process() -> Result<HostProcess, NotHostProcess> {
    if CLI_ROLE.with(|role| role.get()) {
        return Err(NotHostProcess);
    }
    Ok(HostProcess::for_test())
}

#[cfg(test)]
thread_local! {
    static CLI_ROLE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Run `body` on this thread as a process that never ran host start.
#[cfg(test)]
pub(crate) fn as_cli_process<T>(body: impl FnOnce() -> T) -> T {
    CLI_ROLE.with(|role| role.set(true));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(body));
    CLI_ROLE.with(|role| role.set(false));
    match result {
        Ok(value) => value,
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[cfg(test)]
impl HostProcess {
    /// The test constructor. Production has exactly one other:
    /// [`install_for_host_start`].
    pub(crate) const fn for_test() -> Self {
        HostProcess { _private: () }
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                rs_files(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }

    /// Guard: exactly one production way to mint a `HostProcess` (inside
    /// `install_for_host_start`), one `#[cfg(test)]` constructor, and exactly
    /// one caller of `install_for_host_start` (the host start path in `main`).
    #[test]
    fn only_host_start_installs_custody() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        rs_files(&root, &mut files);
        assert!(!files.is_empty(), "no sources found under {root:?}");
        let mut mints = Vec::new();
        let mut installs = Vec::new();
        let (mint, install) = (
            ["HostProcess", " { _private"].concat(),
            ["install_for_host_start", "()"].concat(),
        );
        for path in &files {
            let Ok(text) = std::fs::read_to_string(path) else {
                continue;
            };
            let rel = path.strip_prefix(&root).unwrap_or(path).display().to_string();
            for (n, line) in text.lines().enumerate() {
                let code = line.split("//").next().unwrap_or("");
                if code.contains(&mint) {
                    mints.push(format!("{rel}:{}", n + 1));
                }
                if code.contains(&install)
                    && !code.contains("pub fn ")
                    && rel != "broker/custody.rs"
                {
                    installs.push(format!("{rel}:{}", n + 1));
                }
            }
        }
        assert_eq!(
            mints.len(),
            2,
            "HostProcess must have one production mint and one test constructor: {mints:?}"
        );
        assert!(
            mints.iter().all(|site| site.starts_with("broker/custody.rs:")),
            "HostProcess may only be constructed in broker/custody.rs: {mints:?}"
        );
        assert_eq!(
            installs.len(),
            1,
            "exactly one caller may install host custody: {installs:?}"
        );
        assert!(
            installs[0].starts_with("main.rs:"),
            "host custody is installed only on the host start path in main.rs: {installs:?}"
        );
    }

    /// Stint 1291 pin: a process that never ran host start cannot open,
    /// read, migrate or seal permission state, and says why. A leftover legacy
    /// `permissions.toml` is not imported (that import seals).
    #[test]
    fn cli_process_cannot_seal() {
        let profile = tempfile::tempdir().unwrap();
        let legacy = profile.path().join("permissions.toml");
        std::fs::write(&legacy, "[entries]\n").unwrap();
        super::as_cli_process(|| {
            let monitor = crate::broker::gate::PermissionMonitor::for_profile(profile.path());
            assert_eq!(monitor.err(), Some(super::NotHostProcess));
            assert_eq!(
                crate::broker::GrantStore::load_or_default(profile.path()).err(),
                Some(super::NotHostProcess)
            );
            assert_eq!(
                crate::app::permissions::PermissionStore::load_or_default(profile.path()).err(),
                Some(super::NotHostProcess)
            );
            assert!(super::NotHostProcess
                .to_string()
                .contains("start Plexi and approve in its window"));
        });
        assert_eq!(std::fs::read_to_string(&legacy).unwrap(), "[entries]\n");
        for written in ["grants.toml", "permission-audit.jsonl", "host"] {
            assert!(
                !profile.path().join(written).exists(),
                "a CLI process wrote {written}"
            );
        }
        // An in-memory store has no custody and nothing to write.
        assert!(crate::broker::GrantStore::default().try_save().is_ok());
    }

    #[test]
    fn installing_twice_is_an_error() {
        // The process-wide slot is shared by every test, so assert only on
        // the second of two calls.
        let _ = super::install_for_host_start();
        assert_eq!(
            super::install_for_host_start(),
            Err(super::CustodyAlreadyInstalled)
        );
    }

    #[test]
    fn cli_role_has_no_token() {
        assert!(super::host_process().is_ok());
        super::as_cli_process(|| {
            assert_eq!(super::host_process().unwrap_err(), super::NotHostProcess);
        });
        assert!(super::host_process().is_ok());
    }
}

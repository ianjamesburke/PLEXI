//! Approved typed one-shot CLI operations (stint 1175).
//!
//! Contract: `docs/approved-app-cli-v1.md`. Definitions live in the plugin package
//! (`crate::app::cli_operation`); this module owns executable identity, the
//! sealed private copy that is actually launched, and the bounded supervisor.

pub mod exec;
pub mod service;

use std::path::PathBuf;

/// Where sealed program copies live: `<seal_base>/<app id>/<program>/<sha256>/`.
pub(crate) fn seal_base() -> PathBuf {
    crate::config::config_dir().join("run").join("cli-ops")
}

/// App ids that are safe to use as one directory name.
pub(crate) fn plugin_id_is_path_safe(plugin_id: &str) -> bool {
    !plugin_id.is_empty()
        && !plugin_id.starts_with('.')
        && plugin_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// Remove every sealed program copy an app left behind. An uninstalled app must
/// not leave executable copies on disk.
pub(crate) fn purge_app_copies(plugin_id: &str) {
    if !plugin_id_is_path_safe(plugin_id) {
        return;
    }
    let dir = seal_base().join(plugin_id);
    match std::fs::remove_dir_all(&dir) {
        Ok(()) => log::info!("cli_ops: removed sealed copies of uninstalled app {plugin_id}"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => log::warn!(
            "cli_ops: could not remove sealed copies {}: {error}",
            dir.display()
        ),
    }
}

/// Directories a bare executable name is looked up in.
pub(crate) fn search_dirs() -> Vec<PathBuf> {
    #[cfg(test)]
    if let Some(dirs) = TEST_SEARCH_DIRS.with(|c| c.borrow().clone()) {
        return dirs;
    }
    exec::default_search_dirs()
}

#[cfg(test)]
thread_local! {
    static TEST_SEARCH_DIRS: std::cell::RefCell<Option<Vec<PathBuf>>> =
        const { std::cell::RefCell::new(None) };
}

/// Point executable lookup at `dirs` for the current test thread.
#[cfg(test)]
pub(crate) struct TestSearchDirsGuard;

#[cfg(test)]
impl Drop for TestSearchDirsGuard {
    fn drop(&mut self) {
        TEST_SEARCH_DIRS.with(|c| *c.borrow_mut() = None);
    }
}

#[cfg(test)]
pub(crate) fn set_test_search_dirs(dirs: Vec<PathBuf>) -> TestSearchDirsGuard {
    TEST_SEARCH_DIRS.with(|c| *c.borrow_mut() = Some(dirs));
    TestSearchDirsGuard
}

#[cfg(test)]
pub(crate) mod fixture;

#[cfg(test)]
mod exec_tests;

#[cfg(test)]
mod service_tests;

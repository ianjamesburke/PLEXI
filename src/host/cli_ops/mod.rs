//! Approved typed one-shot CLI operations (stint 1175).
//!
//! Contract: `docs/approved-app-cli-v1.md`. Definitions live in the app package
//! (`crate::app::cli_operation`); this module owns executable identity, the
//! sealed private copy that is actually launched, and the bounded supervisor.

pub mod exec;
pub mod service;

use std::path::PathBuf;

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

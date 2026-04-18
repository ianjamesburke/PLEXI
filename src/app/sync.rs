//! CWD synchronization — polls linked terminals and broadcasts PathChanged to pane groups.

use super::PlexiApp;

impl PlexiApp {
    /// Pane-group cwd sync is handled in the app-pane runtime.
    /// The terminal overlay path was removed in the v3 pane refactor.
    pub(super) fn sync_app_cwd(&mut self) {
        let _ = self;
    }
}

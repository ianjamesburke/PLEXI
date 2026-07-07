//! Shared blocking HTTP helpers for the hosted marketplace + accounts clients.
//!
//! Both [`crate::app::account`] and [`crate::app::marketplace`] talk to
//! plexiapp.com with the same pure-Rust `ureq` setup (no tokio) and the same
//! base-plus-path URL joining. This module owns that shared shape; each caller
//! keeps its own per-endpoint status-code dispatch, which differs by service.

use std::time::Duration;

/// Build a blocking `ureq` agent with a fixed 10s connect timeout and the given
/// overall request timeout. Pure-Rust, no tokio — matches the rest of the host's
/// network CLI commands.
pub fn agent(timeout: Duration) -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout(timeout)
        .build()
}

/// Join a base URL and a path with exactly one `/` between them, trimming any
/// stray slashes on either side.
pub fn join_url(base: &str, path: &str) -> String {
    format!(
        "{}/{}",
        base.trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}

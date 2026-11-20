//! The authority a terminal pane runs under, bound before any credential for
//! that pane exists (stint 1632).
//!
//! Construction creates one immutable [`PaneAuthority`] for the pane's real
//! id, then issues the pane's call credential and host-MCP bearer from it.
//! Each credential registry holds the same binding, so every path that
//! authenticates one of those credentials resolves the pane's ceiling from
//! the credential itself, from the first instant it exists, without the
//! finished pane. A pane credential without a binding authenticates nothing:
//! there is no fallback to a weaker posture.
//!
//! Bindings are not kept in a table keyed by pane id: pane ids are per host
//! instance, and a binding must never be found for a different pane that
//! happens to share an id.

use std::path::PathBuf;

use super::PermissionPosture;

/// What a terminal pane may do, fixed when it is constructed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PaneAuthority {
    pub(crate) pane_id: u64,
    /// Permission ceiling; `None` leaves the broker's default (`Ask`).
    pub(crate) posture: Option<PermissionPosture>,
    /// Host-verified provenance (`human`, `agent:<role>`, `system`, ...).
    pub(crate) spawned_by: Option<String>,
    /// Destination context the credentials are scoped to.
    pub(crate) context_id: u64,
    /// Workspace root the credentials are scoped to at issuance.
    pub(crate) workspace_root: PathBuf,
    /// The directory-derived secret classes the pane was constructed with.
    /// Persisted with the posture so a restored pane gets exactly these.
    pub(crate) secrets: crate::host::shell::SecretPolicy,
}

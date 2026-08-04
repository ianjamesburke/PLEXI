//! Host-owned scope/ownership model (stint 0724).
//!
//! `ScopeOrigin`/`Scope`/`evaluate_reach` are the one reachability predicate
//! connector tools, the event bus, directed/typed pipes, and app discovery
//! route through instead of each reimplementing its own workspace-path or
//! active-context comparison.
//!
//! # Why a host-owned origin
//!
//! Every prior resource-scoping bug in this codebase traced back to the same
//! root cause: some caller derived "which context/pane is this for" from
//! ambient state (`active_window`, `router.active()`, a client-supplied id) at
//! the moment of use, rather than from an identity the host established once,
//! at the trust boundary. `ScopeOrigin` is that identity. It is constructed
//! only by the host model (`PlexiApp::origin_for_pane`) from host-observed
//! facts — the pane table, the window's `context_id`, the router's
//! `Context.root` — never from a request payload, cwd, or env var.
//! Socket-peer callers (e.g. `PlexiApp::resolve_event_bus_caller`) resolve
//! ancestry to a pane id first via stint 0636's `resolve_socket_peer_pane`
//! (kernel-captured ancestry, not the wire), then hand that pane id to
//! `origin_for_pane` — never re-deriving context/window from the caller's
//! own claims.

use std::path::PathBuf;

/// Host-established identity of the acting pane at the dispatch boundary.
///
/// Constructed ONLY by the host model or from socket-peer resolution (stint
/// 0636's `resolve_socket_peer_pane`) — never from client payloads, active
/// focus, `active_window`, `router.active()`, pane cwd, or ambient env vars.
#[derive(Debug, Clone, PartialEq)]
pub struct ScopeOrigin {
    pub context_id: u64,
    pub context_root: PathBuf,
    pub window_id: u64,
    pub pane_id: u64,
    pub app_id: Option<String>,
}

/// The scope a resource (app instance, connector tool, event stream, pipe,
/// ...) is owned at. Distinct from `crate::host::state_scope::StateScope`,
/// which is app-declared addressing for on-disk state files; `Scope` is the
/// host's ownership/visibility model for in-memory and wire resources.
///
/// Every resource this model currently owns is either pane-scoped or
/// app-instance-scoped — nothing in this codebase today has a genuinely
/// global (visible-everywhere) or bare-context/window-scoped resource with
/// no owning pane, so those broader variants aren't declared here. Add one
/// back only when a real consumer needs it (matches `RegistryViews`'s own
/// decision not to adopt `resolve_layers` speculatively — see stint 0724
/// Phase F's follow-up notes).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Scope {
    /// Visible only within the owning context — never cross-context even if
    /// another context happens to have a pane with the same `pane_id`.
    Pane { pane_id: u64, context_id: u64 },
    /// Visible only within the owning context — never cross-context even if
    /// another context happens to have a pane/app_id pair that collides.
    AppInstance {
        pane_id: u64,
        app_id: String,
        context_id: u64,
    },
}

/// Minimal in-memory attributable grant carrier. NOT persisted — persistence
/// for cross-context grants is a different, out-of-scope stint. This exists
/// only so `evaluate_reach` has a grant path to evaluate against in Phase A
/// and the phases that migrate consumers onto it.
#[derive(Debug, Clone)]
pub struct CrossContextGrant {
    pub granting_context_id: u64,
    pub grantee_context_id: u64,
    pub reason: String,
}

/// Outcome of `evaluate_reach`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    /// Owner scope is inherently visible to this viewer (global, or the
    /// viewer's own context).
    Allowed,
    /// Owner scope is a different context, but a matching `CrossContextGrant`
    /// makes it visible anyway.
    AllowedByGrant,
    /// Owner scope is a different context and no matching grant exists.
    Rejected,
}

/// The one central reachability predicate connector tools, the event bus,
/// and directed/typed pipes call to decide whether a viewer in
/// `viewer_context_id` may see a resource owned at `owner`.
///
/// Rule: every scope is context-owned and visible only in the owning
/// context, unless an explicit `CrossContextGrant` names exactly this
/// (owner_context, viewer_context) pair — and even with a grant, reachability
/// is decided purely by `context_id`. A `Pane`/`AppInstance` scope is never
/// reachable from another context merely because the numeric `pane_id`
/// happens to match one there; ids are only unique within their owning
/// context.
///
/// `AllowedByGrant` and `Rejected` are both logged at `info!` — an allowed
/// cross-context grant use names the grant's `reason`, a denied crossing
/// names the scope's shape — without ever logging the resource's payload:
/// `Scope` carries no secret, notification-body, or event-payload fields.
pub fn evaluate_reach(
    owner: &Scope,
    viewer_context_id: u64,
    grant: Option<&CrossContextGrant>,
) -> Reach {
    let owner_context_id = match owner {
        Scope::Pane { context_id, .. } | Scope::AppInstance { context_id, .. } => *context_id,
    };

    if owner_context_id == viewer_context_id {
        return Reach::Allowed;
    }

    if let Some(grant) = grant {
        if grant.granting_context_id == owner_context_id
            && grant.grantee_context_id == viewer_context_id
        {
            log::info!(
                target: "plexi::scope",
                "reach allowed by grant: owner_context={owner_context_id} viewer_context={viewer_context_id} reason={}",
                grant.reason
            );
            return Reach::AllowedByGrant;
        }
    }

    log::info!(
        target: "plexi::scope",
        "reach rejected: owner_context={owner_context_id} viewer_context={viewer_context_id} scope={owner:?}"
    );
    Reach::Rejected
}

/// A lifecycle event that makes some previously-resolved scope/origin/source
/// stale. `PlexiApp::emit_scope_invalidation` fans this out to every live
/// consumer (`RegistryViews::invalidate`, per-pane app-state root refresh)
/// so each reacts to one shared vocabulary of "what changed" instead of
/// re-deriving it from ad hoc call sites.
#[derive(Debug, Clone)]
pub enum ScopeInvalidation {
    /// `pane_ops::workspace::set_context_root` moved a context's anchor.
    ContextRootChanged {
        context_id: u64,
        old_root: PathBuf,
        new_root: PathBuf,
    },
    /// `pane_ops::workspace::delete_context` removed a context (emitted once
    /// per deleted context in a cascading delete, including descendants).
    ContextRemoved { context_id: u64 },
    /// `pane_ops::layout::close_pane_by_id` closed a pane.
    PaneClosed { pane_id: u64, context_id: u64 },
    /// A filesystem-watched source (e.g. the app registry) reported a change
    /// generation the host has not yet resolved against.
    SourceGenerationChanged { source_path: PathBuf },
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── evaluate_reach: the scope compatibility matrix ──────────────────────

    fn grant(granting: u64, grantee: u64) -> CrossContextGrant {
        CrossContextGrant {
            granting_context_id: granting,
            grantee_context_id: grantee,
            reason: "test grant".to_string(),
        }
    }

    #[test]
    fn pane_scope_same_context_is_allowed() {
        let owner = Scope::Pane {
            pane_id: 200,
            context_id: 5,
        };
        assert_eq!(evaluate_reach(&owner, 5, None), Reach::Allowed);
    }

    #[test]
    fn pane_scope_matching_pane_id_in_another_context_is_still_rejected() {
        let owner = Scope::Pane {
            pane_id: 200,
            context_id: 5,
        };
        assert_eq!(evaluate_reach(&owner, 6, None), Reach::Rejected);
    }

    #[test]
    fn pane_scope_different_context_with_grant_is_allowed_by_grant() {
        let owner = Scope::Pane {
            pane_id: 200,
            context_id: 5,
        };
        let g = grant(5, 6);
        assert_eq!(evaluate_reach(&owner, 6, Some(&g)), Reach::AllowedByGrant);
    }

    #[test]
    fn pane_scope_grant_naming_a_different_pair_is_still_rejected() {
        let owner = Scope::Pane {
            pane_id: 200,
            context_id: 5,
        };
        // Grant exists but names a different grantor/grantee pair.
        let wrong_grantee = grant(5, 999);
        assert_eq!(
            evaluate_reach(&owner, 6, Some(&wrong_grantee)),
            Reach::Rejected
        );
        let wrong_grantor = grant(4, 6);
        assert_eq!(
            evaluate_reach(&owner, 6, Some(&wrong_grantor)),
            Reach::Rejected
        );
    }

    #[test]
    fn app_instance_scope_same_context_is_allowed() {
        let owner = Scope::AppInstance {
            pane_id: 200,
            app_id: "acme.todo".to_string(),
            context_id: 5,
        };
        assert_eq!(evaluate_reach(&owner, 5, None), Reach::Allowed);
    }

    #[test]
    fn app_instance_scope_matching_ids_in_another_context_is_still_rejected() {
        let owner = Scope::AppInstance {
            pane_id: 200,
            app_id: "acme.todo".to_string(),
            context_id: 5,
        };
        assert_eq!(evaluate_reach(&owner, 6, None), Reach::Rejected);
    }

    #[test]
    fn app_instance_scope_different_context_with_grant_is_allowed_by_grant() {
        let owner = Scope::AppInstance {
            pane_id: 200,
            app_id: "acme.todo".to_string(),
            context_id: 5,
        };
        let g = grant(5, 6);
        assert_eq!(evaluate_reach(&owner, 6, Some(&g)), Reach::AllowedByGrant);
    }

    // ── ScopeInvalidation / emit_scope_invalidation smoke test ──────────────

    /// `PlexiApp::set_context_root` must fire `emit_scope_invalidation` with a
    /// `ContextRootChanged` event. The observable proof is the test-only
    /// counter `emit_scope_invalidation` increments under `#[cfg(test)]` — a
    /// HostHarness-based smoke test per TESTING.md (host logic → HostHarness,
    /// not a TOML scene, since nothing here is pixel-observable).
    #[test]
    fn set_context_root_fires_scope_invalidation() {
        let mut harness = crate::testing::HostHarness::new();
        let before =
            crate::app::SCOPE_INVALIDATION_COUNT_FOR_TEST.load(std::sync::atomic::Ordering::SeqCst);

        let new_root = tempfile::tempdir().expect("new root tempdir");
        harness
            .app
            .set_context_root(new_root.path().to_path_buf(), None);

        let after =
            crate::app::SCOPE_INVALIDATION_COUNT_FOR_TEST.load(std::sync::atomic::Ordering::SeqCst);
        assert!(
            after > before,
            "set_context_root must fire emit_scope_invalidation (before={before}, after={after})"
        );
    }
}

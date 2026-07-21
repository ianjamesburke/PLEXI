//! Per-frame text-surface registry (stint 0429).
//!
//! Text widgets never call `request_focus`/`surrender_focus` themselves.
//! Instead they register their egui id against the pane or overlay that owns
//! them, and the post-frame reconciler
//! (`crate::app::input_owner::PlexiApp::reconcile_egui_focus`) projects the
//! host's single `InputOwner` onto egui focus: the owner's surface gets
//! focus, every other registered surface surrenders it. Stale focus is
//! structurally impossible because focus is re-derived from host state every
//! frame instead of being hand-managed by each widget.

use crate::app::input_owner::OverlaySurface;
use crate::spatial::tiling::PaneId;
use egui::Id;

const TEXT_SURFACE_REGISTRY_ID: &str = "plexi_text_surface_registry";

/// Which host entity a text surface belongs to. Mirrors the two
/// focus-bearing variants of [`crate::app::input_owner::InputOwner`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum SurfaceKey {
    Overlay(OverlaySurface),
    Pane(PaneId),
}

/// One frame's worth of surface registrations and claims.
#[derive(Clone, Default)]
pub(crate) struct SurfaceRegistry {
    /// `(key, id, is_default)` in registration order.
    pub(crate) surfaces: Vec<(SurfaceKey, Id, bool)>,
    /// Explicit focus claims; the last claim whose key matches the frame's
    /// input owner wins.
    pub(crate) claims: Vec<(SurfaceKey, Id)>,
}

impl SurfaceRegistry {
    /// True if `id` is registered (under any key).
    pub(crate) fn contains(&self, id: Id) -> bool {
        self.surfaces.iter().any(|(_, sid, _)| *sid == id)
    }

    /// True if `id` is registered under `key`.
    pub(crate) fn is_surface_of(&self, key: SurfaceKey, id: Id) -> bool {
        self.surfaces
            .iter()
            .any(|(skey, sid, _)| *skey == key && *sid == id)
    }

    /// The first surface registered as a default for `key`, if any.
    pub(crate) fn default_for(&self, key: SurfaceKey) -> Option<Id> {
        self.surfaces
            .iter()
            .find(|(skey, _, is_default)| *skey == key && *is_default)
            .map(|(_, sid, _)| *sid)
    }

    /// The last claim registered for `key`, if any.
    pub(crate) fn claim_for(&self, key: SurfaceKey) -> Option<Id> {
        self.claims
            .iter()
            .rev()
            .find(|(ckey, _)| *ckey == key)
            .map(|(_, cid)| *cid)
    }
}

fn with_registry(ctx: &egui::Context, f: impl FnOnce(&mut SurfaceRegistry)) {
    let registry_id = Id::new(TEXT_SURFACE_REGISTRY_ID);
    ctx.memory_mut(|memory| {
        let mut registry: SurfaceRegistry = memory.data.get_temp(registry_id).unwrap_or_default();
        f(&mut registry);
        memory.data.insert_temp(registry_id, registry);
    });
}

/// Register a text surface for this frame. The reconciler surrenders it when
/// its key is not the frame's input owner. A registered surface is never
/// auto-granted focus — use [`register_default_text_surface`] or
/// [`claim_text_surface`] for that.
pub(crate) fn register_text_surface(ctx: &egui::Context, key: SurfaceKey, id: Id) {
    register(ctx, key, id, false);
}

/// Register a text surface as its owner's default: when the owner holds no
/// focused surface at frame end, the reconciler grants focus here. The first
/// default registered for a key wins.
pub(crate) fn register_default_text_surface(ctx: &egui::Context, key: SurfaceKey, id: Id) {
    register(ctx, key, id, true);
}

fn register(ctx: &egui::Context, key: SurfaceKey, id: Id, is_default: bool) {
    with_registry(ctx, |registry| {
        match registry
            .surfaces
            .iter_mut()
            .find(|(skey, sid, _)| *skey == key && *sid == id)
        {
            Some((_, _, existing_default)) => *existing_default |= is_default,
            None => registry.surfaces.push((key, id, is_default)),
        }
    });
}

/// Claim focus for a surface this frame. Also registers it. Honored by the
/// reconciler only when `key` is the frame's input owner; a claim from a
/// non-owner is dropped.
pub(crate) fn claim_text_surface(ctx: &egui::Context, key: SurfaceKey, id: Id) {
    with_registry(ctx, |registry| {
        if !registry
            .surfaces
            .iter()
            .any(|(skey, sid, _)| *skey == key && *sid == id)
        {
            registry.surfaces.push((key, id, false));
        }
        registry.claims.push((key, id));
    });
}

/// Take this frame's registry, leaving it empty for the next frame. Called
/// once per frame by the reconciler.
pub(crate) fn drain_text_surfaces(ctx: &egui::Context) -> SurfaceRegistry {
    let registry_id = Id::new(TEXT_SURFACE_REGISTRY_ID);
    ctx.memory_mut(|memory| {
        let registry = memory.data.get_temp(registry_id).unwrap_or_default();
        memory.data.remove::<SurfaceRegistry>(registry_id);
        registry
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::input_owner::OverlaySurface;
    use crate::app::FocusKind;

    #[test]
    fn registry_dedupes_and_keeps_first_default() {
        let ctx = egui::Context::default();
        let key = SurfaceKey::Pane(7);
        let editor = Id::new("editor");
        let find = Id::new("find");

        register_default_text_surface(&ctx, key, editor);
        register_default_text_surface(&ctx, key, editor);
        register_text_surface(&ctx, key, find);

        let registry = drain_text_surfaces(&ctx);
        assert_eq!(registry.surfaces.len(), 2);
        assert_eq!(registry.default_for(key), Some(editor));
        assert!(registry.is_surface_of(key, find));
        assert!(drain_text_surfaces(&ctx).surfaces.is_empty());
    }

    #[test]
    fn claim_registers_and_last_claim_wins() {
        let ctx = egui::Context::default();
        let key = SurfaceKey::Overlay(OverlaySurface::Layer(FocusKind::CommandPalette));
        let first = Id::new("first");
        let second = Id::new("second");

        claim_text_surface(&ctx, key, first);
        claim_text_surface(&ctx, key, second);

        let registry = drain_text_surfaces(&ctx);
        assert_eq!(registry.claim_for(key), Some(second));
        assert!(registry.contains(first));
        assert_eq!(registry.claim_for(SurfaceKey::Pane(1)), None);
    }

    #[test]
    fn register_then_default_upgrades_in_place() {
        let ctx = egui::Context::default();
        let key = SurfaceKey::Pane(3);
        let id = Id::new("field");

        register_text_surface(&ctx, key, id);
        register_default_text_surface(&ctx, key, id);

        let registry = drain_text_surfaces(&ctx);
        assert_eq!(registry.surfaces.len(), 1);
        assert_eq!(registry.default_for(key), Some(id));
    }
}

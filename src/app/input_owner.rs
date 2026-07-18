//! Single-authority input ownership (stint 0429).
//!
//! Host pane focus and egui widget focus used to be reconciled by five
//! independent per-widget conventions (overlay focus registry special cases,
//! assistant/text-editor "request when active" rules, the terminal steal's
//! egui-focus gate, `modal_open` threading, one-shot overlay focus guards).
//! This module replaces all of them: [`PlexiApp::input_owner`] derives one
//! owner per frame from host state, text widgets register their egui ids in
//! `crate::ui::focus`, and [`PlexiApp::reconcile_egui_focus`] makes egui
//! focus a pure projection of that owner after every frame.
//!
//! This module is the only place in the crate allowed to call egui's
//! `request_focus`/`surrender_focus` (enforced by clippy `disallowed_methods`).

use crate::app::{FocusKind, PlexiApp};
use crate::spatial::tiling::PaneId;
use crate::ui::focus::{SurfaceKey, SurfaceRegistry};
use egui::Id;

/// Who owns keyboard input this frame. Derived, never stored — host state is
/// the single source of truth and this is a pure function of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum InputOwner {
    /// The OS window is unfocused; no surface receives keys.
    OsUnfocused,
    /// A host overlay surface owns input (modal focus stack or an inline
    /// sidebar editor).
    Overlay(OverlaySurface),
    /// The focused pane of the active window owns input.
    Pane(PaneId),
    /// No overlay is open and the active window has no focused pane (empty
    /// window, or the focused tile is not a pane). Nothing owns input.
    Nobody,
}

/// The overlay surfaces that can own input. `Layer` covers every modal on
/// the focus stack; the sidebar's inline context-rename editor is host state
/// (`renaming_window` + visible sidebar) that never pushes a stack layer, so
/// it gets its own variant instead of a sixth ad-hoc convention.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum OverlaySurface {
    /// The top of the modal focus stack.
    Layer(FocusKind),
    /// Inline context rename in the visible sidebar.
    SidebarRename,
}

impl PlexiApp {
    /// The overlay surface that owns input, if any: the top of the modal
    /// focus stack, else the inline sidebar rename editor. This is the single
    /// derivation behind [`Self::input_captured_by_overlay`] and
    /// [`Self::input_owner`] — never re-derive it elsewhere.
    pub(crate) fn overlay_surface(&self) -> Option<OverlaySurface> {
        if let Some(kind) = self.focus_stack.last() {
            return Some(OverlaySurface::Layer(*kind));
        }
        if self.sidebar_visible && self.renaming_window.is_some() {
            return Some(OverlaySurface::SidebarRename);
        }
        None
    }

    /// Derive this frame's input owner from host state. Precedence:
    /// OS window focus > overlay surfaces > the focused pane.
    ///
    /// The OS-focus gate applies to key *routing* (`dispatch_app_key_events`,
    /// the terminal steal): while another window owns the keyboard, no pane
    /// receives keys. It does not apply to egui-focus *projection* — see
    /// [`Self::host_input_owner`].
    pub(crate) fn input_owner(&self, ctx: &egui::Context) -> InputOwner {
        if !ctx.input(|i| i.viewport().focused.unwrap_or(true)) {
            return InputOwner::OsUnfocused;
        }
        self.host_input_owner()
    }

    /// The owner derived from host state alone, ignoring OS window focus.
    /// This is what the reconciler projects onto egui focus: blurring the
    /// window does not un-own a surface — egui receives no OS keyboard while
    /// blurred, and synthetic input injected by `plexi pane send`, drive-host,
    /// and the harness (which always arrives with the window blurred, since
    /// the CLI runs in another window) must still land in the owner's surface.
    pub(crate) fn host_input_owner(&self) -> InputOwner {
        if let Some(surface) = self.overlay_surface() {
            return InputOwner::Overlay(surface);
        }
        let win = &self.windows[self.active_window];
        if let Some(focused_tile) = win.focused_pane {
            // `focused_pane` can transiently point at a container after
            // egui_tiles simplification re-tabs a tree; resolve to the visible
            // pane inside it, the same healing `reconcile_stale_tiles` applies
            // to removed tiles.
            let pane_tile = match win.tree.tiles.get(focused_tile) {
                Some(egui_tiles::Tile::Pane(_)) => Some(focused_tile),
                Some(egui_tiles::Tile::Container(_)) => win.find_first_pane_in(focused_tile),
                None => None,
            };
            if let Some(pane_tile) = pane_tile {
                if let Some(egui_tiles::Tile::Pane(pane_id)) = win.tree.tiles.get(pane_tile) {
                    return InputOwner::Pane(*pane_id);
                }
            }
        }
        InputOwner::Nobody
    }

    /// The pane that owns input this frame, if the owner is a pane. Feeds the
    /// tiling behavior's `is_focused` so pane visuals track real ownership.
    pub(crate) fn owner_pane(&self, ctx: &egui::Context) -> Option<PaneId> {
        match self.input_owner(ctx) {
            InputOwner::Pane(pane_id) => Some(pane_id),
            _ => None,
        }
    }

    /// Project the frame's input owner onto egui widget focus. Called exactly
    /// once, at the end of `update()`, after every overlay and pane has
    /// rendered and registered its text surfaces.
    ///
    /// Rules:
    /// - A focused surface registered (this frame or last) under a key other
    ///   than the owner's surrenders focus — stale focus cannot survive.
    /// - The owner's last claim this frame is granted.
    /// - Without a claim, the owner keeps whichever of its own surfaces holds
    ///   focus, else its default surface is granted.
    /// - A focused id that no registration knows about is left alone (egui's
    ///   native click-to-focus on transient widgets).
    /// - OS window focus is ignored: projection follows
    ///   [`Self::host_input_owner`], so a blurred window keeps its owner's
    ///   surface focused and CLI-injected text still lands.
    #[allow(clippy::disallowed_methods)]
    pub(crate) fn reconcile_egui_focus(&mut self, ctx: &egui::Context) {
        let owner = self.host_input_owner();
        let registry = crate::ui::focus::drain_text_surfaces(ctx);
        let previous = take_previous_surfaces(ctx);

        let owner_key = match owner {
            InputOwner::Overlay(surface) => Some(SurfaceKey::Overlay(surface)),
            InputOwner::Pane(pane_id) => Some(SurfaceKey::Pane(pane_id)),
            InputOwner::OsUnfocused | InputOwner::Nobody => None,
        };

        // Surrender a focused surface that does not belong to the owner.
        // Last frame's registrations cover surfaces whose overlay/pane just
        // closed and therefore did not render (and register) this frame.
        if let Some(focused_id) = ctx.memory(|m| m.focused()) {
            let owned = owner_key.is_some_and(|key| registry.is_surface_of(key, focused_id));
            let known = registry.contains(focused_id)
                || previous.iter().any(|(_, id, _)| *id == focused_id);
            if known && !owned {
                log::info!(
                    "input_owner: surrendering egui focus {focused_id:?} — owner is {owner:?}"
                );
                ctx.memory_mut(|m| m.surrender_focus(focused_id));
            }
        }

        // Grant the owner its surface: explicit claim, else keep a focused
        // owner surface, else the owner's default.
        if let Some(key) = owner_key {
            let current = ctx.memory(|m| m.focused());
            let target = registry.claim_for(key).or_else(|| {
                if current.is_some_and(|id| registry.is_surface_of(key, id)) {
                    current
                } else {
                    registry.default_for(key)
                }
            });
            if let Some(target) = target {
                if current != Some(target) {
                    log::debug!("input_owner: granting egui focus {target:?} to {owner:?}");
                    ctx.memory_mut(|m| m.request_focus(target));
                }
            }
        }

        store_previous_surfaces(ctx, registry);
    }
}

const PREVIOUS_SURFACES_ID: &str = "plexi_text_surfaces_previous_frame";

fn take_previous_surfaces(ctx: &egui::Context) -> Vec<(SurfaceKey, Id, bool)> {
    let id = Id::new(PREVIOUS_SURFACES_ID);
    ctx.memory_mut(|memory| {
        memory
            .data
            .get_temp::<SurfaceRegistry>(id)
            .map(|registry| registry.surfaces)
            .unwrap_or_default()
    })
}

fn store_previous_surfaces(ctx: &egui::Context, registry: SurfaceRegistry) {
    let id = Id::new(PREVIOUS_SURFACES_ID);
    ctx.memory_mut(|memory| memory.data.insert_temp(id, registry));
}

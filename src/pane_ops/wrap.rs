//! Layout-preserving pane grouping into child contexts.

use std::collections::{HashMap, HashSet};

use egui_tiles::{Tile, TileId, Tiles, Tree};

use crate::app::PlexiApp;
use crate::host::context::{Context, ContextName, Window};
use crate::host::pane::{Pane, PortalPane};

/// Prune leaves without rebuilding containers, retaining their split shares and IDs.
fn prune_tiles(tiles: &mut Tiles<u64>, tile: TileId, keep: &HashSet<u64>) -> bool {
    let Some(value) = tiles.get(tile) else {
        return false;
    };
    match value {
        Tile::Pane(pane) if keep.contains(pane) => return true,
        Tile::Pane(_) => {}
        Tile::Container(container) => {
            let children = container.children_vec();
            let surviving: HashSet<_> = children
                .into_iter()
                .filter(|child| prune_tiles(tiles, *child, keep))
                .collect();
            if let Some(Tile::Container(container)) = tiles.get_mut(tile) {
                container.retain(|child| surviving.contains(&child));
                if let egui_tiles::Container::Tabs(tabs) = container {
                    if !tabs
                        .active
                        .is_some_and(|active| surviving.contains(&active))
                    {
                        tabs.active = tabs.children.first().copied();
                    }
                }
            }
            if !surviving.is_empty() {
                return true;
            }
        }
    }
    tiles.remove(tile);
    false
}

impl PlexiApp {
    fn reject_pane_wrap(&mut self, message: &str) -> bool {
        log::info!("wrap_panes: rejected: {message}");
        self.attention_queue.transient_hint = Some((message.into(), std::time::Instant::now()));
        self.ctx.request_repaint();
        false
    }

    /// Move existing panes into a child, preserving their pruned split topology.
    /// Validate the entire request before changing IDs, ownership, or focus.
    pub(crate) fn wrap_panes_to_subcontext(
        &mut self,
        pane_ids: &[u64],
        anchor: u64,
        name: Option<String>,
    ) -> bool {
        let selected: HashSet<_> = pane_ids.iter().copied().collect();
        if selected.is_empty() || !selected.contains(&anchor) {
            return self.reject_pane_wrap("Select panes and an anchor before wrapping.");
        }
        let Some(parent_idx) = self
            .windows
            .iter()
            .position(|w| w.panes.contains_key(&anchor))
        else {
            return self.reject_pane_wrap("The selected pane is no longer available.");
        };
        let parent = &self.windows[parent_idx];
        if selected
            .iter()
            .any(|id| !parent.panes.contains_key(id) || parent.tree.tiles.find_pane(id).is_none())
        {
            return self.reject_pane_wrap("Select panes from the same window.");
        }
        if selected
            .iter()
            .any(|id| parent.panes[id].as_portal().is_some())
        {
            return self.reject_pane_wrap("A portal cannot be wrapped in a subcontext.");
        }
        let parent_context = parent.context_id;
        let context_panes: usize = self
            .windows
            .iter()
            .filter(|w| w.context_id == parent_context)
            .map(|w| w.panes.len())
            .sum();
        if selected.len() >= context_panes {
            return self.reject_pane_wrap("Leave at least one pane outside the subcontext.");
        }
        let Some(context) = self.router.iter().find(|c| c.context_id == parent_context) else {
            return self.reject_pane_wrap("The parent context is no longer available.");
        };
        let root = context.root.clone();
        let depth = context.depth + 1;
        let name = name
            .or_else(|| {
                parent.panes[&anchor]
                    .as_terminal()
                    .and_then(|t| t.name.clone())
            })
            .or_else(|| parent.panes[&anchor].as_app().map(|a| a.name.clone()))
            .unwrap_or_else(|| format!("Sub-context {}", self.router.len() + 1));
        let Some(anchor_tile) = parent.tree.tiles.find_pane(&anchor) else {
            return self.reject_pane_wrap("The anchor pane is no longer available.");
        };
        let Some(child_root) = parent.tree.root else {
            return self.reject_pane_wrap("The selected window has no layout.");
        };
        let mut child_tiles = parent.tree.tiles.clone();
        prune_tiles(&mut child_tiles, child_root, &selected);
        let context_id = self.next_window_id;
        let window_id = context_id + 1;
        self.next_window_id += 2;
        let portal_id = self.host.alloc_pane_id();
        let mut parent_strip = self.strip_for_context(parent_context);
        let mut child_strip = parent_strip.clone().extract_panes(&selected, context_id);
        child_strip.focus_pane(anchor);
        for column in &mut parent_strip.columns {
            for pane in &mut column.panes {
                if *pane == anchor {
                    *pane = portal_id;
                }
            }
            if column.last_focused == Some(anchor) {
                column.last_focused = Some(portal_id);
            }
        }
        if parent_strip.focused_pane == Some(anchor) {
            parent_strip.focused_pane = Some(portal_id);
        }
        for id in &selected {
            if *id != anchor {
                parent_strip.remove_pane(*id);
            }
        }
        parent_strip.focus_pane(portal_id);
        let parent = &mut self.windows[parent_idx];
        let mut child_panes = HashMap::new();
        for id in &selected {
            if let Some(pane) = parent.panes.remove(id) {
                child_panes.insert(*id, pane);
            }
        }
        if let Some(Tile::Pane(slot)) = parent.tree.tiles.get_mut(anchor_tile) {
            *slot = portal_id;
        }
        parent.panes.insert(
            portal_id,
            Pane::Portal(Box::new(PortalPane {
                pane_id: portal_id,
                target_context_id: context_id,
                context_state: None,
                hidden: false,
            })),
        );
        let keep = parent.panes.keys().copied().collect();
        if let Some(root) = parent.tree.root {
            prune_tiles(&mut parent.tree.tiles, root, &keep);
        }
        parent.clear_zoom();
        parent.focused_pane = Some(anchor_tile);
        let parent_window = parent.window_id;
        self.host.clear_pane_selection(parent_window);
        self.host.insert_strip(parent_context, parent_strip);
        self.host.insert_strip(context_id, child_strip);
        super::workspace::ensure_context_state_ignore(&root);
        self.router.insert_after_subtree(
            parent_context,
            Context {
                name: ContextName::custom(name),
                root: root.clone(),
                description: None,
                context_id,
                parent_id: Some(parent_context),
                depth,
                parked: false,
            },
        );
        self.windows.push(Window {
            name: String::new(),
            path: root,
            tree: Tree::new(format!("child_tree_{window_id}"), child_root, child_tiles),
            panes: child_panes,
            focused_pane: Some(anchor_tile),
            zoomed_pane: None,
            grid_x: 0,
            grid_y: 0,
            window_id,
            context_id,
        });
        self.context_active_window.insert(context_id, window_id);
        self.router
            .push_depth(parent_context, parent_window, Some(anchor_tile));
        if let Some(index) = self.router.position(|c| c.context_id == context_id) {
            self.switch_workspace(index);
        }
        log::info!("wrap_panes: panes={pane_ids:?} anchor={anchor} parent={parent_context} child={context_id} portal={portal_id}");
        self.mark_workspace_dirty();
        true
    }

    pub(crate) fn extend_pane_selection(&mut self, direction: crate::host::keys::Direction) {
        let window = &self.windows[self.active_window];
        let Some(focused) = window.focused_pane else {
            return;
        };
        let Some(Tile::Pane(start)) = window.tree.tiles.get(focused) else {
            return;
        };
        let start = *start;
        let target = window
            .find_pane_in_direction_from(focused, direction)
            .unwrap_or(focused);
        let Some(Tile::Pane(next)) = window.tree.tiles.get(target) else {
            return;
        };
        let next = *next;
        self.host
            .extend_pane_selection(window.window_id, start, next);
        self.windows[self.active_window].navigate_to(target);
        self.ctx.request_repaint();
    }

    pub(crate) fn wrap_selection_or_zoom_in(&mut self) {
        let window_id = self.windows[self.active_window].window_id;
        if let Some(selection) = self
            .host
            .clear_pane_selection(window_id)
            .filter(|s| s.panes.len() >= 2)
        {
            self.wrap_panes_to_subcontext(&selection.panes, selection.anchor, None);
        } else {
            self.zoom_in_level();
        }
    }
}

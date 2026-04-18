use crate::keys::Direction;
use crate::pane::{Pane, TerminalPane};
use crate::shell;
use crate::tiling::PaneId;
use egui_tiles::{Container, Tile, TileId, Tree};
use std::collections::HashMap;
use std::path::PathBuf;

pub(crate) enum ContextMenuAction {
    Rename,
    MoveToTop,
    MoveUp,
    MoveDown,
    Delete,
}

pub struct Context {
    pub name: String,
    pub path: PathBuf,
    pub tree: Tree<PaneId>,
    pub panes: HashMap<PaneId, Pane>,
    pub focused_pane: Option<TileId>,
    pub zoomed_pane: Option<TileId>,
}

impl Context {
    pub(crate) fn find_ancestor_tabs(&self, tile_id: TileId) -> Option<(TileId, TileId)> {
        let mut current = tile_id;
        loop {
            let parent_id = self.tree.tiles.parent_of(current)?;
            if matches!(
                self.tree.tiles.get(parent_id),
                Some(Tile::Container(Container::Tabs(_)))
            ) {
                return Some((parent_id, current));
            }
            current = parent_id;
        }
    }

    pub(crate) fn activate_tab_for(&mut self, tile_id: TileId) {
        let result = self.find_ancestor_tabs(tile_id);
        if let Some((tabs_id, child_tile)) = result {
            if let Some(Tile::Container(Container::Tabs(tabs))) = self.tree.tiles.get_mut(tabs_id) {
                tabs.set_active(child_tile);
            }
        }
    }

    pub(crate) fn find_logical_parent(&self, tile_id: TileId) -> Option<(TileId, TileId)> {
        let mut current = tile_id;
        loop {
            let parent_id = self.tree.tiles.parent_of(current)?;
            if let Some(Tile::Container(container)) = self.tree.tiles.get(parent_id) {
                if container.children().count() > 1 {
                    return Some((parent_id, current));
                }
            }
            current = parent_id;
        }
    }

    pub(crate) fn find_first_pane_in(&self, tile_id: TileId) -> Option<TileId> {
        match self.tree.tiles.get(tile_id)? {
            Tile::Pane(_) => Some(tile_id),
            Tile::Container(container) => {
                if let Container::Tabs(tabs) = container {
                    // Only follow the active tab — others are invisible
                    return self.find_first_pane_in(tabs.active?);
                }
                for &child in container.children() {
                    if let Some(pane) = self.find_first_pane_in(child) {
                        return Some(pane);
                    }
                }
                None
            }
        }
    }

    pub(crate) fn find_next_focus(&self, excluding: TileId) -> Option<TileId> {
        for dir in [
            Direction::Left,
            Direction::Up,
            Direction::Right,
            Direction::Down,
        ] {
            if let Some(target) = self.find_pane_in_direction_from(excluding, dir) {
                return Some(target);
            }
        }
        self.tree
            .active_tiles()
            .into_iter()
            .find(|&id| id != excluding && matches!(self.tree.tiles.get(id), Some(Tile::Pane(_))))
    }

    pub(crate) fn find_pane_in_direction_from(
        &self,
        from: TileId,
        dir: Direction,
    ) -> Option<TileId> {
        let from_rect = self.tree.tiles.rect(from)?;
        let center = from_rect.center();

        let mut best: Option<(TileId, (u8, f32))> = None;
        let is_horizontal = matches!(dir, Direction::Left | Direction::Right);

        for tile_id in self.tree.active_tiles() {
            if tile_id == from {
                continue;
            }
            if !matches!(self.tree.tiles.get(tile_id), Some(Tile::Pane(_))) {
                continue;
            }
            let Some(rect) = self.tree.tiles.rect(tile_id) else {
                continue;
            };
            let other = rect.center();

            let valid = match dir {
                Direction::Left => other.x < center.x,
                Direction::Right => other.x > center.x,
                Direction::Up => other.y < center.y,
                Direction::Down => other.y > center.y,
            };

            if valid {
                let (has_overlap, primary_dist) = if is_horizontal {
                    let overlap =
                        from_rect.top() < rect.bottom() && rect.top() < from_rect.bottom();
                    (overlap, (other.x - center.x).abs())
                } else {
                    let overlap =
                        from_rect.left() < rect.right() && rect.left() < from_rect.right();
                    (overlap, (other.y - center.y).abs())
                };

                let tier = if has_overlap { 0 } else { 1 };
                let score = (tier, primary_dist);
                if best.is_none_or(|(_, s)| score < s) {
                    best = Some((tile_id, score));
                }
            }
        }

        best.map(|(id, _)| id)
    }

    pub(crate) fn compute_tab_info(&self) -> HashMap<TileId, (usize, usize)> {
        let mut info = HashMap::new();
        for (_tile_id, tile) in self.tree.tiles.iter() {
            if let Tile::Container(Container::Tabs(tabs)) = tile {
                let children = &tabs.children;
                if children.len() < 2 {
                    continue;
                }
                let count = children.len();
                let active_idx = tabs
                    .active
                    .and_then(|a| children.iter().position(|&c| c == a))
                    .unwrap_or(0);
                for child in children {
                    self.collect_panes(*child, &mut |pane_tile| {
                        info.insert(pane_tile, (active_idx, count));
                    });
                }
            }
        }
        info
    }

    fn collect_panes(&self, tile_id: TileId, f: &mut dyn FnMut(TileId)) {
        match self.tree.tiles.get(tile_id) {
            Some(Tile::Pane(_)) => f(tile_id),
            Some(Tile::Container(container)) => {
                for &child in container.children() {
                    self.collect_panes(child, f);
                }
            }
            None => {}
        }
    }

    pub(crate) fn get_focused_pane_cwd(&self, tile_id: TileId) -> Option<PathBuf> {
        let pane_id = match self.tree.tiles.get(tile_id)? {
            Tile::Pane(id) => *id,
            _ => return None,
        };
        let pane = self.panes.get(&pane_id)?;
        if let Some(terminal) = pane.as_terminal() {
            shell::get_pid_cwd(terminal.backend.child_pid())
        } else {
            pane.as_app().map(|app| app.workspace_root.clone())
        }
    }

    /// Returns (pane_id, &mut TerminalPane) for the currently focused pane, if any.
    pub(crate) fn focused_pane_mut(&mut self) -> Option<(PaneId, &mut TerminalPane)> {
        let tile_id = self.focused_pane?;
        let pane_id = match self.tree.tiles.get(tile_id)? {
            Tile::Pane(id) => *id,
            _ => return None,
        };
        let pane = self.panes.get_mut(&pane_id)?;
        let terminal = pane.as_terminal_mut()?;
        Some((pane_id, terminal))
    }
}

pub(crate) fn replace_child(container: &mut Container, old: TileId, new: TileId) {
    match container {
        Container::Linear(linear) => {
            if let Some(pos) = linear.children.iter().position(|&c| c == old) {
                linear.children[pos] = new;
            }
        }
        Container::Tabs(tabs) => {
            if let Some(pos) = tabs.children.iter().position(|&c| c == old) {
                tabs.children[pos] = new;
            }
        }
        Container::Grid(_) => {
            container.remove_child(old);
            container.add_child(new);
        }
    }
}

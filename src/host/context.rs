use crate::host::keys::Direction;
use crate::host::pane::Pane;
use crate::host::shell;
use crate::spatial::tiling::PaneId;
use egui_tiles::{Container, Tile, TileId, Tree};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

pub(crate) enum WindowMenuAction {
    Rename,
    EditDescription,
    MoveToTop,
    MoveUp,
    MoveDown,
    MoveToBottom,
    Delete,
    SetRoot(PathBuf),
    ClearRoot,
    /// Open the text-input overlay to set the root interactively.
    OpenRootOverlay,
}

/// A context is a sidebar item — a project or directory scope.
/// It does not own panes directly; pane layouts live in `Window` pages
/// that reference this context via `context_id`.
///
/// This is the canonical context type used everywhere: live GUI state,
/// persistence (workspace save/restore), and test harness. Replaces the
/// former `SavedContext` and unifies with `HostContext`.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Context {
    pub name: String,
    pub path: PathBuf,
    /// Optional project root. When set, new panes in this context open at
    /// this directory rather than inheriting the focused pane's CWD.
    #[serde(default)]
    pub root: Option<PathBuf>,
    /// Optional description — ambient intent for what you're doing in this context.
    #[serde(default)]
    pub description: Option<String>,
    /// Stable unique ID, assigned at creation and never reused.
    pub context_id: u64,
    /// Parent context_id for sub-contexts. None = top-level.
    #[serde(default)]
    pub parent_id: Option<u64>,
    /// Nesting depth. 0 = root level. Capped at 3.
    /// Kept for backward-compatible deserialization but should be derived
    /// from `parent_id` chain at runtime.
    #[serde(default)]
    pub depth: u32,
    /// When true, this context is parked: hidden from the active sidebar list,
    /// collapsed into a "Parked (N)" divider at the bottom. All panes stay alive.
    #[serde(default)]
    pub parked: bool,
}

/// A window is a single spatial grid page — a pane layout with focus and zoom state.
/// Every window belongs to a context (`context_id`).
pub struct Window {
    pub name: String,
    pub path: PathBuf,
    pub tree: Tree<PaneId>,
    pub panes: HashMap<PaneId, Pane>,
    pub focused_pane: Option<TileId>,
    pub zoomed_pane: Option<TileId>,
    /// Spatial grid coordinates within the context's window grid.
    /// `(0, 0)` is the origin (top-left). `grid_x` grows rightward,
    /// `grid_y` grows downward.
    pub grid_x: u32,
    pub grid_y: u32,
    /// Stable unique ID for this window, assigned at creation.
    pub window_id: u64,
    /// The context this window belongs to.
    pub context_id: u64,
}

impl Window {
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
            Direction::Right,
            Direction::Down,
            Direction::Left,
            Direction::Up,
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

        let candidates = self.tree.active_tiles().into_iter().filter_map(|tile_id| {
            if tile_id == from {
                return None;
            }
            if !matches!(self.tree.tiles.get(tile_id), Some(Tile::Pane(_))) {
                return None;
            }
            let rect = self.tree.tiles.rect(tile_id)?;
            Some((tile_id, rect))
        });

        find_in_direction_geometric(from_rect, candidates, dir)
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
}

/// Pure-geometry direction-finder. Picks the geometrically-nearest neighbor
/// of `from_rect` in the requested direction.
///
/// Two-tier scoring (matches the v3.2 lateral-focus spec):
///
/// 1. **Overlap tier**: candidates whose bounding rect overlaps the source's
///    perpendicular axis range (top-to-bottom for L/R, left-to-right for U/D)
///    are preferred over non-overlapping ones.
/// 2. **Distance**: among the preferred tier, pick the candidate with the
///    smallest distance from the source's center to the candidate's nearest
///    edge along the requested axis.
/// 3. **Tie-break**: smallest perpendicular-axis center distance.
///
/// Returns `None` if no candidate lies in the requested direction (no
/// wrap-around).
pub(crate) fn find_in_direction_geometric<I, T>(
    from_rect: egui::Rect,
    candidates: I,
    dir: Direction,
) -> Option<T>
where
    I: IntoIterator<Item = (T, egui::Rect)>,
    T: Copy,
{
    let center = from_rect.center();
    let is_horizontal = matches!(dir, Direction::Left | Direction::Right);

    let mut best: Option<(T, (u8, f32, f32))> = None;

    for (id, rect) in candidates {
        // "In the requested direction" — strict half-plane test using nearest
        // edge so a candidate exactly adjacent to `from_rect` still counts.
        let in_direction = match dir {
            Direction::Left => rect.right() <= center.x,
            Direction::Right => rect.left() >= center.x,
            Direction::Up => rect.bottom() <= center.y,
            Direction::Down => rect.top() >= center.y,
        };
        if !in_direction {
            continue;
        }

        let (has_overlap, primary_dist, secondary_dist) = if is_horizontal {
            let overlap =
                from_rect.top() < rect.bottom() && rect.top() < from_rect.bottom();
            // Distance from source center to candidate's nearest vertical edge.
            let nearest_edge_x = match dir {
                Direction::Left => rect.right(),
                Direction::Right => rect.left(),
                _ => unreachable!(),
            };
            let primary = (nearest_edge_x - center.x).abs();
            let secondary = (rect.center().y - center.y).abs();
            (overlap, primary, secondary)
        } else {
            let overlap =
                from_rect.left() < rect.right() && rect.left() < from_rect.right();
            let nearest_edge_y = match dir {
                Direction::Up => rect.bottom(),
                Direction::Down => rect.top(),
                _ => unreachable!(),
            };
            let primary = (nearest_edge_y - center.y).abs();
            let secondary = (rect.center().x - center.x).abs();
            (overlap, primary, secondary)
        };

        let tier = if has_overlap { 0 } else { 1 };
        let score = (tier, primary_dist, secondary_dist);
        if best.is_none_or(|(_, s)| score_lt(score, s)) {
            best = Some((id, score));
        }
    }

    best.map(|(id, _)| id)
}

/// Lexicographic comparison on `(tier, primary, secondary)` — `f32` doesn't
/// implement `Ord`, so wrap with explicit total-order checks.
fn score_lt(a: (u8, f32, f32), b: (u8, f32, f32)) -> bool {
    if a.0 != b.0 {
        return a.0 < b.0;
    }
    if a.1 != b.1 {
        return a.1 < b.1;
    }
    a.2 < b.2
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

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Pos2, Rect};

    /// 6-pane 2-row × 3-col grid:
    ///     ┌───┬───┬───┐
    ///     │ 1 │ 2 │ 3 │
    ///     ├───┼───┼───┤
    ///     │ 4 │ 5 │ 6 │
    ///     └───┴───┴───┘
    /// Coordinates: each cell 100×100, top-left of grid at (0, 0).
    fn grid_2x3() -> [(usize, Rect); 6] {
        let cell = |col: f32, row: f32| {
            Rect::from_min_max(
                Pos2::new(col * 100.0, row * 100.0),
                Pos2::new((col + 1.0) * 100.0, (row + 1.0) * 100.0),
            )
        };
        [
            (1, cell(0.0, 0.0)),
            (2, cell(1.0, 0.0)),
            (3, cell(2.0, 0.0)),
            (4, cell(0.0, 1.0)),
            (5, cell(1.0, 1.0)),
            (6, cell(2.0, 1.0)),
        ]
    }

    #[test]
    fn lateral_focus_picks_geometrically_correct_neighbor() {
        let panes = grid_2x3();
        // Source = pane 5 (center of the grid).
        let from = panes[4].1; // pane 5

        let candidates = |exclude: usize| {
            panes
                .iter()
                .copied()
                .filter(move |(id, _)| *id != exclude)
                .collect::<Vec<_>>()
        };

        let go = |dir: Direction| {
            find_in_direction_geometric(from, candidates(5).into_iter(), dir)
        };

        assert_eq!(go(Direction::Left), Some(4), "left of 5 should be 4");
        assert_eq!(go(Direction::Right), Some(6), "right of 5 should be 6");
        assert_eq!(go(Direction::Up), Some(2), "above 5 should be 2");
        assert_eq!(
            go(Direction::Down),
            None,
            "5 is in the bottom row → no neighbor below"
        );
    }

    #[test]
    fn lateral_focus_no_op_at_edge() {
        let panes = grid_2x3();
        // Source = pane 1 (top-left corner).
        let from = panes[0].1;
        let candidates: Vec<_> = panes
            .iter()
            .copied()
            .filter(|(id, _)| *id != 1)
            .collect();

        // No pane to the left of 1, no pane above 1.
        assert_eq!(
            find_in_direction_geometric(from, candidates.iter().copied(), Direction::Left),
            None
        );
        assert_eq!(
            find_in_direction_geometric(from, candidates.iter().copied(), Direction::Up),
            None
        );
        // But 2 is to the right and 4 is below.
        assert_eq!(
            find_in_direction_geometric(from, candidates.iter().copied(), Direction::Right),
            Some(2)
        );
        assert_eq!(
            find_in_direction_geometric(from, candidates.iter().copied(), Direction::Down),
            Some(4)
        );
    }

    #[test]
    fn lateral_focus_prefers_overlapping_perpendicular_range() {
        // Source pane on the left, two candidates to the right:
        //   A: same vertical band as source (overlap → tier 0)
        //   B: strictly above source (no overlap → tier 1), even though
        //      its left edge is closer.
        let from = Rect::from_min_max(Pos2::new(0.0, 100.0), Pos2::new(100.0, 200.0));
        let a = (
            "overlap",
            Rect::from_min_max(Pos2::new(300.0, 120.0), Pos2::new(400.0, 180.0)),
        );
        let b = (
            "above_no_overlap",
            Rect::from_min_max(Pos2::new(150.0, 0.0), Pos2::new(250.0, 50.0)),
        );

        // Despite `b` having a smaller x-distance, `a` wins because it overlaps
        // the source's vertical range (tier 0 beats tier 1).
        assert_eq!(
            find_in_direction_geometric(
                from,
                vec![a, b].into_iter(),
                Direction::Right
            ),
            Some("overlap")
        );
    }

    #[test]
    fn lateral_focus_picks_nearest_edge_among_overlap_tier() {
        // Source on the left; two overlapping candidates on the right at
        // different x-distances — pick the closer one.
        let from = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(100.0, 100.0));
        let near = (
            "near",
            Rect::from_min_max(Pos2::new(150.0, 0.0), Pos2::new(250.0, 100.0)),
        );
        let far = (
            "far",
            Rect::from_min_max(Pos2::new(400.0, 0.0), Pos2::new(500.0, 100.0)),
        );

        assert_eq!(
            find_in_direction_geometric(
                from,
                vec![near, far].into_iter(),
                Direction::Right
            ),
            Some("near")
        );
    }
}

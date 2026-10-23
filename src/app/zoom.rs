//! Zoom Ladder, Niri-Style Scrolling Column Strip, Sibling Context Cards, and Tree Atlas
//! (`SPEC.md` R31–R36, R40, R42).

use crate::app::app_trait::KeyDisposition;
use crate::app::attention_queue::QueueCategory;
use crate::app::input_router::PlexiInput;
use crate::app::PlexiApp;
use crate::host::keys::Direction;
use crate::spatial::tiling::PaneId;
use crate::ui::style;
use egui::{
    Align2, CornerRadius, FontFamily, FontId, Key, Pos2, Rect, RichText, Sense, Stroke, Vec2,
};
use egui_tiles::{Container, LinearDir, Tile, TileId};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// The three zoom levels on the ladder (`SPEC.md` § Levels).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum ZoomLevel {
    /// Default level inside a context: Niri-style scrolling column strip.
    /// The ONLY level where a pane takes keyboard input.
    #[default]
    Columns,
    /// Current context plus its siblings (same `parent_id`), in creation order.
    SiblingCards,
    /// Whole-workspace context graph: left-to-right tidy tree with Motorways road edges.
    TreeAtlas,
}

impl ZoomLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Columns => "columns",
            Self::SiblingCards => "sibling_cards",
            Self::TreeAtlas => "tree_atlas",
        }
    }
}

/// Fixed column width presets (`35%` parent/list, `65%` child/main).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum ColumnWidthPreset {
    Parent35,
    #[default]
    Child65,
}

impl ColumnWidthPreset {
    pub fn ratio(self) -> f32 {
        match self {
            Self::Parent35 => style::COLUMN_WIDTH_PARENT,
            Self::Child65 => style::COLUMN_WIDTH_CHILD,
        }
    }
}

/// One column in a context's horizontal scrolling strip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StripColumn {
    pub index: usize,
    pub context_id: u64,
    pub panes: Vec<PaneId>,
    pub preset: ColumnWidthPreset,
}

/// Ghost marker recorded when zooming out from `ZoomLevel::Columns` ("you were here").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GhostMarker {
    pub context_id: u64,
    pub parent_context_id: Option<u64>,
    pub column_idx: usize,
    pub pane_id: PaneId,
    pub strip_offset: usize,
}

/// A positioned context node in the Tree Atlas.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct AtlasNode {
    pub context_id: u64,
    pub parent_id: Option<u64>,
    pub name: String,
    pub depth: usize,
    /// Layout order (preorder), stable for navigation.
    pub order: usize,
    pub rect: Rect,
    pub ask_rollup: usize,
    pub direct_asks: usize,
    pub failed_count: usize,
    pub pane_count: usize,
    pub is_ghost: bool,
    pub preview_lines: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AtlasEdgeKind {
    TreeRoad,
    NeedsRollup,
    Failed,
    PipeLink,
    /// An agent pane in the target context was spawned by a pane in the source context.
    Spawn,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct AtlasEdge {
    pub from_context_id: u64,
    pub to_context_id: u64,
    pub from_pos: Pos2,
    pub to_pos: Pos2,
    pub kind: AtlasEdgeKind,
}

/// Live tile content tier determined by rendered tile size with 8px hysteresis (`SPEC.md` Q3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TileContentTier {
    /// >= 252x88: header + last 5 lines
    Lines5,
    /// >= 190x60: header + last 3 lines
    Lines3,
    /// >= 150x32: header + 1 urgent/status line + glyph (floor for asking/failed nodes)
    Line1,
    /// >= 96x22: glyph + name
    GlyphAndName,
    /// < 96x22: glyph only
    GlyphOnly,
}

impl TileContentTier {
    pub fn from_size(size: Vec2, is_asking_or_failed: bool, prev: Option<Self>) -> Self {
        let h = if prev.is_some() { 4.0 } else { 0.0 };
        let tier = if size.x >= 252.0 - h && size.y >= 88.0 - h {
            Self::Lines5
        } else if size.x >= 190.0 - h && size.y >= 60.0 - h {
            Self::Lines3
        } else if size.x >= 150.0 - h && size.y >= 32.0 - h {
            Self::Line1
        } else if size.x >= 96.0 - h && size.y >= 22.0 - h {
            Self::GlyphAndName
        } else {
            Self::GlyphOnly
        };
        if is_asking_or_failed && matches!(tier, Self::GlyphAndName | Self::GlyphOnly) {
            Self::Line1
        } else {
            tier
        }
    }
}

/// Complete zoom, column strip, sibling cards, and tree atlas state on `PlexiApp`.
#[derive(Debug, Default)]
#[allow(dead_code)]
pub struct ZoomState {
    pub level: ZoomLevel,
    /// Recorded origin when zooming out from `Columns`.
    pub ghost: Option<GhostMarker>,
    /// Per-context horizontal scroll offset (index of leftmost visible column in the 2-column viewport).
    pub strip_offsets: HashMap<u64, usize>,
    /// Per-(context_id, column_idx) last focused pane (`SPEC.md` § Focus memory).
    pub column_last_pane: HashMap<(u64, usize), PaneId>,
    /// Per-context last active pane (`SPEC.md` R36).
    pub context_last_pane: HashMap<u64, PaneId>,
    /// Optional explicit column groupings per context (`context_id -> Vec<Vec<PaneId>>`).
    pub explicit_columns: HashMap<u64, Vec<Vec<PaneId>>>,
    /// Optional column width preset overrides (`(context_id, col_idx) -> ColumnWidthPreset`).
    pub column_presets: HashMap<(u64, usize), ColumnWidthPreset>,
    /// Focused context ID while at `ZoomLevel::SiblingCards`.
    pub cards_focused_context: Option<u64>,
    /// Focused context ID while at `ZoomLevel::TreeAtlas`.
    pub atlas_focused_context: Option<u64>,
    /// Per-parent context last-visited child context in the atlas (`h` / `l` memory).
    pub atlas_last_child: HashMap<u64, u64>,
    /// True when the read-only Peek card (`p`) is open in the Tree Atlas.
    pub atlas_peek_open: bool,
    /// Directed context-to-context links (`(from_ctx, to_ctx)`) for `f`/`F` traversal in atlas.
    pub context_links: Vec<(u64, u64)>,
    /// Monotonic counter of edge bumps (observable by tests for R31/R32).
    pub edge_bump_count: u64,
    /// Last edge bump direction and timestamp for rendering the 60–90ms focus ring bump.
    pub last_edge_bump: Option<(Direction, Instant)>,
    /// Retargetable zoom transition state (`(from_progress, target_progress, started_at)`).
    pub transition: Option<(f32, f32, Instant)>,
    /// Buffered text/key events typed during a zoom-in flight to `Columns`.
    pub flight_key_buffer: Vec<egui::Event>,
}

impl ZoomState {
    pub fn strip_offset_for(&self, context_id: u64) -> usize {
        self.strip_offsets.get(&context_id).copied().unwrap_or(0)
    }

    /// Current visual zoom progress (`0.0` = Columns, `1.0` = SiblingCards, `2.0` = TreeAtlas).
    pub fn visual_level_progress(&self, reduce_motion: bool) -> f32 {
        let target = match self.level {
            ZoomLevel::Columns => 0.0,
            ZoomLevel::SiblingCards => 1.0,
            ZoomLevel::TreeAtlas => 2.0,
        };
        if reduce_motion || cfg!(test) {
            return target;
        }
        let Some((from, to, started_at)) = self.transition else {
            return target;
        };
        let elapsed_ms = started_at.elapsed().as_secs_f32() * 1000.0;
        let t = (elapsed_ms / (style::ZOOM_DURATION_MS as f32)).clamp(0.0, 1.0);
        let eased = style::ease_out_cubic(t);
        from + (to - from) * eased
    }

    /// Current pixel offset for an active 60–90ms edge bump.
    pub fn active_edge_bump_offset(&self) -> Vec2 {
        let Some((dir, started_at)) = self.last_edge_bump else {
            return Vec2::ZERO;
        };
        let elapsed = started_at.elapsed().as_millis() as u64;
        if elapsed >= style::EDGE_BUMP_DURATION_MS {
            return Vec2::ZERO;
        }
        let phase = (elapsed as f32) / (style::EDGE_BUMP_DURATION_MS as f32);
        // Triangle pulse: out then back within 75ms.
        let amp = if phase < 0.5 {
            phase * 2.0
        } else {
            (1.0 - phase) * 2.0
        } * style::EDGE_BUMP_OFFSET_PX;
        match dir {
            Direction::Left => Vec2::new(-amp, 0.0),
            Direction::Right => Vec2::new(amp, 0.0),
            Direction::Up => Vec2::new(0.0, -amp),
            Direction::Down => Vec2::new(0.0, amp),
        }
    }
}

/// Collect visible non-portal leaf `PaneId`s under `tile_id` in top-to-bottom / left-to-right order.
pub(crate) fn collect_leaf_panes_in_tile(
    tree: &egui_tiles::Tree<PaneId>,
    panes_map: &HashMap<PaneId, crate::host::pane::Pane>,
    tile_id: TileId,
    out: &mut Vec<PaneId>,
) {
    let Some(tile) = tree.tiles.get(tile_id) else {
        return;
    };
    match tile {
        Tile::Pane(pid) => {
            if let Some(p) = panes_map.get(pid) {
                if !p.is_hidden() && !matches!(p, crate::host::pane::Pane::Portal(_)) {
                    out.push(*pid);
                }
            }
            // A tile whose pane is absent from the live window is stale saved
            // layout state, not a pane: never enumerate it.
        }
        Tile::Container(Container::Tabs(tabs)) => {
            if let Some(active) = tabs.active {
                collect_leaf_panes_in_tile(tree, panes_map, active, out);
            } else if let Some(&first) = tabs.children.first() {
                collect_leaf_panes_in_tile(tree, panes_map, first, out);
            }
        }
        Tile::Container(container) => {
            for &child in container.children() {
                collect_leaf_panes_in_tile(tree, panes_map, child, out);
            }
        }
    }
}

impl PlexiApp {
    /// Record an edge bump in `dir` (increments `self.zoom.edge_bump_count` and triggers visual pulse).
    pub fn record_edge_bump(&mut self, dir: Direction) {
        self.zoom.edge_bump_count += 1;
        self.zoom.last_edge_bump = Some((dir, Instant::now()));
        log::info!(
            "zoom: edge_bump dir={dir:?} count={} level={}",
            self.zoom.edge_bump_count,
            self.zoom.level.as_str()
        );
    }

    /// Record the currently focused pane in both column memory and context memory.
    pub fn record_focused_pane_memory(&mut self) {
        let Some(pane_id) = self.active_focused_pane_id() else {
            return;
        };
        let ctx_id = self.router.active().context_id;
        self.zoom.context_last_pane.insert(ctx_id, pane_id);

        let cols = self.columns_for_context(ctx_id);
        if let Some(col) = cols.iter().find(|c| c.panes.contains(&pane_id)) {
            self.zoom
                .column_last_pane
                .insert((ctx_id, col.index), pane_id);
            self.ensure_column_visible(ctx_id, col.index, cols.len());
        }
    }

    /// Ensure `col_idx` is within the visible 2-column window (`strip_offset .. strip_offset + 2`).
    pub fn ensure_column_visible(&mut self, context_id: u64, col_idx: usize, total_cols: usize) {
        if total_cols <= 2 {
            self.zoom.strip_offsets.insert(context_id, 0);
            return;
        }
        let mut offset = self.zoom.strip_offset_for(context_id);
        if col_idx < offset {
            offset = col_idx;
        } else if col_idx >= offset + 2 {
            offset = col_idx.saturating_sub(1);
        }
        let max_offset = total_cols.saturating_sub(2);
        offset = offset.min(max_offset);
        self.zoom.strip_offsets.insert(context_id, offset);
    }

    /// Create a context and backing window without spawning a PTY (for fast, deterministic tests
    /// and headless graph construction), and switch to it while recording the outgoing context's
    /// last focused pane.
    #[allow(dead_code)]
    pub fn create_context_without_pty(&mut self, name: &str, parent_id: Option<u64>) -> u64 {
        if let Some(old_pane) = self.active_focused_pane_id() {
            let old_ctx = self.router.active().context_id;
            self.zoom.context_last_pane.insert(old_ctx, old_pane);
        }
        let ctx_id = self.next_window_id;
        self.next_window_id += 1;
        let win_id = self.next_window_id;
        self.next_window_id += 1;
        let depth = parent_id
            .and_then(|pid| {
                self.router
                    .as_slice()
                    .iter()
                    .find(|c| c.context_id == pid)
                    .map(|c| c.depth + 1)
            })
            .unwrap_or(0);
        let root = self.router.active().root.clone();
        self.windows.push(crate::host::context::Window {
            name: name.to_string(),
            path: root.clone(),
            tree: egui_tiles::Tree::empty("plexi"),
            panes: HashMap::new(),
            focused_pane: None,
            zoomed_pane: None,
            grid_x: 0,
            grid_y: 0,
            window_id: win_id,
            context_id: ctx_id,
        });
        let win_idx = self.windows.len() - 1;
        self.router.push(crate::host::context::Context {
            name: crate::host::context::ContextName::custom(name.to_string()),
            root,
            description: None,
            context_id: ctx_id,
            parent_id,
            depth,
            parked: false,
        });
        self.router.activate_last();
        self.active_window = win_idx;
        self.context_active_window.insert(ctx_id, win_id);
        ctx_id
    }

    /// Configure explicit columns for a context (`each inner Vec<PaneId>` is a vertical stack of panes).
    /// Also arranges the context's `window.tree` to match (`Horizontal` root of `Vertical` stacks).
    #[allow(dead_code)]
    pub fn set_context_columns(&mut self, context_id: u64, columns: Vec<Vec<PaneId>>) {
        self.zoom
            .explicit_columns
            .insert(context_id, columns.clone());
        if let Some(win_idx) = self.windows.iter().position(|w| w.context_id == context_id) {
            let target_pids: Vec<PaneId> = columns.iter().flatten().copied().collect();
            let mut moved_panes = Vec::new();
            for (idx, win) in self.windows.iter_mut().enumerate() {
                if idx == win_idx {
                    continue;
                }
                for &pid in &target_pids {
                    if let Some(pane) = win.panes.remove(&pid) {
                        moved_panes.push((pid, pane));
                    }
                    if let Some(tile_id) = win.tree.tiles.find_pane(&pid) {
                        win.tree.tiles.remove(tile_id);
                    }
                }
            }
            for (pid, pane) in moved_panes {
                self.windows[win_idx].panes.insert(pid, pane);
            }

            let mut tiles = egui_tiles::Tiles::default();
            let mut col_tiles = Vec::new();
            let mut first_pane_tile = None;
            for stack in &columns {
                let mut stack_tiles = Vec::new();
                for &pid in stack {
                    let t = tiles.insert_pane(pid);
                    if first_pane_tile.is_none() {
                        first_pane_tile = Some(t);
                    }
                    stack_tiles.push(t);
                }
                if stack_tiles.len() == 1 {
                    col_tiles.push(stack_tiles[0]);
                } else if !stack_tiles.is_empty() {
                    col_tiles.push(tiles.insert_vertical_tile(stack_tiles));
                }
            }
            let root = if col_tiles.len() == 1 {
                Some(col_tiles[0])
            } else if !col_tiles.is_empty() {
                Some(tiles.insert_horizontal_tile(col_tiles))
            } else {
                None
            };
            let mut tree = egui_tiles::Tree::empty("plexi_columns");
            tree.tiles = tiles;
            tree.root = root;
            let prev_focused_pid = self.windows[win_idx].focused_pane.and_then(|t| {
                match self.windows[win_idx].tree.tiles.get(t) {
                    Some(Tile::Pane(id)) => Some(*id),
                    _ => None,
                }
            });
            self.windows[win_idx].tree = tree;
            if let Some(pid) = prev_focused_pid {
                if let Some(t) = self.windows[win_idx].tree.tiles.find_pane(&pid) {
                    self.windows[win_idx].focused_pane = Some(t);
                } else {
                    self.windows[win_idx].focused_pane = first_pane_tile;
                }
            } else {
                self.windows[win_idx].focused_pane = first_pane_tile;
            }
        }
        self.record_focused_pane_memory();
    }

    /// Derive the ordered horizontal strip of columns for `context_id`.
    pub fn columns_for_context(&self, context_id: u64) -> Vec<StripColumn> {
        let mut raw_cols: Vec<(u64, Vec<PaneId>)> = Vec::new();

        // Include conjoined parent as column 0 when active in this child context.
        if context_id == self.router.active().context_id {
            if let Some(parent_pane_id) = self.conjoined_parent() {
                let parent_ctx_id = self
                    .find_pane_in_any_window(parent_pane_id)
                    .map(|(wi, _)| self.windows[wi].context_id)
                    .unwrap_or(context_id);
                if parent_ctx_id != context_id {
                    raw_cols.push((parent_ctx_id, vec![parent_pane_id]));
                }
            }
        }

        if let Some(explicit) = self.zoom.explicit_columns.get(&context_id) {
            let live_panes: std::collections::HashSet<PaneId> = self
                .windows
                .iter()
                .filter(|w| w.context_id == context_id)
                .flat_map(|w| w.panes.keys().copied())
                .collect();
            for stack in explicit {
                let filtered: Vec<PaneId> = stack
                    .iter()
                    .copied()
                    .filter(|pid| live_panes.contains(pid))
                    .collect();
                if !filtered.is_empty() {
                    raw_cols.push((context_id, filtered));
                }
            }
        } else {
            let mut ctx_windows: Vec<(usize, &crate::host::context::Window)> = self
                .windows
                .iter()
                .enumerate()
                .filter(|(_, w)| w.context_id == context_id)
                .collect();
            ctx_windows.sort_by_key(|(idx, w)| (w.grid_x, w.grid_y, *idx));

            for (_, win) in ctx_windows {
                let Some(root_id) = win.tree.root() else {
                    continue;
                };
                match win.tree.tiles.get(root_id) {
                    Some(Tile::Container(Container::Linear(lin)))
                        if lin.dir == LinearDir::Horizontal =>
                    {
                        for &col_tile in &lin.children {
                            let mut stack = Vec::new();
                            collect_leaf_panes_in_tile(&win.tree, &win.panes, col_tile, &mut stack);
                            if !stack.is_empty() {
                                raw_cols.push((context_id, stack));
                            }
                        }
                    }
                    Some(Tile::Container(Container::Grid(grid))) => {
                        for &col_tile in grid.children() {
                            let mut stack = Vec::new();
                            collect_leaf_panes_in_tile(&win.tree, &win.panes, col_tile, &mut stack);
                            if !stack.is_empty() {
                                raw_cols.push((context_id, stack));
                            }
                        }
                    }
                    _ => {
                        let mut stack = Vec::new();
                        collect_leaf_panes_in_tile(&win.tree, &win.panes, root_id, &mut stack);
                        if !stack.is_empty() {
                            raw_cols.push((context_id, stack));
                        }
                    }
                }
            }
        }

        let total = raw_cols.len();
        raw_cols
            .into_iter()
            .enumerate()
            .map(|(idx, (col_ctx_id, panes))| {
                let default_preset = if total >= 2 && idx == 0 {
                    ColumnWidthPreset::Parent35
                } else {
                    ColumnWidthPreset::Child65
                };
                let preset = self
                    .zoom
                    .column_presets
                    .get(&(context_id, idx))
                    .copied()
                    .unwrap_or(default_preset);
                StripColumn {
                    index: idx,
                    context_id: col_ctx_id,
                    panes,
                    preset,
                }
            })
            .collect()
    }

    /// True if `pane_id` is in a column that is currently fully visible (not clipped off-screen)
    /// at `ZoomLevel::Columns` and no overlay/queue owns input (`SPEC.md` R40).
    pub fn pane_qualifies_for_input(&self, pane_id: PaneId) -> bool {
        if self.zoom.level != ZoomLevel::Columns {
            return false;
        }
        if self.attention_queue.decision_overlay.is_some() || self.attention_queue.open {
            return false;
        }
        if self.overlay_surface().is_some() {
            return false;
        }
        if self.active_focused_pane_id() != Some(pane_id) {
            return false;
        }
        let ctx_id = self.router.active().context_id;
        let cols = self.columns_for_context(ctx_id);
        if cols.len() <= 2 {
            return true;
        }
        let offset = self.zoom.strip_offset_for(ctx_id);
        cols.iter()
            .find(|c| c.panes.contains(&pane_id))
            .is_some_and(|c| c.index >= offset && c.index < offset + 2)
    }

    /// Focus `pane_id` inside its context and update column & context focus memory.
    pub fn focus_pane_in_strip(&mut self, pane_id: PaneId) -> bool {
        if Some(pane_id) == self.conjoined_parent() {
            self.conjoined_parent_focus = Some(pane_id);
            let ctx_id = self.router.active().context_id;
            self.zoom.column_last_pane.insert((ctx_id, 0), pane_id);
            let total = self.columns_for_context(ctx_id).len();
            self.ensure_column_visible(ctx_id, 0, total);
            self.sync_attention_queue();
            return true;
        }

        let Some((win_idx, tile_id)) = self.find_pane_in_any_window(pane_id) else {
            return false;
        };
        self.conjoined_parent_focus = None;
        let ctx_id = self.windows[win_idx].context_id;
        if let Some(router_idx) = self
            .router
            .as_slice()
            .iter()
            .position(|c| c.context_id == ctx_id)
        {
            self.router.set_active(router_idx);
        }
        self.active_window = win_idx;
        let wid = self.windows[win_idx].window_id;
        self.context_active_window.insert(ctx_id, wid);
        self.windows[win_idx].navigate_to(tile_id);
        self.mark_agent_seen(pane_id);

        self.zoom.context_last_pane.insert(ctx_id, pane_id);
        let cols = self.columns_for_context(ctx_id);
        if let Some(col) = cols.iter().find(|c| c.panes.contains(&pane_id)) {
            self.zoom
                .column_last_pane
                .insert((ctx_id, col.index), pane_id);
            self.ensure_column_visible(ctx_id, col.index, cols.len());
        }
        self.sync_attention_queue();
        true
    }

    /// Jump to `pane_id` at `ZoomLevel::Columns` (used by queue bar Enter, ask jumps, etc.).
    pub fn jump_to_pane_at_columns(&mut self, pane_id: PaneId) -> bool {
        let ok = self.focus_pane_in_strip(pane_id);
        self.set_zoom_level_with_transition(ZoomLevel::Columns);
        self.zoom.ghost = None;
        self.zoom.atlas_peek_open = false;
        ok
    }

    /// Switch to `context_id` and restore its last active pane (`SPEC.md` R36).
    pub fn switch_to_context_restoring_last_pane(&mut self, context_id: u64) -> bool {
        // Save current context's focused pane before leaving.
        self.record_focused_pane_memory();

        let Some(router_idx) = self
            .router
            .as_slice()
            .iter()
            .position(|c| c.context_id == context_id)
        else {
            return false;
        };
        self.switch_workspace(router_idx);

        // Restore last active pane for `context_id` if still alive.
        if let Some(&last_pid) = self.zoom.context_last_pane.get(&context_id) {
            if let Some((win_idx, tile_id)) = self.find_pane_in_any_window(last_pid) {
                if self.windows[win_idx].context_id == context_id {
                    self.active_window = win_idx;
                    self.windows[win_idx].navigate_to(tile_id);
                }
            }
        }
        self.record_focused_pane_memory();
        self.sync_attention_queue();
        true
    }

    /// Move focus between columns (`Left`/`Right`) or stacked panes (`Up`/`Down`)
    /// at `ZoomLevel::Columns`. Stops at edges with an edge bump; never wraps (`SPEC.md` R32).
    pub fn navigate_columns_strip(&mut self, dir: Direction) -> bool {
        let ctx_id = self.router.active().context_id;
        let cols = self.columns_for_context(ctx_id);
        if cols.is_empty() {
            self.record_edge_bump(dir);
            return false;
        }

        let focused_pid = self.active_focused_pane_id();
        let (cur_col_idx, cur_row_idx) = focused_pid
            .and_then(|pid| {
                cols.iter()
                    .find_map(|c| c.panes.iter().position(|&p| p == pid).map(|r| (c.index, r)))
            })
            .unwrap_or((0, 0));

        // Record current pane in its column before moving.
        if let Some(pid) = focused_pid {
            self.zoom
                .column_last_pane
                .insert((ctx_id, cur_col_idx), pid);
        }

        match dir {
            Direction::Left => {
                if cur_col_idx == 0 {
                    self.record_edge_bump(Direction::Left);
                    return false;
                }
                let target_col = &cols[cur_col_idx - 1];
                let target_pid = self
                    .zoom
                    .column_last_pane
                    .get(&(ctx_id, target_col.index))
                    .copied()
                    .filter(|pid| target_col.panes.contains(pid))
                    .unwrap_or(target_col.panes[0]);
                self.focus_pane_in_strip(target_pid);
                self.ensure_column_visible(ctx_id, target_col.index, cols.len());
                true
            }
            Direction::Right => {
                if cur_col_idx + 1 >= cols.len() {
                    self.record_edge_bump(Direction::Right);
                    return false;
                }
                let target_col = &cols[cur_col_idx + 1];
                let target_pid = self
                    .zoom
                    .column_last_pane
                    .get(&(ctx_id, target_col.index))
                    .copied()
                    .filter(|pid| target_col.panes.contains(pid))
                    .unwrap_or(target_col.panes[0]);
                self.focus_pane_in_strip(target_pid);
                self.ensure_column_visible(ctx_id, target_col.index, cols.len());
                true
            }
            Direction::Down => {
                let stack = &cols[cur_col_idx].panes;
                if cur_row_idx + 1 >= stack.len() {
                    self.record_edge_bump(Direction::Down);
                    return false;
                }
                let target_pid = stack[cur_row_idx + 1];
                self.focus_pane_in_strip(target_pid);
                true
            }
            Direction::Up => {
                let stack = &cols[cur_col_idx].panes;
                if cur_row_idx == 0 {
                    self.record_edge_bump(Direction::Up);
                    return false;
                }
                let target_pid = stack[cur_row_idx - 1];
                self.focus_pane_in_strip(target_pid);
                true
            }
        }
    }

    pub(crate) fn set_zoom_level_with_transition(&mut self, next_level: ZoomLevel) {
        let from_progress = self.zoom.visual_level_progress(false);
        let to_progress = match next_level {
            ZoomLevel::Columns => 0.0,
            ZoomLevel::SiblingCards => 1.0,
            ZoomLevel::TreeAtlas => 2.0,
        };
        self.zoom.level = next_level;
        self.zoom.transition = Some((from_progress, to_progress, Instant::now()));
    }

    /// `Cmd+Esc`:
    /// 1. Closes innermost Plexi surface (`DecisionOverlay`, `AttentionQueue` list, or atlas `Peek`).
    /// 2. Else zooms out one level (`Columns -> SiblingCards -> TreeAtlas`).
    /// 3. At `TreeAtlas` with no open surface: no-op with edge bump (`SPEC.md` R31).
    pub fn zoom_out_or_close_surface(&mut self) -> bool {
        if self.attention_queue.decision_overlay.is_some() {
            self.attention_queue.decision_overlay = None;
            log::info!("zoom: Cmd+Esc closed DecisionOverlay");
            return true;
        }
        if self.attention_queue.open {
            self.attention_queue.open = false;
            log::info!("zoom: Cmd+Esc closed AttentionQueue list");
            return true;
        }
        if self.zoom.level == ZoomLevel::TreeAtlas && self.zoom.atlas_peek_open {
            self.zoom.atlas_peek_open = false;
            log::info!("zoom: Cmd+Esc closed atlas Peek");
            return true;
        }

        match self.zoom.level {
            ZoomLevel::Columns => {
                self.record_focused_pane_memory();
                let ctx_id = self.router.active().context_id;
                let parent_id = self.router.active().parent_id;
                let pane_id = self.active_focused_pane_id().unwrap_or(1);
                let cols = self.columns_for_context(ctx_id);
                let col_idx = cols
                    .iter()
                    .find(|c| c.panes.contains(&pane_id))
                    .map(|c| c.index)
                    .unwrap_or(0);
                let offset = self.zoom.strip_offset_for(ctx_id);
                self.zoom.ghost = Some(GhostMarker {
                    context_id: ctx_id,
                    parent_context_id: parent_id,
                    column_idx: col_idx,
                    pane_id,
                    strip_offset: offset,
                });
                self.zoom.cards_focused_context = Some(ctx_id);
                self.set_zoom_level_with_transition(ZoomLevel::SiblingCards);
                log::info!(
                    "zoom: Columns -> SiblingCards (ghost ctx={ctx_id} col={col_idx} pane={pane_id})"
                );
                true
            }
            ZoomLevel::SiblingCards => {
                let focus_ctx = self
                    .zoom
                    .cards_focused_context
                    .unwrap_or_else(|| self.router.active().context_id);
                self.zoom.atlas_focused_context = Some(focus_ctx);
                self.zoom.atlas_peek_open = false;
                self.set_zoom_level_with_transition(ZoomLevel::TreeAtlas);
                log::info!("zoom: SiblingCards -> TreeAtlas (focus_ctx={focus_ctx})");
                true
            }
            ZoomLevel::TreeAtlas => {
                self.record_edge_bump(Direction::Up);
                false
            }
        }
    }

    /// `Cmd+Enter`:
    /// Zooms in one level back toward the ghost (`TreeAtlas -> SiblingCards -> Columns`).
    /// No-op at `Columns` with an edge bump (`SPEC.md` R31).
    pub fn zoom_in_level(&mut self) -> bool {
        match self.zoom.level {
            ZoomLevel::Columns => {
                self.record_edge_bump(Direction::Down);
                false
            }
            ZoomLevel::SiblingCards => {
                self.restore_ghost_or_active_columns();
                log::info!("zoom: SiblingCards -> Columns");
                true
            }
            ZoomLevel::TreeAtlas => {
                self.zoom.atlas_peek_open = false;
                let target_ctx = self
                    .zoom
                    .ghost
                    .as_ref()
                    .map(|g| g.context_id)
                    .or(self.zoom.atlas_focused_context)
                    .unwrap_or_else(|| self.router.active().context_id);
                if let Some(r_idx) = self
                    .router
                    .as_slice()
                    .iter()
                    .position(|c| c.context_id == target_ctx)
                {
                    self.router.set_active(r_idx);
                }
                self.zoom.cards_focused_context = Some(target_ctx);
                self.set_zoom_level_with_transition(ZoomLevel::SiblingCards);
                log::info!("zoom: TreeAtlas -> SiblingCards (ctx={target_ctx})");
                true
            }
        }
    }

    /// Restore the ghost marker (`q` in atlas or `Cmd+Enter` from `SiblingCards`).
    /// If the ghost target died, falls back to column neighbour -> adjacent column -> parent context (`SPEC.md` Q7).
    pub fn restore_ghost_or_active_columns(&mut self) {
        self.zoom.atlas_peek_open = false;
        let Some(ghost) = self.zoom.ghost.take() else {
            let ctx_id = self
                .zoom
                .cards_focused_context
                .unwrap_or_else(|| self.router.active().context_id);
            self.switch_to_context_restoring_last_pane(ctx_id);
            self.set_zoom_level_with_transition(ZoomLevel::Columns);
            return;
        };

        // 1. Exact ghost pane still alive in ghost context.
        if self.find_pane_in_any_window(ghost.pane_id).is_some() {
            self.switch_to_context_restoring_last_pane(ghost.context_id);
            self.focus_pane_in_strip(ghost.pane_id);
            let total = self.columns_for_context(ghost.context_id).len();
            let clamped_offset = ghost.strip_offset.min(total.saturating_sub(2));
            self.zoom
                .strip_offsets
                .insert(ghost.context_id, clamped_offset);
            self.set_zoom_level_with_transition(ZoomLevel::Columns);
            return;
        }

        // 2. Ghost pane died, but ghost context still exists -> column neighbour or adjacent column.
        let ctx_alive = self
            .router
            .as_slice()
            .iter()
            .any(|c| c.context_id == ghost.context_id);
        if ctx_alive {
            self.switch_to_context_restoring_last_pane(ghost.context_id);
            let cols = self.columns_for_context(ghost.context_id);
            if !cols.is_empty() {
                let col_idx = ghost.column_idx.min(cols.len() - 1);
                if let Some(&fallback_pane) = cols[col_idx].panes.first() {
                    self.focus_pane_in_strip(fallback_pane);
                }
            }
            self.set_zoom_level_with_transition(ZoomLevel::Columns);
            return;
        }

        // 3. Ghost context died -> fall back to parent context (else active context).
        let fallback_ctx = ghost
            .parent_context_id
            .filter(|pid| self.router.as_slice().iter().any(|c| c.context_id == *pid))
            .unwrap_or_else(|| self.router.active().context_id);
        self.switch_to_context_restoring_last_pane(fallback_ctx);
        self.set_zoom_level_with_transition(ZoomLevel::Columns);
    }

    /// Sibling contexts for the current `SiblingCards` view: same `parent_id`, in creation order.
    pub fn sibling_card_contexts(&self) -> Vec<u64> {
        let anchor_id = self
            .zoom
            .cards_focused_context
            .unwrap_or_else(|| self.router.active().context_id);
        let parent_id = self
            .router
            .as_slice()
            .iter()
            .find(|c| c.context_id == anchor_id)
            .and_then(|c| c.parent_id);
        let siblings: Vec<u64> = self
            .router
            .as_slice()
            .iter()
            .filter(|c| !c.parked && c.parent_id == parent_id)
            .map(|c| c.context_id)
            .collect();
        if siblings.is_empty() {
            vec![self.router.active().context_id]
        } else {
            siblings
        }
    }

    /// Handle directional movement (`Cmd+H/J/K/L`) at the current zoom level.
    pub fn navigate_at_current_zoom_level(&mut self, dir: Direction) -> bool {
        match self.zoom.level {
            ZoomLevel::Columns => self.navigate_columns_strip(dir),
            ZoomLevel::SiblingCards => self.navigate_sibling_cards(dir),
            ZoomLevel::TreeAtlas => self.navigate_tree_atlas_dir(dir),
        }
    }

    pub fn navigate_sibling_cards(&mut self, dir: Direction) -> bool {
        let siblings = self.sibling_card_contexts();
        if siblings.is_empty() {
            self.record_edge_bump(dir);
            return false;
        }
        let cur_ctx = self
            .zoom
            .cards_focused_context
            .unwrap_or_else(|| self.router.active().context_id);
        let cur_idx = siblings.iter().position(|&c| c == cur_ctx).unwrap_or(0);

        match dir {
            Direction::Left => {
                if cur_idx == 0 {
                    self.record_edge_bump(Direction::Left);
                    false
                } else {
                    self.zoom.cards_focused_context = Some(siblings[cur_idx - 1]);
                    true
                }
            }
            Direction::Right => {
                if cur_idx + 1 >= siblings.len() {
                    self.record_edge_bump(Direction::Right);
                    false
                } else {
                    self.zoom.cards_focused_context = Some(siblings[cur_idx + 1]);
                    true
                }
            }
            Direction::Up | Direction::Down => {
                self.record_edge_bump(dir);
                false
            }
        }
    }

    /// Enter the currently focused sibling card at its last active pane.
    pub fn enter_focused_sibling_card(&mut self) -> bool {
        let target_ctx = self
            .zoom
            .cards_focused_context
            .unwrap_or_else(|| self.router.active().context_id);
        self.zoom.ghost = None;
        self.switch_to_context_restoring_last_pane(target_ctx);
        self.set_zoom_level_with_transition(ZoomLevel::Columns);
        true
    }

    /// Build the ordered tree of contexts and their positions for `ZoomLevel::TreeAtlas`.
    pub fn compute_atlas_graph(&self, viewport: Rect) -> (Vec<AtlasNode>, Vec<AtlasEdge>) {
        let contexts: Vec<&crate::host::context::Context> = self
            .router
            .as_slice()
            .iter()
            .filter(|c| !c.parked)
            .collect();
        let live_ids: std::collections::HashSet<u64> =
            contexts.iter().map(|c| c.context_id).collect();

        let roots: Vec<u64> = contexts
            .iter()
            .filter(|c| c.parent_id.is_none_or(|pid| !live_ids.contains(&pid)))
            .map(|c| c.context_id)
            .collect();

        // Radial tree: the root sits at the viewport center and descendants
        // fan out on concentric rings, each node owning an angular wedge sized
        // by its leaf count. Several roots hang off a virtual hub at the center.
        let center = viewport.center();
        let hub = roots.len() > 1;
        let children_of = |ctx_id: u64| -> Vec<u64> {
            contexts
                .iter()
                .filter(|c| c.parent_id == Some(ctx_id))
                .map(|c| c.context_id)
                .collect()
        };
        fn leaves(ctx_id: u64, children_of: &dyn Fn(u64) -> Vec<u64>) -> usize {
            let kids = children_of(ctx_id);
            if kids.is_empty() {
                1
            } else {
                kids.iter().map(|&k| leaves(k, children_of)).sum()
            }
        }
        fn max_depth(ctx_id: u64, children_of: &dyn Fn(u64) -> Vec<u64>) -> usize {
            children_of(ctx_id)
                .iter()
                .map(|&k| 1 + max_depth(k, children_of))
                .max()
                .unwrap_or(0)
        }
        let deepest = roots
            .iter()
            .map(|&r| max_depth(r, &children_of))
            .max()
            .unwrap_or(0);
        let max_ring = deepest + usize::from(hub);
        let (rx_step, ry_step) = if max_ring == 0 {
            (0.0, 0.0)
        } else {
            let fit_x = ((viewport.width() - style::ATLAS_NODE_W) * 0.5 - 16.0) / max_ring as f32;
            let fit_y = ((viewport.height() - style::ATLAS_NODE_H) * 0.5 - 16.0) / max_ring as f32;
            (
                fit_x.max(style::ATLAS_NODE_W * 0.75),
                fit_y.max(style::ATLAS_NODE_H * 1.1),
            )
        };

        let mut nodes_out: Vec<AtlasNode> = Vec::new();
        // (context_id, depth, ring, wedge_start, wedge_end)
        struct Placement {
            ctx_id: u64,
            depth: usize,
            ring: usize,
            a0: f32,
            a1: f32,
        }
        let tau = std::f32::consts::TAU;
        let down = std::f32::consts::FRAC_PI_2;
        let mut stack: Vec<Placement> = Vec::new();
        {
            // Root wedges: the whole circle (starting so a lone child lands below).
            let total: usize = roots.iter().map(|&r| leaves(r, &children_of)).sum::<usize>().max(1);
            let mut a = down - tau * 0.5;
            let mut placements = Vec::new();
            for &r in &roots {
                let span = tau * leaves(r, &children_of) as f32 / total as f32;
                placements.push(Placement {
                    ctx_id: r,
                    depth: 0,
                    ring: usize::from(hub),
                    a0: a,
                    a1: a + span,
                });
                a += span;
            }
            placements.reverse();
            stack.extend(placements);
        }
        while let Some(pl) = stack.pop() {
            let ctx_id = pl.ctx_id;
            let mid = (pl.a0 + pl.a1) * 0.5;
            let ring = pl.ring as f32;
            let node_center = if pl.ring == 0 {
                center
            } else {
                Pos2::new(
                    center.x + mid.cos() * rx_step * ring,
                    center.y + mid.sin() * ry_step * ring,
                )
            };
            let rect = Rect::from_center_size(
                node_center,
                Vec2::new(style::ATLAS_NODE_W, style::ATLAS_NODE_H),
            );
            let name = contexts
                .iter()
                .find(|c| c.context_id == ctx_id)
                .map(|c| c.name.displayed().to_string())
                .unwrap_or_else(|| format!("ctx-{ctx_id}"));
            let parent_id = contexts
                .iter()
                .find(|c| c.context_id == ctx_id)
                .and_then(|c| c.parent_id)
                .filter(|pid| live_ids.contains(pid));

            // Same source as every other surface: the queue's per-context count.
            let direct_asks = self.attention_queue.ask_count_for_context(ctx_id);
            let ask_rollup = self
                .attention_queue
                .ask_count_for_context_subtree(ctx_id, &self.router);
            let failed_count = self
                .attention_queue
                .items
                .iter()
                .filter(|b| b.category == QueueCategory::Failed && b.context_id == ctx_id)
                .count();
            let pane_count: usize = self
                .windows
                .iter()
                .filter(|w| w.context_id == ctx_id)
                .map(|w| w.panes.len())
                .sum();
            let is_ghost = self
                .zoom
                .ghost
                .as_ref()
                .is_some_and(|g| g.context_id == ctx_id);
            let preview_lines = self.context_preview_lines(ctx_id, 3);
            let order = nodes_out.len();
            nodes_out.push(AtlasNode {
                context_id: ctx_id,
                parent_id,
                name,
                depth: pl.depth,
                order,
                rect,
                ask_rollup,
                direct_asks,
                failed_count,
                pane_count,
                is_ghost,
                preview_lines,
            });

            let kids = children_of(ctx_id);
            let total: usize = kids.iter().map(|&k| leaves(k, &children_of)).sum::<usize>().max(1);
            let mut a = pl.a0;
            let mut kid_placements = Vec::new();
            for &k in &kids {
                let span = (pl.a1 - pl.a0) * leaves(k, &children_of) as f32 / total as f32;
                kid_placements.push(Placement {
                    ctx_id: k,
                    depth: pl.depth + 1,
                    ring: pl.ring + 1,
                    a0: a,
                    a1: a + span,
                });
                a += span;
            }
            kid_placements.reverse();
            stack.extend(kid_placements);
        }

        let mut edges_out: Vec<AtlasEdge> = Vec::new();
        // Parent/child context relationships.
        for node in &nodes_out {
            if let Some(pid) = node.parent_id {
                if let Some(parent_node) = nodes_out.iter().find(|n| n.context_id == pid) {
                    let kind = if node.ask_rollup > 0 {
                        AtlasEdgeKind::NeedsRollup
                    } else if node.failed_count > 0 {
                        AtlasEdgeKind::Failed
                    } else {
                        AtlasEdgeKind::TreeRoad
                    };
                    edges_out.push(AtlasEdge {
                        from_context_id: pid,
                        to_context_id: node.context_id,
                        from_pos: parent_node.rect.center(),
                        to_pos: node.rect.center(),
                        kind,
                    });
                }
            }
        }
        // Virtual hub: with several roots, each hangs off the center point.
        if hub {
            for node in nodes_out.iter().filter(|n| n.parent_id.is_none()) {
                edges_out.push(AtlasEdge {
                    from_context_id: 0,
                    to_context_id: node.context_id,
                    from_pos: center,
                    to_pos: node.rect.center(),
                    kind: AtlasEdgeKind::TreeRoad,
                });
            }
        }
        // Spawn lineage: an agent pane in one context spawned by a pane in another.
        let mut spawn_pairs: Vec<(u64, u64)> = Vec::new();
        for win in &self.windows {
            for pane in win.panes.values() {
                let Some(parent_pane) = pane.as_terminal().and_then(|t| t.parent_pane_id) else {
                    continue;
                };
                let Some((pwi, _)) = self.find_pane_in_any_window(parent_pane) else {
                    continue;
                };
                let from_ctx = self.windows[pwi].context_id;
                let to_ctx = win.context_id;
                if from_ctx != to_ctx && !spawn_pairs.contains(&(from_ctx, to_ctx)) {
                    spawn_pairs.push((from_ctx, to_ctx));
                }
            }
        }
        for (from_ctx, to_ctx) in spawn_pairs {
            if let (Some(a), Some(b)) = (
                nodes_out.iter().find(|n| n.context_id == from_ctx),
                nodes_out.iter().find(|n| n.context_id == to_ctx),
            ) {
                edges_out.push(AtlasEdge {
                    from_context_id: from_ctx,
                    to_context_id: to_ctx,
                    from_pos: a.rect.center(),
                    to_pos: b.rect.center(),
                    kind: AtlasEdgeKind::Spawn,
                });
            }
        }

        // Add typed pipe / spawn links for the focused node only (`SPEC.md` Q4).
        let focused_ctx = self
            .zoom
            .atlas_focused_context
            .unwrap_or_else(|| self.router.active().context_id);
        for &(from_ctx, to_ctx) in &self.zoom.context_links {
            if from_ctx == focused_ctx || to_ctx == focused_ctx {
                if let (Some(a), Some(b)) = (
                    nodes_out.iter().find(|n| n.context_id == from_ctx),
                    nodes_out.iter().find(|n| n.context_id == to_ctx),
                ) {
                    edges_out.push(AtlasEdge {
                        from_context_id: from_ctx,
                        to_context_id: to_ctx,
                        from_pos: a.rect.center(),
                        to_pos: b.rect.center(),
                        kind: AtlasEdgeKind::PipeLink,
                    });
                }
            }
        }

        (nodes_out, edges_out)
    }

    pub(crate) fn context_preview_lines(&self, context_id: u64, max_lines: usize) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(ask) = self.attention_queue.first_ask_for_context(context_id) {
            out.push(format!("◆ {}", ask.summary));
        }
        for win in self.windows.iter().filter(|w| w.context_id == context_id) {
            for (&pid, pane) in &win.panes {
                if out.len() >= max_lines {
                    break;
                }
                let name = pane
                    .as_terminal()
                    .and_then(|t| t.name.as_deref())
                    .or_else(|| pane.as_app().map(|a| a.name.as_str()))
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("pane-{pid}"));
                if let Some(ag) = pane.agent() {
                    let detail = ag.detail.as_deref().unwrap_or(ag.state.as_str());
                    out.push(format!("{name}: {detail}"));
                } else {
                    out.push(name);
                }
            }
        }
        out.truncate(max_lines);
        out
    }

    /// Move in the Tree Atlas along `h` (`Left` = parent), `l` (`Right` = last-visited/first child),
    /// `j` (`Down` = next node at same depth), `k` (`Up` = prev node at same depth).
    /// Never wraps (`SPEC.md` lines 21–22, 32).
    pub fn navigate_tree_atlas_dir(&mut self, dir: Direction) -> bool {
        let (nodes, _) =
            self.compute_atlas_graph(Rect::from_min_size(Pos2::ZERO, Vec2::new(1440.0, 900.0)));
        if nodes.is_empty() {
            self.record_edge_bump(dir);
            return false;
        }
        let cur_ctx = self
            .zoom
            .atlas_focused_context
            .filter(|id| nodes.iter().any(|n| n.context_id == *id))
            .unwrap_or_else(|| self.router.active().context_id);
        let Some(cur_node) = nodes.iter().find(|n| n.context_id == cur_ctx) else {
            self.record_edge_bump(dir);
            return false;
        };

        match dir {
            Direction::Left => {
                // `h`: Parent context.
                let Some(parent_id) = cur_node.parent_id else {
                    self.record_edge_bump(Direction::Left);
                    return false;
                };
                self.zoom.atlas_last_child.insert(parent_id, cur_ctx);
                self.zoom.atlas_focused_context = Some(parent_id);
                true
            }
            Direction::Right => {
                // `l`: Last-visited (else first) child context.
                let mut children: Vec<&AtlasNode> = nodes
                    .iter()
                    .filter(|n| n.parent_id == Some(cur_ctx))
                    .collect();
                children.sort_by_key(|a| a.order);
                if children.is_empty() {
                    self.record_edge_bump(Direction::Right);
                    return false;
                }
                let target_child = self
                    .zoom
                    .atlas_last_child
                    .get(&cur_ctx)
                    .copied()
                    .filter(|cid| children.iter().any(|c| c.context_id == *cid))
                    .unwrap_or(children[0].context_id);
                self.zoom.atlas_last_child.insert(cur_ctx, target_child);
                self.zoom.atlas_focused_context = Some(target_child);
                true
            }
            Direction::Down | Direction::Up => {
                // `j` / `k`: Next / prev node at the SAME depth in tree (vertical y) order.
                let mut same_depth: Vec<&AtlasNode> =
                    nodes.iter().filter(|n| n.depth == cur_node.depth).collect();
                same_depth.sort_by_key(|a| a.order);
                let pos = same_depth
                    .iter()
                    .position(|n| n.context_id == cur_ctx)
                    .unwrap_or(0);
                if dir == Direction::Down {
                    if pos + 1 >= same_depth.len() {
                        self.record_edge_bump(Direction::Down);
                        return false;
                    }
                    let next_node = same_depth[pos + 1];
                    if let Some(pid) = next_node.parent_id {
                        self.zoom.atlas_last_child.insert(pid, next_node.context_id);
                    }
                    self.zoom.atlas_focused_context = Some(next_node.context_id);
                    true
                } else {
                    if pos == 0 {
                        self.record_edge_bump(Direction::Up);
                        return false;
                    }
                    let prev_node = same_depth[pos - 1];
                    if let Some(pid) = prev_node.parent_id {
                        self.zoom.atlas_last_child.insert(pid, prev_node.context_id);
                    }
                    self.zoom.atlas_focused_context = Some(prev_node.context_id);
                    true
                }
            }
        }
    }

    /// `[` / `]` in the Tree Atlas: cycle prev/next sibling (same `parent_id`). Wraps around (`SPEC.md` line 23).
    pub fn navigate_tree_atlas_sibling(&mut self, forward: bool) -> bool {
        let (nodes, _) =
            self.compute_atlas_graph(Rect::from_min_size(Pos2::ZERO, Vec2::new(1440.0, 900.0)));
        let cur_ctx = self
            .zoom
            .atlas_focused_context
            .unwrap_or_else(|| self.router.active().context_id);
        let Some(cur_node) = nodes.iter().find(|n| n.context_id == cur_ctx) else {
            return false;
        };
        let mut siblings: Vec<&AtlasNode> = nodes
            .iter()
            .filter(|n| n.parent_id == cur_node.parent_id)
            .collect();
        siblings.sort_by_key(|a| a.order);
        if siblings.len() <= 1 {
            self.record_edge_bump(if forward {
                Direction::Down
            } else {
                Direction::Up
            });
            return false;
        }
        let pos = siblings
            .iter()
            .position(|n| n.context_id == cur_ctx)
            .unwrap_or(0);
        let next_idx = if forward {
            (pos + 1) % siblings.len()
        } else {
            (pos + siblings.len() - 1) % siblings.len()
        };
        let next_ctx = siblings[next_idx].context_id;
        if let Some(pid) = cur_node.parent_id {
            self.zoom.atlas_last_child.insert(pid, next_ctx);
        }
        self.zoom.atlas_focused_context = Some(next_ctx);
        true
    }

    /// `n` / `N` in the Tree Atlas: cycle next/prev node that has a direct open ask. Wraps around (`SPEC.md` line 23).
    pub fn navigate_tree_atlas_needs_you(&mut self, forward: bool) -> bool {
        self.sync_attention_queue();
        let (mut nodes, _) =
            self.compute_atlas_graph(Rect::from_min_size(Pos2::ZERO, Vec2::new(1440.0, 900.0)));
        nodes.sort_by(|a, b| {
            a.depth
                .cmp(&b.depth)
                .then_with(|| a.order.cmp(&b.order))
        });
        let asking: Vec<u64> = nodes
            .iter()
            .filter(|n| n.direct_asks > 0)
            .map(|n| n.context_id)
            .collect();
        if asking.is_empty() {
            self.record_edge_bump(Direction::Right);
            return false;
        }
        let cur_ctx = self
            .zoom
            .atlas_focused_context
            .unwrap_or_else(|| self.router.active().context_id);
        let next_ctx = match asking.iter().position(|&c| c == cur_ctx) {
            Some(idx) => {
                if forward {
                    asking[(idx + 1) % asking.len()]
                } else {
                    asking[(idx + asking.len() - 1) % asking.len()]
                }
            }
            None => {
                if forward {
                    asking[0]
                } else {
                    *asking.last().unwrap()
                }
            }
        };
        self.zoom.atlas_focused_context = Some(next_ctx);
        true
    }

    /// `f` / `F` in the Tree Atlas: follow outgoing (`f`) or incoming (`F`) pipe/spawn link.
    pub fn navigate_tree_atlas_link(&mut self, forward: bool) -> bool {
        let cur_ctx = self
            .zoom
            .atlas_focused_context
            .unwrap_or_else(|| self.router.active().context_id);
        let target = if forward {
            self.zoom
                .context_links
                .iter()
                .find(|(from, _)| *from == cur_ctx)
                .map(|(_, to)| *to)
        } else {
            self.zoom
                .context_links
                .iter()
                .find(|(_, to)| *to == cur_ctx)
                .map(|(from, _)| *from)
        };
        if let Some(next_ctx) = target {
            self.zoom.atlas_focused_context = Some(next_ctx);
            true
        } else {
            self.record_edge_bump(if forward {
                Direction::Right
            } else {
                Direction::Left
            });
            false
        }
    }

    /// `Enter` in the Tree Atlas (`SPEC.md` R35):
    /// - If the focused atlas node has an open ask: dive to `ZoomLevel::Columns`, focus the asking
    ///   pane, and open the `DecisionOverlay` on that ask ID!
    /// - Else: dive to `ZoomLevel::Columns` and restore that context's last active pane.
    pub fn dive_from_tree_atlas(&mut self) -> bool {
        self.sync_attention_queue();
        let target_ctx = self
            .zoom
            .atlas_focused_context
            .unwrap_or_else(|| self.router.active().context_id);
        self.zoom.atlas_peek_open = false;
        self.zoom.ghost = None;

        if let Some(ask) = self
            .attention_queue
            .first_ask_for_context(target_ctx)
            .cloned()
        {
            let _ = self.open_decision_overlay_for_ask(&ask.id);
            return true;
        }

        self.switch_to_context_restoring_last_pane(target_ctx);
        self.set_zoom_level_with_transition(ZoomLevel::Columns);
        true
    }

    /// Handle bare keys on Plexi-owned zoom levels (`SiblingCards` and `TreeAtlas`).
    /// Swallows all unbound bare keys and `Cmd+V` so nothing leaks to any pane (`SPEC.md` R34, R35, R40).
    pub(crate) fn handle_zoom_surface_input(&mut self, input: &mut PlexiInput) -> KeyDisposition {
        if self.zoom.level == ZoomLevel::Columns {
            return KeyDisposition::Passthrough;
        }

        // Swallow Cmd+V / Paste on Plexi-owned surfaces.
        if input.consume_key(egui::Modifiers::COMMAND, Key::V)
            || input.consume_key(egui::Modifiers::CTRL, Key::V)
        {
            input.clear();
            return KeyDisposition::Consumed;
        }

        match self.zoom.level {
            ZoomLevel::SiblingCards => {
                let mut move_dir: Option<Direction> = None;
                let mut enter = false;

                for ev in input.events() {
                    if let egui::Event::Key {
                        key,
                        pressed: true,
                        repeat,
                        modifiers,
                        ..
                    } = ev
                    {
                        if modifiers.alt {
                            continue;
                        }
                        if (modifiers.command || modifiers.ctrl) && !modifiers.shift {
                            match key {
                                Key::H | Key::ArrowLeft => move_dir = Some(Direction::Left),
                                Key::L | Key::ArrowRight => move_dir = Some(Direction::Right),
                                Key::J | Key::ArrowDown => move_dir = Some(Direction::Down),
                                Key::K | Key::ArrowUp => move_dir = Some(Direction::Up),
                                _ => {}
                            }
                            continue;
                        }
                        if modifiers.command || modifiers.ctrl {
                            continue;
                        }
                        match key {
                            Key::H | Key::ArrowLeft => move_dir = Some(Direction::Left),
                            Key::L | Key::ArrowRight => move_dir = Some(Direction::Right),
                            Key::J | Key::ArrowDown => move_dir = Some(Direction::Down),
                            Key::K | Key::ArrowUp => move_dir = Some(Direction::Up),
                            Key::Enter if !*repeat => enter = true,
                            _ => {}
                        }
                    }
                }

                if enter {
                    self.enter_focused_sibling_card();
                } else if let Some(d) = move_dir {
                    self.navigate_sibling_cards(d);
                }
                input.clear();
                KeyDisposition::Consumed
            }
            ZoomLevel::TreeAtlas => {
                let mut move_dir: Option<Direction> = None;
                let mut sib_forward: Option<bool> = None;
                let mut needs_forward: Option<bool> = None;
                let mut link_forward: Option<bool> = None;
                let mut toggle_peek = false;
                let mut dive = false;
                let mut back_to_ghost = false;

                for ev in input.events() {
                    if let egui::Event::Key {
                        key,
                        pressed: true,
                        repeat,
                        modifiers,
                        ..
                    } = ev
                    {
                        if modifiers.alt {
                            continue;
                        }
                        if (modifiers.command || modifiers.ctrl) && !modifiers.shift {
                            match key {
                                Key::H | Key::ArrowLeft => move_dir = Some(Direction::Left),
                                Key::L | Key::ArrowRight => move_dir = Some(Direction::Right),
                                Key::J | Key::ArrowDown => move_dir = Some(Direction::Down),
                                Key::K | Key::ArrowUp => move_dir = Some(Direction::Up),
                                _ => {}
                            }
                            continue;
                        }
                        if modifiers.command || modifiers.ctrl {
                            continue;
                        }
                        match key {
                            Key::H | Key::ArrowLeft => move_dir = Some(Direction::Left),
                            Key::L | Key::ArrowRight => move_dir = Some(Direction::Right),
                            Key::J | Key::ArrowDown => move_dir = Some(Direction::Down),
                            Key::K | Key::ArrowUp => move_dir = Some(Direction::Up),
                            Key::OpenBracket => sib_forward = Some(false),
                            Key::CloseBracket => sib_forward = Some(true),
                            Key::N if !*repeat => needs_forward = Some(!modifiers.shift),
                            Key::F if !*repeat => link_forward = Some(!modifiers.shift),
                            Key::P if !*repeat => toggle_peek = true,
                            Key::Enter if !*repeat => dive = true,
                            Key::Q if !*repeat => back_to_ghost = true,
                            _ => {}
                        }
                    }
                }

                if back_to_ghost {
                    self.restore_ghost_or_active_columns();
                } else if dive {
                    self.dive_from_tree_atlas();
                } else if toggle_peek {
                    self.zoom.atlas_peek_open = !self.zoom.atlas_peek_open;
                    log::info!("zoom: atlas_peek_open={}", self.zoom.atlas_peek_open);
                } else if let Some(fwd) = sib_forward {
                    self.navigate_tree_atlas_sibling(fwd);
                } else if let Some(fwd) = needs_forward {
                    self.navigate_tree_atlas_needs_you(fwd);
                } else if let Some(fwd) = link_forward {
                    self.navigate_tree_atlas_link(fwd);
                } else if let Some(d) = move_dir {
                    self.navigate_tree_atlas_dir(d);
                }

                input.clear();
                KeyDisposition::Consumed
            }
            ZoomLevel::Columns => KeyDisposition::Passthrough,
        }
    }

    /// Synchronize the active window's horizontal root `Linear` container shares
    /// with the Niri column strip presets (`35%` parent / `65%` child) and scroll offset.
    pub(crate) fn sync_strip_tree_shares(&mut self) {
        if self.zoom.level != ZoomLevel::Columns {
            return;
        }
        let ctx_id = self.router.active().context_id;
        let cols = self.columns_for_context(ctx_id);
        let offset = self.zoom.strip_offset_for(ctx_id);
        let Some(win) = self.windows.get_mut(self.active_window) else {
            return;
        };
        let Some(root_id) = win.tree.root() else {
            return;
        };
        if let Some(Tile::Container(Container::Linear(lin))) = win.tree.tiles.get_mut(root_id) {
            if lin.dir == LinearDir::Horizontal && lin.children.len() >= 2 {
                let n = lin.children.len();
                let children = lin.children.clone();
                for (idx, child_id) in children.into_iter().enumerate() {
                    let ratio = cols
                        .get(idx)
                        .map(|c| c.preset.ratio())
                        .unwrap_or(if idx == 0 {
                            style::COLUMN_WIDTH_PARENT
                        } else {
                            style::COLUMN_WIDTH_CHILD
                        });
                    let share = if n <= 2 {
                        ratio * 2.0
                    } else if idx >= offset && idx < offset + 2 {
                        let slot_ratio = if idx == offset {
                            cols.get(idx)
                                .map(|c| c.preset.ratio())
                                .unwrap_or(style::COLUMN_WIDTH_PARENT)
                        } else {
                            cols.get(idx)
                                .map(|c| c.preset.ratio())
                                .unwrap_or(style::COLUMN_WIDTH_CHILD)
                        };
                        slot_ratio * (n as f32)
                    } else {
                        0.06
                    };
                    lin.shares.set_share(child_id, share);
                }
            }
        }
    }

    /// Returns `(has_left_clipped, has_right_clipped)` for the active context's column strip.
    pub(crate) fn active_strip_clipped_neighbors(&self) -> (bool, bool) {
        if self.zoom.level != ZoomLevel::Columns {
            return (false, false);
        }
        let ctx_id = self.router.active().context_id;
        let cols = self.columns_for_context(ctx_id);
        if cols.len() <= 2 {
            return (false, false);
        }
        let offset = self.zoom.strip_offset_for(ctx_id);
        (offset > 0, offset + 2 < cols.len())
    }

    /// Paint the `28px` clipped neighbor indicator strips at the left/right edges
    /// of the `ZoomLevel::Columns` viewport when off-screen columns exist.
    pub(crate) fn paint_clipped_neighbor_edges(
        painter: &egui::Painter,
        canvas_rect: Rect,
        has_left: bool,
        has_right: bool,
        colors: crate::ui::theme::Colors,
    ) {
        if has_left {
            let left_strip = Rect::from_min_size(
                canvas_rect.min,
                Vec2::new(style::STRIP_PEEK_W, canvas_rect.height()),
            );
            painter.rect_filled(left_strip, style::RADIUS_SM, colors.depth_tint(1));
            painter.rect_stroke(
                left_strip,
                style::RADIUS_SM,
                Stroke::new(1.0_f32, colors.border),
                egui::StrokeKind::Inside,
            );
            painter.text(
                left_strip.center(),
                Align2::CENTER_CENTER,
                "‹",
                FontId::new(style::TEXT_BODY, FontFamily::Proportional),
                colors.text_dim,
            );
        }
        if has_right {
            let right_strip = Rect::from_min_size(
                Pos2::new(canvas_rect.max.x - style::STRIP_PEEK_W, canvas_rect.min.y),
                Vec2::new(style::STRIP_PEEK_W, canvas_rect.height()),
            );
            painter.rect_filled(right_strip, style::RADIUS_SM, colors.depth_tint(1));
            painter.rect_stroke(
                right_strip,
                style::RADIUS_SM,
                Stroke::new(1.0_f32, colors.border),
                egui::StrokeKind::Inside,
            );
            painter.text(
                right_strip.center(),
                Align2::CENTER_CENTER,
                "›",
                FontId::new(style::TEXT_BODY, FontFamily::Proportional),
                colors.text_dim,
            );
        }
    }

    /// Render the single 1-line Path Bar at the top of the window (`34px` height):
    /// - Left: Context breadcrumb (`root › child`) + current zoom level badge when zoomed out
    /// - Center: Niri-style column strip position pips (`• ● •`) at `Columns`, or sibling/atlas count
    /// - Right: Calm Attention Queue pill (`◆N · total` + top bar preview, clickable to toggle `Cmd+I`)
    pub(crate) fn draw_zoom_path_bar(&mut self, ui: &mut egui::Ui) {
        self.sync_attention_queue();
        let colors = self.colors;
        let active_ctx_id = match self.zoom.level {
            ZoomLevel::Columns => self.router.active().context_id,
            ZoomLevel::SiblingCards => self
                .zoom
                .cards_focused_context
                .unwrap_or_else(|| self.router.active().context_id),
            ZoomLevel::TreeAtlas => self
                .zoom
                .atlas_focused_context
                .unwrap_or_else(|| self.router.active().context_id),
        };
        let breadcrumb = self.context_breadcrumb_string(active_ctx_id);
        let cols = self.columns_for_context(active_ctx_id);
        let focused_pid = self.active_focused_pane_id();
        let active_col_idx = focused_pid
            .and_then(|pid| {
                cols.iter()
                    .find(|c| c.panes.contains(&pid))
                    .map(|c| c.index)
            })
            .unwrap_or(0);

        let mut toggle_queue = false;

        egui::Panel::top("zoom_path_bar")
            .exact_size(style::PATH_BAR_H)
            .frame(
                egui::Frame::NONE
                    .fill(colors.bg_darkest)
                    .inner_margin(egui::Margin::symmetric(14, 0)),
            )
            .show_inside(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    // Breadcrumb
                    ui.label(
                        RichText::new(&breadcrumb)
                            .font(crate::ui::theme::font_medium(style::TEXT_CAPTION))
                            .color(colors.text_primary),
                    );

                    // Level badge when zoomed out
                    match self.zoom.level {
                        ZoomLevel::Columns => {}
                        ZoomLevel::SiblingCards => {
                            ui.label(
                                RichText::new("· SIBLING CARDS")
                                    .size(style::TEXT_META)
                                    .color(colors.accent),
                            );
                        }
                        ZoomLevel::TreeAtlas => {
                            ui.label(
                                RichText::new("· TREE ATLAS")
                                    .size(style::TEXT_META)
                                    .color(colors.accent),
                            );
                        }
                    }

                    // Position pips for columns (Niri strip scroll indicator)
                    if self.zoom.level == ZoomLevel::Columns && !cols.is_empty() {
                        ui.add_space(style::SPACE_SM);
                        let offset = self.zoom.strip_offset_for(active_ctx_id);
                        for col in &cols {
                            let is_focused_col = col.index == active_col_idx;
                            let is_visible =
                                cols.len() <= 2 || (col.index >= offset && col.index < offset + 2);
                            let pip_char = if is_focused_col {
                                "●"
                            } else if is_visible {
                                "•"
                            } else {
                                "◦"
                            };
                            let pip_color = if is_focused_col {
                                colors.accent
                            } else if is_visible {
                                colors.text_primary
                            } else {
                                colors.text_dim
                            };
                            ui.label(RichText::new(pip_char).size(11.0).color(pip_color));
                        }
                    }

                    // Transient hint if active
                    if let Some((ref hint, at)) = self.attention_queue.transient_hint {
                        if at.elapsed() < Duration::from_millis(1600) {
                            ui.add_space(style::SPACE_SM);
                            ui.label(
                                RichText::new(hint)
                                    .size(style::TEXT_HINT)
                                    .color(colors.zoom_needs),
                            );
                        }
                    }

                    // Right-aligned calm Attention Queue pill (`◆N · total` + top bar preview)
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let asks = self.attention_queue.ask_count();
                        let failed = self.attention_queue.failed_count();
                        let total = self.attention_queue.total_count();

                        let pill_stroke = if asks > 0 {
                            Stroke::new(1.0_f32, colors.zoom_needs)
                        } else if failed > 0 {
                            Stroke::new(1.0_f32, colors.zoom_failed)
                        } else {
                            Stroke::new(1.0_f32, colors.border)
                        };
                        let pill_fill = if asks > 0 {
                            colors.needs_badge_fill
                        } else if failed > 0 {
                            colors.failed_fill
                        } else {
                            colors.code_bg
                        };

                        let pill_text = if let Some(top) = self.attention_queue.items.first() {
                            let lock_prefix = if top.locked { "🔒 " } else { "" };
                            format!(
                                "{lock_prefix}◆{asks} · {total}  {} {}",
                                top.pane_name, top.summary
                            )
                        } else {
                            "◆0 · Queue calm".to_string()
                        };

                        let resp = egui::Frame::NONE
                            .fill(pill_fill)
                            .stroke(pill_stroke)
                            .corner_radius(style::RADIUS_PILL)
                            .inner_margin(egui::Margin::symmetric(10, 3))
                            .show(ui, |ui| {
                                ui.label(RichText::new(pill_text).size(style::TEXT_HINT).color(
                                    if asks > 0 {
                                        colors.zoom_needs
                                    } else if failed > 0 {
                                        colors.zoom_failed
                                    } else {
                                        colors.text_dim
                                    },
                                ));
                            })
                            .response
                            .interact(Sense::click());
                        if resp.clicked() {
                            toggle_queue = true;
                        }
                    });
                });
            });

        if toggle_queue {
            self.toggle_attention_queue_list();
        }
    }

    /// Render the `ZoomLevel::SiblingCards` view inside the central canvas.
    pub(crate) fn draw_sibling_cards_canvas(&mut self, ui: &mut egui::Ui) {
        self.sync_attention_queue();
        let colors = self.colors;
        let rect = ui.available_rect_before_wrap();
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, CornerRadius::ZERO, colors.bg_darkest);

        let siblings = self.sibling_card_contexts();
        let focused_ctx = self
            .zoom
            .cards_focused_context
            .unwrap_or_else(|| self.router.active().context_id);
        let ghost_ctx = self.zoom.ghost.as_ref().map(|g| g.context_id);
        let bump = self.zoom.active_edge_bump_offset();

        let total_w = (siblings.len() as f32) * style::CARD_W
            + (siblings.len().saturating_sub(1) as f32) * style::CARD_GAP;
        let start_x = (rect.center().x - total_w * 0.5).max(rect.min.x + 24.0);
        let center_y = rect.center().y;

        let mut clicked_ctx: Option<u64> = None;

        for (idx, &cid) in siblings.iter().enumerate() {
            let is_focused = cid == focused_ctx;
            let is_ghost = Some(cid) == ghost_ctx;
            let offset = if is_focused { bump } else { Vec2::ZERO };
            let card_min = Pos2::new(
                start_x + (idx as f32) * (style::CARD_W + style::CARD_GAP) + offset.x,
                center_y - style::CARD_H * 0.5 + offset.y,
            );
            let card_rect = Rect::from_min_size(card_min, Vec2::new(style::CARD_W, style::CARD_H));

            let depth = self
                .router
                .as_slice()
                .iter()
                .find(|c| c.context_id == cid)
                .map(|c| c.depth as usize)
                .unwrap_or(0);
            let fill = colors.depth_tint(depth);
            let asks = self.attention_queue.ask_count_for_context(cid);
            let stroke = if is_focused {
                Stroke::new(2.0_f32, colors.accent)
            } else if asks > 0 {
                Stroke::new(1.5_f32, colors.zoom_needs)
            } else {
                Stroke::new(1.0_f32, colors.border)
            };

            // Flat offset shadow
            let shadow_rect = card_rect.translate(Vec2::new(2.0, 2.0));
            painter.rect_filled(shadow_rect, style::RADIUS_MOTORWAYS, colors.shadow_flat);
            painter.rect_filled(card_rect, style::RADIUS_MOTORWAYS, fill);
            painter.rect_stroke(
                card_rect,
                style::RADIUS_MOTORWAYS,
                stroke,
                egui::StrokeKind::Inside,
            );

            // Header
            let ctx_name = self
                .router
                .as_slice()
                .iter()
                .find(|c| c.context_id == cid)
                .map(|c| c.name.displayed().to_string())
                .unwrap_or_else(|| format!("Context {cid}"));
            painter.text(
                Pos2::new(card_rect.min.x + 14.0, card_rect.min.y + 14.0),
                Align2::LEFT_TOP,
                &ctx_name,
                crate::ui::theme::font_medium(style::TEXT_BODY),
                colors.text_primary,
            );

            if asks > 0 {
                painter.text(
                    Pos2::new(card_rect.max.x - 14.0, card_rect.min.y + 14.0),
                    Align2::RIGHT_TOP,
                    format!("◆{asks}"),
                    crate::ui::theme::font_medium(style::TEXT_CAPTION),
                    colors.zoom_needs,
                );
            }

            if is_ghost {
                painter.text(
                    Pos2::new(card_rect.min.x + 14.0, card_rect.max.y - 16.0),
                    Align2::LEFT_BOTTOM,
                    "◌ you were here",
                    FontId::new(style::TEXT_HINT, FontFamily::Proportional),
                    colors.accent,
                );
            }

            // Mini column tiles inside the card
            let cols = self.columns_for_context(cid);
            let inner_top = card_rect.min.y + 42.0;
            let inner_h = (card_rect.height() - 72.0).max(40.0);
            let inner_w = card_rect.width() - 28.0;
            let col_count = cols.len().max(1);
            let col_w = (inner_w - ((col_count - 1) as f32) * 6.0) / (col_count as f32);

            for (c_idx, col) in cols.iter().enumerate() {
                let cx = card_rect.min.x + 14.0 + (c_idx as f32) * (col_w + 6.0);
                let stack_len = col.panes.len().max(1);
                let ph = (inner_h - ((stack_len - 1) as f32) * 4.0) / (stack_len as f32);
                for (r_idx, &pid) in col.panes.iter().enumerate() {
                    let py = inner_top + (r_idx as f32) * (ph + 4.0);
                    let mini_rect = Rect::from_min_size(Pos2::new(cx, py), Vec2::new(col_w, ph));
                    let has_ask = self.attention_queue.first_ask_for_pane(pid).is_some();
                    painter.rect_filled(mini_rect, style::RADIUS_SM, colors.tile_bg);
                    painter.rect_stroke(
                        mini_rect,
                        style::RADIUS_SM,
                        Stroke::new(
                            1.0_f32,
                            if has_ask {
                                colors.zoom_needs
                            } else {
                                colors.border
                            },
                        ),
                        egui::StrokeKind::Inside,
                    );
                    let (_, pname, _) = self.describe_pane_for_queue(pid);
                    painter.text(
                        Pos2::new(mini_rect.min.x + 6.0, mini_rect.min.y + 6.0),
                        Align2::LEFT_TOP,
                        pname,
                        FontId::new(style::TEXT_META, FontFamily::Monospace),
                        colors.text_dim,
                    );
                }
            }

            let resp = ui.interact(
                card_rect,
                egui::Id::new(("sibling_card", cid)),
                Sense::click(),
            );
            if resp.clicked() {
                clicked_ctx = Some(cid);
            }
        }

        // Bottom key legend
        painter.text(
            Pos2::new(rect.center().x, rect.max.y - 16.0),
            Align2::CENTER_BOTTOM,
            "h/l move card · Enter enter context · Cmd+Esc tree atlas · Cmd+Enter back to ghost",
            FontId::new(style::TEXT_HINT, FontFamily::Proportional),
            colors.text_dim,
        );

        if let Some(cid) = clicked_ctx {
            self.zoom.cards_focused_context = Some(cid);
            self.enter_focused_sibling_card();
        }
    }

    /// Render the `ZoomLevel::TreeAtlas` view inside the central canvas (Mini Motorways style).
    pub(crate) fn draw_tree_atlas_canvas(&mut self, ui: &mut egui::Ui) {
        self.sync_attention_queue();
        let colors = self.colors;
        let rect = ui.available_rect_before_wrap();
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, CornerRadius::ZERO, colors.bg_darkest);

        let (nodes, edges) = self.compute_atlas_graph(rect);
        let focused_ctx = self
            .zoom
            .atlas_focused_context
            .unwrap_or_else(|| self.router.active().context_id);
        let bump = self.zoom.active_edge_bump_offset();

        // 1. Draw Motorways road edges (casing + road stroke).
        for edge in &edges {
            let stroke = match edge.kind {
                AtlasEdgeKind::NeedsRollup => Stroke::new(style::EDGE_NEEDS_W, colors.zoom_needs),
                AtlasEdgeKind::Failed => Stroke::new(style::EDGE_ROAD_W, colors.zoom_failed),
                AtlasEdgeKind::PipeLink => Stroke::new(1.5_f32, colors.accent),
                AtlasEdgeKind::Spawn => Stroke::new(1.5_f32, colors.text_dim),
                AtlasEdgeKind::TreeRoad => Stroke::new(style::EDGE_ROAD_W, colors.border),
            };
            let casing = Stroke::new(stroke.width + 3.0_f32, colors.shadow_flat);
            painter.line_segment([edge.from_pos, edge.to_pos], casing);
            painter.line_segment([edge.from_pos, edge.to_pos], stroke);
        }

        let mut clicked_node: Option<u64> = None;

        // 2. Draw context nodes with depth tinting (0..4 clamped).
        for node in &nodes {
            if !rect.intersects(node.rect.expand(40.0)) {
                continue; // Viewport culling for 50+ nodes (`SPEC.md` Q7).
            }
            let is_focused = node.context_id == focused_ctx;
            let nrect = if is_focused {
                node.rect.translate(bump)
            } else {
                node.rect
            };

            let fill = colors.depth_tint(node.depth);
            let stroke = if is_focused {
                Stroke::new(2.0_f32, colors.accent)
            } else if node.direct_asks > 0 || node.ask_rollup > 0 {
                Stroke::new(1.5_f32, colors.zoom_needs)
            } else if node.failed_count > 0 {
                Stroke::new(1.5_f32, colors.zoom_failed)
            } else {
                Stroke::new(1.0_f32, colors.border)
            };

            let shadow_rect = nrect.translate(Vec2::new(2.0, 2.0));
            painter.rect_filled(shadow_rect, style::RADIUS_MOTORWAYS, colors.shadow_flat);
            painter.rect_filled(nrect, style::RADIUS_MOTORWAYS, fill);
            painter.rect_stroke(
                nrect,
                style::RADIUS_MOTORWAYS,
                stroke,
                egui::StrokeKind::Inside,
            );

            // Title + rollup badge
            painter.text(
                Pos2::new(nrect.min.x + 12.0, nrect.min.y + 10.0),
                Align2::LEFT_TOP,
                &node.name,
                crate::ui::theme::font_medium(style::TEXT_CAPTION),
                colors.text_primary,
            );

            if node.direct_asks > 0 {
                painter.text(
                    Pos2::new(nrect.max.x - 12.0, nrect.min.y + 10.0),
                    Align2::RIGHT_TOP,
                    format!("◆{}", node.direct_asks),
                    crate::ui::theme::font_medium(style::TEXT_CAPTION),
                    colors.zoom_needs,
                );
            } else if node.failed_count > 0 {
                painter.text(
                    Pos2::new(nrect.max.x - 12.0, nrect.min.y + 10.0),
                    Align2::RIGHT_TOP,
                    format!("✕{}", node.failed_count),
                    crate::ui::theme::font_medium(style::TEXT_CAPTION),
                    colors.zoom_failed,
                );
            }

            // Preview lines based on tile size tier
            let tier = TileContentTier::from_size(
                nrect.size(),
                node.direct_asks > 0 || node.failed_count > 0,
                None,
            );
            if matches!(
                tier,
                TileContentTier::Lines5 | TileContentTier::Lines3 | TileContentTier::Line1
            ) {
                for (l_idx, line) in node.preview_lines.iter().take(2).enumerate() {
                    painter.text(
                        Pos2::new(
                            nrect.min.x + 12.0,
                            nrect.min.y + 32.0 + (l_idx as f32) * 16.0,
                        ),
                        Align2::LEFT_TOP,
                        line,
                        FontId::new(style::TEXT_HINT, FontFamily::Monospace),
                        if line.starts_with('◆') {
                            colors.zoom_needs
                        } else {
                            colors.text_dim
                        },
                    );
                }
            }

            if node.is_ghost {
                painter.text(
                    Pos2::new(nrect.max.x - 12.0, nrect.max.y - 8.0),
                    Align2::RIGHT_BOTTOM,
                    "◌ here",
                    FontId::new(style::TEXT_META, FontFamily::Proportional),
                    colors.accent,
                );
            }

            let resp = ui.interact(
                nrect,
                egui::Id::new(("atlas_node", node.context_id)),
                Sense::click(),
            );
            if resp.clicked() {
                clicked_node = Some(node.context_id);
            }
        }

        // 3. Read-only Peek card (`p`) when open — follows focused node (`SPEC.md` line 24).
        if self.zoom.atlas_peek_open {
            if let Some(fnode) = nodes.iter().find(|n| n.context_id == focused_ctx) {
                let peek_w = style::PEEK_CARD_W;
                let peek_h = 220.0;
                let peek_rect = Rect::from_min_size(
                    Pos2::new(rect.max.x - peek_w - 20.0, rect.min.y + 20.0),
                    Vec2::new(peek_w, peek_h),
                );
                painter.rect_filled(
                    peek_rect.translate(Vec2::new(2.0, 2.0)),
                    style::RADIUS_MOTORWAYS,
                    colors.shadow_flat,
                );
                painter.rect_filled(peek_rect, style::RADIUS_MOTORWAYS, colors.terminal_bg);
                painter.rect_stroke(
                    peek_rect,
                    style::RADIUS_MOTORWAYS,
                    Stroke::new(1.5_f32, colors.accent),
                    egui::StrokeKind::Inside,
                );
                painter.text(
                    Pos2::new(peek_rect.min.x + 14.0, peek_rect.min.y + 12.0),
                    Align2::LEFT_TOP,
                    format!("PEEK · {} (read-only)", fnode.name),
                    crate::ui::theme::font_medium(style::TEXT_CAPTION),
                    colors.accent,
                );
                let lines = self.context_preview_lines(fnode.context_id, 6);
                for (idx, line) in lines.iter().enumerate() {
                    painter.text(
                        Pos2::new(
                            peek_rect.min.x + 14.0,
                            peek_rect.min.y + 38.0 + (idx as f32) * 18.0,
                        ),
                        Align2::LEFT_TOP,
                        line,
                        FontId::new(style::TEXT_HINT, FontFamily::Monospace),
                        colors.text_primary,
                    );
                }
            }
        }

        // Bottom key legend
        painter.text(
            Pos2::new(rect.center().x, rect.max.y - 16.0),
            Align2::CENTER_BOTTOM,
            "h parent · l child · j/k same depth · [/] sibling · n/N needs you · p peek · Enter dive · q back to ghost",
            FontId::new(style::TEXT_HINT, FontFamily::Proportional),
            colors.text_dim,
        );

        if let Some(cid) = clicked_node {
            self.zoom.atlas_focused_context = Some(cid);
            self.dive_from_tree_atlas();
        }
    }
}

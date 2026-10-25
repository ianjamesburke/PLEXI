//! Zoom Ladder, Niri-Style Scrolling Column Strip, Sibling Context Cards, and Tree Atlas
//! (`SPEC.md` R31–R36, R40, R42).

use crate::app::app_trait::KeyDisposition;
use crate::app::attention_queue::QueueCategory;
use crate::app::input_router::PlexiInput;
use crate::app::PlexiApp;
use crate::host::keys::Direction;
use crate::spatial::tiling::PaneId;
use crate::ui::card_block::{layout_card, paint_card, wrap_text, CardForm, CardSpec, CardState};
use crate::ui::style;
use egui::{
    Align2, CornerRadius, FontFamily, FontId, Key, Pos2, Rect, RichText, Sense, Stroke, Vec2,
};
use egui_tiles::{Container, LinearDir, Tile, TileId};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

/// Minimum height under a sibling card's header block for the mini column tiles.
const SIBLING_BODY_MIN_H: f32 = 120.0;

/// The three zoom levels on the ladder (`SPEC.md` § Levels).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum ZoomLevel {
    /// Default level inside a context: Niri-style scrolling column strip.
    /// The ONLY level where a pane takes keyboard input.
    #[default]
    Columns,
    /// Current context plus its siblings (same `parent_id`), in creation order.
    SiblingCards,
    /// Whole-workspace context graph: top-down tidy tree with Motorways road edges.
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
#[derive(Debug, Clone, PartialEq)]
pub struct StripColumn {
    pub index: usize,
    pub context_id: u64,
    pub panes: Vec<PaneId>,
    pub preset: ColumnWidthPreset,
    pub width: crate::strip::ColumnWidth,
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

/// Measured geometry for the origin marker ("you were here" circle + label).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OriginMarkerGeometry {
    pub circle_center: Pos2,
    pub circle_radius: f32,
    pub circle_stroke_width: f32,
    pub text_pos: Pos2,
    pub text_size: Vec2,
    pub total_rect: Rect,
}

pub const ORIGIN_MARKER_TEXT: &str = "you were here";
pub const ORIGIN_MARKER_CIRCLE_RADIUS: f32 = 3.5;
pub const ORIGIN_MARKER_CIRCLE_STROKE: f32 = 1.2;
pub const ORIGIN_MARKER_GAP: f32 = 6.0;

/// Paints the origin marker ("you were here" circle + text) using shared measured geometry.
/// `anchor_bottom_left` is the bottom-left anchor position within its container.
pub fn paint_origin_marker(
    painter: &egui::Painter,
    anchor_bottom_left: Pos2,
    color: egui::Color32,
) -> OriginMarkerGeometry {
    let font = FontId::new(style::TEXT_HINT, FontFamily::Proportional);
    let galley = painter.layout_no_wrap(ORIGIN_MARKER_TEXT.to_string(), font, color);
    let text_size = galley.size();
    let text_h = text_size.y;
    let circle_radius = ORIGIN_MARKER_CIRCLE_RADIUS;
    let circle_stroke_width = ORIGIN_MARKER_CIRCLE_STROKE;
    let gap = ORIGIN_MARKER_GAP;

    let top_y = anchor_bottom_left.y - text_h;
    let mid_y = top_y + text_h * 0.5;

    let circle_center = Pos2::new(anchor_bottom_left.x + circle_radius, mid_y);
    let text_pos = Pos2::new(anchor_bottom_left.x + circle_radius * 2.0 + gap, top_y);

    painter.circle_stroke(
        circle_center,
        circle_radius,
        Stroke::new(circle_stroke_width, color),
    );
    painter.galley(text_pos, galley, color);

    let total_w = circle_radius * 2.0 + gap + text_size.x;
    let total_h = text_h.max(circle_radius * 2.0);
    let total_rect = Rect::from_min_size(
        Pos2::new(anchor_bottom_left.x, top_y.min(mid_y - circle_radius)),
        Vec2::new(total_w, total_h),
    );

    OriginMarkerGeometry {
        circle_center,
        circle_radius,
        circle_stroke_width,
        text_pos,
        text_size,
        total_rect,
    }
}

/// The active window's column strip in window-local indices (`App::active_window_strip`).
#[derive(Debug, Clone)]
pub(crate) struct WindowStrip {
    /// Root `Linear` children, left to right.
    pub children: Vec<TileId>,
    /// Authoritative width definition of each child's global column.
    pub widths: Vec<Option<crate::strip::ColumnWidth>>,
    /// First of the two visible columns (always 0 when there are two or fewer).
    pub lo: usize,
}

/// Root `Linear` shares, in pixels, for a strip of `ratios.len()` columns whose
/// visible pair starts at `lo` (`egui_tiles` normalizes shares by their sum, so
/// pixel shares reproduce exactly these widths when `canvas_width` is the tree's
/// width).
///
/// With more than two columns, each clipped group (left of `lo`, right of
/// `lo + 2`) spans `STRIP_PEEK_W` plus the gap to its visible neighbour, slivers
/// and inter-sliver gaps included, so the peek strip painted over the canvas
/// edge covers the slivers exactly and the visible pair keeps the rest at preset
/// ratio. A group of many columns holds a per-sliver floor instead, spilling
/// past the strip but never into a visible pane.
pub(crate) fn strip_shares(ratios: &[f32], lo: usize, gap: f32, canvas_width: f32) -> Vec<f32> {
    let n = ratios.len();
    if n <= 2 {
        return ratios.iter().map(|r| r * 2.0).collect();
    }
    let left = lo;
    let right = n - lo - 2;
    let group_total = |count: usize| -> f32 {
        if count == 0 {
            0.0
        } else {
            (style::STRIP_PEEK_W + gap - count as f32 * gap)
                .max(count as f32 * style::STRIP_MIN_SLIVER_W)
        }
    };
    let (left_total, right_total) = (group_total(left), group_total(right));
    let available = (canvas_width - gap * (n - 1) as f32).max(0.0);
    let visible_total = (available - left_total - right_total).max(2.0);
    let ratio_sum = (ratios[lo] + ratios[lo + 1]).max(f32::EPSILON);
    ratios
        .iter()
        .enumerate()
        .map(|(idx, ratio)| {
            if idx < lo {
                left_total / left as f32
            } else if idx >= lo + 2 {
                right_total / right as f32
            } else {
                (visible_total * ratio / ratio_sum).max(1.0)
            }
        })
        .collect()
}

/// Geometry of the left and right `STRIP_PEEK_W` clipped-neighbor strips at
/// `ZoomLevel::Columns`. Matches the top and bottom `y` of `frame_outer` (the
/// focused pane's outer focus-frame rect, falling back to `canvas_rect` when
/// absent) and clamps horizontally to `canvas_rect` so the strips never leave
/// the tree canvas.
pub(crate) fn peek_strip_rects(
    canvas_rect: Rect,
    frame_outer: Option<Rect>,
    has_left: bool,
    has_right: bool,
) -> (Option<Rect>, Option<Rect>) {
    let (top, bottom) = match frame_outer {
        Some(frame) => (frame.min.y, frame.max.y),
        None => (canvas_rect.min.y, canvas_rect.max.y),
    };
    let clamp_x = |min_x: f32, max_x: f32| -> Rect {
        let x0 = min_x.clamp(canvas_rect.min.x, canvas_rect.max.x);
        let x1 = max_x.clamp(x0, canvas_rect.max.x);
        Rect::from_min_max(Pos2::new(x0, top), Pos2::new(x1, bottom))
    };
    let left =
        has_left.then(|| clamp_x(canvas_rect.min.x, canvas_rect.min.x + style::STRIP_PEEK_W));
    let right =
        has_right.then(|| clamp_x(canvas_rect.max.x - style::STRIP_PEEK_W, canvas_rect.max.x));
    (left, right)
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
    /// Optional authoritative column widths per context (`(context_id, col_idx) -> ColumnWidth`).
    pub column_widths: HashMap<(u64, usize), crate::strip::ColumnWidth>,
    /// Focused context ID while at `ZoomLevel::SiblingCards`.
    pub cards_focused_context: Option<u64>,
    /// Explicit horizontal viewport scroll offset (center X in content coordinates) for `ZoomLevel::SiblingCards`.
    pub cards_viewport_offset: f32,
    /// Last context ID for which cards viewport offset was synchronized with focus.
    pub cards_last_focused_context: Option<u64>,
    /// Focused context ID while at `ZoomLevel::TreeAtlas`.
    pub atlas_focused_context: Option<u64>,
    /// Per-parent context last-visited child context in the atlas (`j` / `k` child memory).
    pub atlas_last_child: HashMap<u64, u64>,
    /// True when the read-only Peek card (`p`) is open in the Tree Atlas.
    pub atlas_peek_open: bool,
    /// Directed context-to-context links (`(from_ctx, to_ctx)`) for `f`/`F` traversal in atlas.
    pub context_links: Vec<(u64, u64)>,
    /// Monotonic counter of edge attempts (observable by tests for R31/R32 only).
    pub edge_bump_count: u64,
    /// Retargetable zoom transition state (`(from_progress, target_progress, started_at)`).
    pub transition: Option<(f32, f32, Instant)>,
    /// Buffered text/key events typed during a zoom-in flight to `Columns`.
    pub flight_key_buffer: Vec<egui::Event>,
    /// `(window_id, lo, root children)` of the strip painted last frame; a change
    /// is a discrete layout jump (see `strip_layout_jump`).
    pub last_strip_sig: Option<(u64, usize, Vec<TileId>)>,
    /// True on the frame the active window's strip signature changed. The tiles
    /// jump to their new rects at once, so terminals commit the new grid
    /// immediately instead of debouncing it like a divider drag.
    pub strip_layout_jump: bool,
    /// Panes of the columns outside the two visible ones: clipped slivers that
    /// paint a placeholder and never resize their terminal.
    pub off_strip_panes: HashSet<PaneId>,
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
    /// Record an edge attempt in `dir` without changing layout or starting an animation.
    pub fn record_edge_bump(&mut self, dir: Direction) {
        self.zoom.edge_bump_count += 1;
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
                let width = self
                    .zoom
                    .column_widths
                    .get(&(context_id, idx))
                    .copied()
                    .unwrap_or(crate::strip::ColumnWidth::Preset(preset));
                StripColumn {
                    index: idx,
                    context_id: col_ctx_id,
                    panes,
                    preset,
                    width,
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
        // A focus move must clear zoom before navigate_to changes focused_pane.
        // Otherwise the render pass can observe a zoomed tile that is no longer
        // focused between this helper and its caller's cleanup.
        if self.active_focused_pane_id() != Some(pane_id) {
            self.windows[self.active_window].clear_zoom();
        }
        if self.windows[win_idx].zoomed_pane != Some(tile_id) {
            self.windows[win_idx].clear_zoom();
        }
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
    /// at `ZoomLevel::Columns`. Stops at edges without wrapping (`SPEC.md` R32).
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
                self.zoom.cards_last_focused_context = None;
                self.zoom.cards_viewport_offset = 0.0;
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

    /// Toggle focused pane fullscreen zoom within Columns (`SPEC.md` R31).
    ///
    /// Preserves focus, scroll and CLI zoom semantics:
    /// - If the focused pane is a Portal tile, zooms into its sub-context.
    /// - If any pane is already zoomed in the active window, clears zoom and
    ///   restores prior columns geometry, focus and scroll.
    /// - If the active window has only one pane, zoom is ignored and an edge bump
    ///   is recorded.
    /// - Otherwise, zooms the focused pane to fullscreen.
    pub fn toggle_columns_pane_zoom(&mut self) -> bool {
        if let Some(parent) = self.fleet_focused_pane() {
            if self.dive_into_agent_children(parent) {
                return true;
            }
        }
        let (focused_tile, portal_target) = {
            let win = &self.windows[self.active_window];
            let tile = win.focused_pane;
            let target = tile
                .and_then(|tile_id| win.tree.tiles.get(tile_id))
                .and_then(|t| {
                    if let egui_tiles::Tile::Pane(p) = t {
                        Some(*p)
                    } else {
                        None
                    }
                })
                .and_then(|pane_id| win.panes.get(&pane_id))
                .and_then(|p| p.portal_target());
            (tile, target)
        };
        if let Some(child_ctx_id) = portal_target {
            if let Some(ctx_idx) = self.router.position(|c| c.context_id == child_ctx_id) {
                log::info!("zoom: toggle on Portal diving into context_id={child_ctx_id}");
                let current_ctx_id = self.router.active().context_id;
                let current_win_id = self.windows[self.active_window].window_id;
                self.router
                    .push_depth(current_ctx_id, current_win_id, focused_tile);
                self.switch_workspace(ctx_idx);
                true
            } else {
                log::warn!(
                    "zoom: Portal target context_id={child_ctx_id} not found in router"
                );
                false
            }
        } else {
            let win = &self.windows[self.active_window];
            let zoomed = win.zoomed_pane;
            let focused = win.focused_pane;
            let pane_count = win.panes.len();
            let window_id = win.window_id;

            if let Some(prev) = zoomed {
                self.windows[self.active_window].clear_zoom();
                log::info!("zoom: toggle off — pane={prev:?}");
                true
            } else if let Some(focused) = focused {
                if pane_count <= 1 {
                    self.record_edge_bump(Direction::Down);
                    log::info!(
                        "zoom: toggle ignored — pane={focused:?} is the only pane in window={window_id}",
                    );
                    false
                } else {
                    self.windows[self.active_window].zoom_to(focused);
                    log::info!("zoom: toggle on — pane={focused:?}");
                    true
                }
            } else {
                self.record_edge_bump(Direction::Down);
                false
            }
        }
    }

    /// `Cmd+Enter`:
    /// Zooms in one level back toward the ghost (`TreeAtlas -> SiblingCards -> Columns`).
    /// At `Columns`: toggles fullscreen zoom of the focused pane.
    pub fn zoom_in_level(&mut self) -> bool {
        match self.zoom.level {
            ZoomLevel::Columns => self.toggle_columns_pane_zoom(),
            ZoomLevel::SiblingCards => {
                let selected = self.zoom.cards_focused_context;
                let ghost_ctx = self.zoom.ghost.as_ref().map(|g| g.context_id);
                if selected.is_none() || selected == ghost_ctx {
                    self.restore_ghost_or_active_columns();
                } else {
                    // Selection moved off the origin: same path as bare Enter.
                    self.enter_focused_sibling_card();
                }
                log::info!("zoom: SiblingCards -> Columns (selected={selected:?})");
                true
            }
            ZoomLevel::TreeAtlas => {
                self.zoom.atlas_peek_open = false;
                let target_ctx = self
                    .zoom
                    .atlas_focused_context
                    .or_else(|| self.zoom.ghost.as_ref().map(|g| g.context_id))
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
                self.zoom.cards_last_focused_context = None;
                self.zoom.cards_viewport_offset = 0.0;
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

        let new_idx = match dir {
            Direction::Left => {
                if cur_idx == 0 {
                    self.record_edge_bump(Direction::Left);
                    return false;
                }
                cur_idx - 1
            }
            Direction::Right => {
                if cur_idx + 1 >= siblings.len() {
                    self.record_edge_bump(Direction::Right);
                    return false;
                }
                cur_idx + 1
            }
            Direction::Up | Direction::Down => {
                self.record_edge_bump(dir);
                return false;
            }
        };

        let target_ctx = siblings[new_idx];
        self.zoom.cards_focused_context = Some(target_ctx);
        self.zoom.cards_viewport_offset =
            (new_idx as f32) * (style::CARD_W + style::CARD_GAP) + style::CARD_W * 0.5;
        self.zoom.cards_last_focused_context = Some(target_ctx);
        log::info!(
            "zoom: sibling cards navigate dir={dir:?} target_ctx={target_ctx} idx={new_idx} offset={:.1}",
            self.zoom.cards_viewport_offset
        );
        true
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
        log::info!("zoom: enter sibling card target_ctx={target_ctx}");
        true
    }

    /// Bounding rect of the top-center virtual hub pill shown when `ZoomLevel::TreeAtlas`
    /// has multiple root contexts.
    pub fn atlas_hub_rect(viewport: Rect) -> Rect {
        Rect::from_center_size(
            Pos2::new(viewport.center().x, viewport.min.y + 44.0),
            Vec2::new(136.0, 28.0),
        )
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

        // Top-down tree layout:
        // - Single root: root sits at top-center, children on rows below.
        // - Multiple roots: visible hub pill sits at top-center (`atlas_hub_rect`),
        //   roots sit on row 1 below the hub, and children sit on rows below their parent.
        let hub = roots.len() > 1;
        let hub_rect = Self::atlas_hub_rect(viewport);
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
        let root_row_y = if hub {
            hub_rect.max.y + 48.0 + style::ATLAS_NODE_H * 0.5
        } else {
            viewport.min.y + 48.0 + style::ATLAS_NODE_H * 0.5
        };
        let row_step_y = if deepest == 0 {
            style::ATLAS_NODE_H + 56.0
        } else {
            let avail_h =
                (viewport.max.y - 64.0 - root_row_y - style::ATLAS_NODE_H * 0.5).max(style::ATLAS_NODE_H);
            (avail_h / deepest as f32).clamp(style::ATLAS_NODE_H + 36.0, style::ATLAS_NODE_H * 1.85)
        };

        let total_leaves: usize = roots
            .iter()
            .map(|&r| leaves(r, &children_of))
            .sum::<usize>()
            .max(1);
        let slot_w = style::ATLAS_NODE_W + 48.0;
        let band_w = (total_leaves as f32 * slot_w)
            .min((viewport.width() - 48.0).max(style::ATLAS_NODE_W));
        let band_x0 = viewport.center().x - band_w * 0.5;
        // Narrow logical viewports (high UI scale) shrink nodes to their slot
        // rather than letting siblings overlap.
        let node_w = (band_w / total_leaves as f32 - 12.0).clamp(96.0, style::ATLAS_NODE_W);

        let mut nodes_out: Vec<AtlasNode> = Vec::new();
        struct Placement {
            ctx_id: u64,
            depth: usize,
            x0: f32,
            x1: f32,
        }
        let mut stack: Vec<Placement> = Vec::new();
        {
            let mut x = band_x0;
            let mut placements = Vec::new();
            for &r in &roots {
                let span = band_w * leaves(r, &children_of) as f32 / total_leaves as f32;
                placements.push(Placement {
                    ctx_id: r,
                    depth: 0,
                    x0: x,
                    x1: x + span,
                });
                x += span;
            }
            placements.reverse();
            stack.extend(placements);
        }
        while let Some(pl) = stack.pop() {
            let ctx_id = pl.ctx_id;
            let node_center = Pos2::new(
                (pl.x0 + pl.x1) * 0.5,
                root_row_y + (pl.depth as f32) * row_step_y,
            );
            let rect = Rect::from_center_size(
                node_center,
                Vec2::new(node_w, style::ATLAS_NODE_H),
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
            let mut x = pl.x0;
            let mut kid_placements = Vec::new();
            for &k in &kids {
                let span = (pl.x1 - pl.x0) * leaves(k, &children_of) as f32 / total as f32;
                kid_placements.push(Placement {
                    ctx_id: k,
                    depth: pl.depth + 1,
                    x0: x,
                    x1: x + span,
                });
                x += span;
            }
            kid_placements.reverse();
            stack.extend(kid_placements);
        }

        let mut edges_out: Vec<AtlasEdge> = Vec::new();
        // Parent/child context relationships (top-down: parent bottom-center -> child top-center).
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
                        from_pos: Pos2::new(parent_node.rect.center().x, parent_node.rect.max.y),
                        to_pos: Pos2::new(node.rect.center().x, node.rect.min.y),
                        kind,
                    });
                }
            }
        }
        // Virtual hub: with several roots, each root hangs below the top-center hub pill.
        if hub {
            let hub_bottom = Pos2::new(hub_rect.center().x, hub_rect.max.y);
            for node in nodes_out.iter().filter(|n| n.parent_id.is_none()) {
                edges_out.push(AtlasEdge {
                    from_context_id: 0,
                    to_context_id: node.context_id,
                    from_pos: hub_bottom,
                    to_pos: Pos2::new(node.rect.center().x, node.rect.min.y),
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

    /// What a context's card should look like: its own asks first, then any
    /// failure, then live agent activity, else a quiet shell.
    pub(crate) fn context_card_state(&self, context_id: u64) -> CardState {
        use crate::protocol::AgentState;
        if self.attention_queue.ask_count_for_context(context_id) > 0 {
            return CardState::NeedsYou;
        }
        let mut state = CardState::Idle;
        for win in self.windows.iter().filter(|w| w.context_id == context_id) {
            for pane in win.panes.values() {
                match pane.agent().map(|a| &a.state) {
                    Some(&AgentState::Failed) => return CardState::Failed,
                    Some(&AgentState::Working) => state = CardState::Working,
                    Some(&AgentState::Completed) if state == CardState::Idle => {
                        state = CardState::Done
                    }
                    _ => {}
                }
            }
        }
        state
    }

    /// One-sentence body for a context card: its first ask, else the first
    /// agent's live activity.
    pub(crate) fn context_card_detail(&self, context_id: u64) -> String {
        self.context_preview_lines(context_id, 1)
            .into_iter()
            .next()
            .map(|l| l.trim_start_matches("◆ ").to_string())
            .unwrap_or_default()
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

    /// Move in the Tree Atlas along `k` (`Up` = parent), `j` (`Down` = last-visited/first child),
    /// `h` (`Left` = previous neighboring sibling), `l` (`Right` = next neighboring sibling).
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
            .or_else(|| {
                nodes
                    .iter()
                    .find(|n| n.context_id == self.router.active().context_id)
                    .map(|n| n.context_id)
            })
            .or_else(|| nodes.first().map(|n| n.context_id))
            .unwrap_or_else(|| self.router.active().context_id);
        let Some(cur_node) = nodes.iter().find(|n| n.context_id == cur_ctx) else {
            self.record_edge_bump(dir);
            return false;
        };

        match dir {
            Direction::Up => {
                // `k` / `ArrowUp`: Parent context. Root boundary if none.
                let Some(parent_id) = cur_node.parent_id else {
                    self.record_edge_bump(Direction::Up);
                    return false;
                };
                if !nodes.iter().any(|n| n.context_id == parent_id) {
                    self.record_edge_bump(Direction::Up);
                    return false;
                }
                self.zoom.atlas_last_child.insert(parent_id, cur_ctx);
                self.zoom.atlas_focused_context = Some(parent_id);
                log::info!("zoom: atlas nav up parent target_ctx={parent_id}");
                true
            }
            Direction::Down => {
                // `j` / `ArrowDown`: Last-visited (else first/leftmost) child context. Leaf boundary if none.
                let mut children: Vec<&AtlasNode> = nodes
                    .iter()
                    .filter(|n| n.parent_id == Some(cur_ctx))
                    .collect();
                if children.is_empty() {
                    self.record_edge_bump(Direction::Down);
                    return false;
                }
                children.sort_by(|a, b| a.rect.center().x.total_cmp(&b.rect.center().x));
                let target_child = self
                    .zoom
                    .atlas_last_child
                    .get(&cur_ctx)
                    .copied()
                    .filter(|cid| children.iter().any(|c| c.context_id == *cid))
                    .unwrap_or(children[0].context_id);
                self.zoom.atlas_last_child.insert(cur_ctx, target_child);
                self.zoom.atlas_focused_context = Some(target_child);
                log::info!("zoom: atlas nav down child target_ctx={target_child}");
                true
            }
            Direction::Left => {
                // `h` / `ArrowLeft`: Left neighboring sibling (same parent_id, ordered by x). Boundary if none.
                let mut siblings: Vec<&AtlasNode> = nodes
                    .iter()
                    .filter(|n| n.parent_id == cur_node.parent_id)
                    .collect();
                siblings.sort_by(|a, b| a.rect.center().x.total_cmp(&b.rect.center().x));
                let pos = siblings
                    .iter()
                    .position(|n| n.context_id == cur_ctx)
                    .unwrap_or(0);
                if pos == 0 {
                    self.record_edge_bump(Direction::Left);
                    return false;
                }
                let target_node = siblings[pos - 1];
                let target_ctx = target_node.context_id;
                if let Some(pid) = cur_node.parent_id {
                    self.zoom.atlas_last_child.insert(pid, target_ctx);
                }
                self.zoom.atlas_focused_context = Some(target_ctx);
                log::info!("zoom: atlas nav left sibling target_ctx={target_ctx}");
                true
            }
            Direction::Right => {
                // `l` / `ArrowRight`: Right neighboring sibling (same parent_id, ordered by x). Boundary if none.
                let mut siblings: Vec<&AtlasNode> = nodes
                    .iter()
                    .filter(|n| n.parent_id == cur_node.parent_id)
                    .collect();
                siblings.sort_by(|a, b| a.rect.center().x.total_cmp(&b.rect.center().x));
                let pos = siblings
                    .iter()
                    .position(|n| n.context_id == cur_ctx)
                    .unwrap_or(0);
                if pos + 1 >= siblings.len() {
                    self.record_edge_bump(Direction::Right);
                    return false;
                }
                let target_node = siblings[pos + 1];
                let target_ctx = target_node.context_id;
                if let Some(pid) = cur_node.parent_id {
                    self.zoom.atlas_last_child.insert(pid, target_ctx);
                }
                self.zoom.atlas_focused_context = Some(target_ctx);
                log::info!("zoom: atlas nav right sibling target_ctx={target_ctx}");
                true
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
            .filter(|id| nodes.iter().any(|n| n.context_id == *id))
            .or_else(|| {
                nodes
                    .iter()
                    .find(|n| n.context_id == self.router.active().context_id)
                    .map(|n| n.context_id)
            })
            .or_else(|| nodes.first().map(|n| n.context_id))
            .unwrap_or_else(|| self.router.active().context_id);
        let Some(cur_node) = nodes.iter().find(|n| n.context_id == cur_ctx) else {
            return false;
        };
        let mut siblings: Vec<&AtlasNode> = nodes
            .iter()
            .filter(|n| n.parent_id == cur_node.parent_id)
            .collect();
        siblings.sort_by(|a, b| a.rect.center().x.total_cmp(&b.rect.center().x));
        if siblings.len() <= 1 {
            self.record_edge_bump(if forward {
                Direction::Right
            } else {
                Direction::Left
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
        log::info!("zoom: atlas nav sibling forward={forward} target_ctx={next_ctx}");
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

    /// Strip view of the ACTIVE window's tree, in the window's own column indices.
    ///
    /// A context can hold several windows (pages), each with its own tree, while
    /// `columns_for_context` and `strip_offset_for` count columns across ALL of them.
    /// Painting shares or clip indicators straight from those global indices starves
    /// the columns of whichever window is active (the first window after a move back
    /// from a later one rendered as 6% slivers next to one oversized pane). This is the
    /// single place that maps global columns onto the active window's root children.
    pub(crate) fn active_window_strip(&self) -> Option<WindowStrip> {
        let ctx_id = self.router.active().context_id;
        let cols = self.columns_for_context(ctx_id);
        let offset = self.zoom.strip_offset_for(ctx_id);
        let win = self.windows.get(self.active_window)?;
        let root_id = win.tree.root()?;
        let Some(Tile::Container(Container::Linear(lin))) = win.tree.tiles.get(root_id) else {
            return None;
        };
        if lin.dir != LinearDir::Horizontal || lin.children.len() < 2 {
            return None;
        }
        let focused = self.active_focused_pane_id();
        let mut children = Vec::with_capacity(lin.children.len());
        let mut global = Vec::with_capacity(lin.children.len());
        let mut focused_local = None;
        for (idx, &child) in lin.children.iter().enumerate() {
            let mut leaves = Vec::new();
            collect_leaf_panes_in_tile(&win.tree, &win.panes, child, &mut leaves);
            if focused.is_some_and(|f| leaves.contains(&f)) {
                focused_local = Some(idx);
            }
            global.push(
                cols.iter()
                    .position(|c| leaves.iter().any(|p| c.panes.contains(p))),
            );
            children.push(child);
        }
        let n = children.len();
        let last_lo = n.saturating_sub(2);
        let mut lo = global
            .iter()
            .position(|g| g.is_some_and(|g| g >= offset))
            .unwrap_or(last_lo)
            .min(last_lo);
        if let Some(f) = focused_local {
            if f < lo {
                lo = f;
            } else if f >= lo + 2 {
                lo = f - 1;
            }
            lo = lo.min(last_lo);
        }
        if n <= 2 {
            lo = 0;
        }
        let widths = global
            .iter()
            .map(|g| g.and_then(|g| cols.get(g)).map(|c| c.width))
            .collect();
        Some(WindowStrip {
            children,
            widths,
            lo,
        })
    }

    /// Synchronize the active window's horizontal root `Linear` container shares
    /// with the Niri column strip presets (`35%` parent / `65%` child) and scroll offset,
    /// using only the active window's own columns (see [`Self::active_window_strip`]).
    ///
    /// Shares are written in pixels of `canvas_width` (the tree's available width)
    /// so the clipped groups come out exactly `STRIP_PEEK_W` wide. Also records
    /// which panes are off-strip and whether the strip layout jumped this frame.
    pub(crate) fn sync_strip_tree_shares(&mut self, canvas_width: Option<f32>) {
        self.zoom.strip_layout_jump = false;
        self.zoom.off_strip_panes.clear();
        if self.zoom.level != ZoomLevel::Columns {
            return;
        }
        let Some(strip) = self.active_window_strip() else {
            self.zoom.last_strip_sig = None;
            return;
        };
        let n = strip.children.len();
        let gap = self.config.pane_gap.unwrap_or(4.0).clamp(0.0, 20.0);
        let canvas_width = canvas_width
            .filter(|w| w.is_finite() && *w > 0.0)
            .unwrap_or(style::STRIP_FALLBACK_CANVAS_W);
        let Some(win) = self.windows.get(self.active_window) else {
            return;
        };
        let sig = (win.window_id, strip.lo, strip.children.clone());
        let jump = self.zoom.last_strip_sig.as_ref() != Some(&sig);
        if jump {
            log::info!(
                "strip layout jump: window={} lo={} columns={n}",
                sig.0,
                strip.lo
            );
        }
        self.zoom.strip_layout_jump = jump;
        self.zoom.last_strip_sig = Some(sig);

        let mut off_strip = HashSet::new();
        if n > 2 {
            let mut leaves = Vec::new();
            for (idx, &child) in strip.children.iter().enumerate() {
                if idx < strip.lo || idx >= strip.lo + 2 {
                    collect_leaf_panes_in_tile(&win.tree, &win.panes, child, &mut leaves);
                }
            }
            off_strip.extend(leaves);
        }
        self.zoom.off_strip_panes = off_strip;

        let ctx_id = self.router.active().context_id;
        let ratios: Vec<f32> = strip
            .widths
            .iter()
            .enumerate()
            .map(|(idx, width)| {
                width.map(|w| {
                    let _ = w.is_custom();
                    w.ratio()
                }).unwrap_or(if idx == 0 || idx == strip.lo {
                    style::COLUMN_WIDTH_PARENT
                } else {
                    style::COLUMN_WIDTH_CHILD
                })
            })
            .collect();

        let shares = strip_shares(&ratios, strip.lo, gap, canvas_width);

        // Compute pure authoritative strip layout to validate geometry
        let authoritative_strip = self.strip_for_context(ctx_id);
        let vp = crate::strip::StripViewport::new(
            Rect::from_min_size(Pos2::ZERO, Vec2::new(canvas_width, 800.0)),
            gap,
        );
        let layout = crate::strip::layout_strip(&authoritative_strip, &vp);
        if authoritative_strip.column_count() > 0 {
            log::debug!(
                "strip: authoritative layout computed cols={} panes={} dividers={}",
                authoritative_strip.column_count(),
                authoritative_strip.pane_count(),
                layout.dividers.len()
            );
        }

        let Some(win) = self.windows.get_mut(self.active_window) else {
            return;
        };
        let Some(root_id) = win.tree.root() else {
            return;
        };
        if let Some(Tile::Container(Container::Linear(lin))) = win.tree.tiles.get_mut(root_id) {
            for (child_id, share) in strip.children.iter().copied().zip(shares) {
                lin.shares.set_share(child_id, share);
            }
        }
    }

    /// Returns `(has_left_clipped, has_right_clipped)` for the active window's column strip,
    /// accounting for multi-window pages and off-screen columns across the context.
    pub(crate) fn active_strip_clipped_neighbors(&self) -> (bool, bool) {
        if self.zoom.level != ZoomLevel::Columns {
            return (false, false);
        }
        let ctx_id = self.router.active().context_id;
        let strip = self.strip_for_context(ctx_id);
        if strip.column_count() <= 1 {
            return (false, false);
        }

        // If active window contains a multi-column horizontal strip, use window strip indices
        if let Some(w_strip) = self.active_window_strip() {
            if w_strip.children.len() > 2 {
                let has_left = w_strip.lo > 0;
                let has_right = w_strip.lo + 2 < w_strip.children.len();
                log::info!(
                    "strip: active_strip_clipped_neighbors window-local lo={} cols={} has_left={has_left} has_right={has_right}",
                    w_strip.lo,
                    w_strip.children.len()
                );
                return (has_left, has_right);
            }
        }

        // Otherwise check context-wide columns (e.g. across multiple windows / pages)
        let active_col = strip
            .focused_pane
            .and_then(|p| strip.find_column_for_pane(p))
            .unwrap_or(0);
        let has_left = active_col > 0;
        let has_right = active_col + 1 < strip.column_count();
        log::info!(
            "strip: active_strip_clipped_neighbors context-wide active_col={active_col} total_cols={} has_left={has_left} has_right={has_right}",
            strip.column_count()
        );
        (has_left, has_right)
    }

    /// Handle click activation on a left or right peek strip, navigating directly to the nearest off-screen column.
    pub fn handle_peek_click(&mut self, is_left: bool) -> bool {
        let ctx_id = self.router.active().context_id;
        let mut strip = self.strip_for_context(ctx_id);
        let has_left = strip.has_left_clipped();
        let has_right = strip.has_right_clipped();
        if is_left && has_left {
            let target_col = strip.scroll_offset.saturating_sub(1);
            strip.ensure_column_visible(target_col);
            self.zoom.strip_offsets.insert(ctx_id, strip.scroll_offset);
            if let Some(col) = strip.columns.get(target_col) {
                if let Some(pid) = col.panes.first().copied() {
                    strip.focus_pane(pid);
                    self.focus_pane_in_strip(pid);
                }
            }
            log::info!("strip: peek clicked left context={ctx_id} target_col={target_col}");
            true
        } else if !is_left && has_right {
            let target_col = (strip.scroll_offset + 2).min(strip.column_count().saturating_sub(1));
            strip.ensure_column_visible(target_col);
            self.zoom.strip_offsets.insert(ctx_id, strip.scroll_offset);
            if let Some(col) = strip.columns.get(target_col) {
                if let Some(pid) = col.panes.first().copied() {
                    strip.focus_pane(pid);
                    self.focus_pane_in_strip(pid);
                }
            }
            log::info!("strip: peek clicked right context={ctx_id} target_col={target_col}");
            true
        } else {
            false
        }
    }

    /// Set a custom width ratio on a column in `context_id`.
    pub fn set_column_width_ratio(&mut self, context_id: u64, col_idx: usize, ratio: f32) {
        let r = ratio.clamp(0.05, 0.95);
        let width = crate::strip::ColumnWidth::CustomRatio(r);
        if width.is_custom() {
            self.zoom.column_widths.insert((context_id, col_idx), width);
        }
        let cols = self.columns_for_context(context_id);
        if cols.len() == 2 {
            let other_idx = if col_idx == 0 { 1 } else { 0 };
            self.zoom.column_widths.insert(
                (context_id, other_idx),
                crate::strip::ColumnWidth::CustomRatio(1.0 - r),
            );
        }
        let mut strip = self.strip_for_context(context_id);
        strip.set_column_width(col_idx, width);
        log::info!("strip: column width updated context={context_id} col={col_idx} ratio={r:.3}");
    }

    /// Reset column widths in `context_id` back to default presets.
    pub fn reset_column_widths(&mut self, context_id: u64) {
        self.set_column_width_ratio(context_id, 0, style::COLUMN_WIDTH_PARENT);
        let mut strip = self.strip_for_context(context_id);
        strip.reset_column_widths();
        self.zoom.column_widths.retain(|(cid, _), _| *cid != context_id);
        log::info!("strip: column widths reset context={context_id}");
    }

    /// Build the authoritative `Strip` model for `context_id`.
    pub fn strip_for_context(&self, context_id: u64) -> crate::strip::Strip {
        let cols = self.columns_for_context(context_id);
        let mut strip = crate::strip::Strip::new(context_id);
        strip.scroll_offset = self.zoom.strip_offset_for(context_id).min(strip.max_scroll_offset());
        strip.focused_pane = self.active_focused_pane_id();
        for col in cols {
            let mut column = if col.panes.len() > 1 {
                crate::strip::Column::new_stacked(
                    crate::strip::ColumnId(col.index),
                    col.panes,
                    col.width,
                )
            } else if let Some(&first) = col.panes.first() {
                crate::strip::Column::new(
                    crate::strip::ColumnId(col.index),
                    first,
                    col.width,
                )
            } else {
                continue;
            };
            if let Some(focused) = strip.focused_pane {
                column.focus_pane(focused);
            }
            strip.push_column(column);
        }
        if let Some(focused) = strip.focused_pane {
            if strip.contains_pane(focused) {
                strip.focus_pane(focused);
            }
        }
        strip
    }

    /// Paint the `28px` clipped neighbor indicator strips at the left/right edges
    /// of the `ZoomLevel::Columns` viewport when off-screen columns exist.
    pub(crate) fn paint_clipped_neighbor_edges(
        painter: &egui::Painter,
        canvas_rect: Rect,
        frame_outer: Option<Rect>,
        has_left: bool,
        has_right: bool,
        colors: crate::ui::theme::Colors,
    ) {
        let (left, right) = peek_strip_rects(canvas_rect, frame_outer, has_left, has_right);
        for (strip, glyph) in [(left, "‹"), (right, "›")] {
            let Some(strip) = strip else { continue };
            painter.rect_filled(strip, style::RADIUS_MOTORWAYS, colors.depth_tint(1));
            painter.rect_stroke(
                strip,
                style::RADIUS_MOTORWAYS,
                Stroke::new(1.0_f32, colors.border),
                egui::StrokeKind::Inside,
            );
            painter.text(
                strip.center(),
                Align2::CENTER_CENTER,
                glyph,
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

        egui::Panel::top("zoom_path_bar")
            .exact_size(style::PATH_BAR_H)
            .frame(
                egui::Frame::NONE
                    .fill(colors.bg_darkest)
                    .inner_margin(egui::Margin::symmetric(14, 0)),
            )
            .show_inside(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    let bar = ui.max_rect();
                    ui.scope_builder(
                        egui::UiBuilder::new().layout(egui::Layout::left_to_right(egui::Align::Center)),
                        |ui| {
                            ui.set_clip_rect(ui.clip_rect().intersect(bar));
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
                        },
                    );
                });
            });
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
        let focused_idx = siblings
            .iter()
            .position(|&cid| cid == focused_ctx)
            .unwrap_or_default();

        let total_w = if siblings.is_empty() {
            0.0
        } else {
            (siblings.len() as f32) * style::CARD_W
                + (siblings.len().saturating_sub(1) as f32) * style::CARD_GAP
        };
        let horizontal_margin = 24.0;
        let is_overflow = total_w > rect.width() - horizontal_margin * 2.0;

        let ideal_offset = if !is_overflow {
            total_w * 0.5
        } else {
            (focused_idx as f32) * (style::CARD_W + style::CARD_GAP) + style::CARD_W * 0.5
        };

        // Selection-follow behavior: keep selected card visible and centered.
        if self.zoom.cards_last_focused_context != Some(focused_ctx) {
            self.zoom.cards_last_focused_context = Some(focused_ctx);
            self.zoom.cards_viewport_offset = ideal_offset;
            log::info!(
                "zoom: sibling cards selection follow ctx={focused_ctx} idx={focused_idx} offset={:.1}",
                self.zoom.cards_viewport_offset
            );
        } else if !is_overflow && self.zoom.cards_viewport_offset == 0.0 {
            self.zoom.cards_viewport_offset = ideal_offset;
        }

        // Pointer scrolling & drag handling
        let min_offset = style::CARD_W * 0.5;
        let max_offset = (siblings.len().saturating_sub(1) as f32) * (style::CARD_W + style::CARD_GAP)
            + style::CARD_W * 0.5;

        let bg_resp = ui.interact(rect, egui::Id::new("sibling_cards_canvas_drag"), Sense::drag());
        if bg_resp.dragged() {
            let drag_x = bg_resp.drag_delta().x;
            if drag_x.abs() > 0.0 {
                self.zoom.cards_viewport_offset -= drag_x;
                if is_overflow {
                    self.zoom.cards_viewport_offset = self.zoom.cards_viewport_offset.clamp(
                        min_offset.min(max_offset),
                        min_offset.max(max_offset),
                    );
                }
                log::info!(
                    "zoom: sibling cards pointer drag delta={drag_x:.1} offset={:.1}",
                    self.zoom.cards_viewport_offset
                );
            }
        }

        if ui.rect_contains_pointer(rect) {
            let scroll_delta = ui.input(|i| i.smooth_scroll_delta);
            let delta_x = if scroll_delta.x.abs() > 0.0 {
                scroll_delta.x
            } else {
                scroll_delta.y
            };
            if delta_x.abs() > 0.0 {
                self.zoom.cards_viewport_offset -= delta_x;
                if is_overflow {
                    self.zoom.cards_viewport_offset = self.zoom.cards_viewport_offset.clamp(
                        min_offset.min(max_offset),
                        min_offset.max(max_offset),
                    );
                }
                log::info!(
                    "zoom: sibling cards pointer scroll delta={delta_x:.1} offset={:.1}",
                    self.zoom.cards_viewport_offset
                );
            }
        }

        let start_x = rect.center().x - self.zoom.cards_viewport_offset;
        let center_y = rect.center().y;

        let mut clicked_ctx: Option<u64> = None;
        #[cfg(test)]
        let mut painted_origin_geom: Option<OriginMarkerGeometry> = None;
        #[cfg(test)]
        let mut painted_cards = Vec::with_capacity(siblings.len());

        for (idx, &cid) in siblings.iter().enumerate() {
            let is_focused = cid == focused_ctx;
            let is_ghost = Some(cid) == ghost_ctx;
            let depth = self
                .router
                .as_slice()
                .iter()
                .find(|c| c.context_id == cid)
                .map(|c| c.depth as usize)
                .unwrap_or(0);
            let asks = self.attention_queue.ask_count_for_context(cid);
            let ctx_name = self
                .router
                .as_slice()
                .iter()
                .find(|c| c.context_id == cid)
                .map(|c| c.name.displayed().to_string())
                .unwrap_or_else(|| format!("Context {cid}"));
            let cols = self.columns_for_context(cid);
            let pane_total: usize = cols.iter().map(|c| c.panes.len()).sum();
            let subtitle = format!("{pane_total} pane{}", if pane_total == 1 { "" } else { "s" });
            let detail = self.context_card_detail(cid);
            let spec = CardSpec {
                state: self.context_card_state(cid),
                title: &ctx_name,
                subtitle: &subtitle,
                detail: &detail,
                badge: (asks > 0).then(|| (format!("◆{asks}"), colors.zoom_needs)),
            };
            let layout = layout_card(ui.ctx(), CardForm::Sibling, &spec, style::CARD_W);
            let header_h = layout.height;
            let card_h = style::CARD_H.max(header_h + SIBLING_BODY_MIN_H);
            let card_min = Pos2::new(
                start_x + (idx as f32) * (style::CARD_W + style::CARD_GAP),
                center_y - card_h * 0.5,
            );
            let card_rect = Rect::from_min_size(card_min, Vec2::new(style::CARD_W, card_h));
            #[cfg(test)]
            painted_cards.push((cid, card_rect, is_focused));
            paint_card(
                &painter,
                card_rect,
                &layout,
                &spec,
                &colors,
                colors.depth_tint(depth),
                is_focused,
            );
            let card_painter = painter.with_clip_rect(card_rect.shrink(1.0));

            if is_ghost {
                let _geom = paint_origin_marker(
                    &card_painter,
                    Pos2::new(card_rect.min.x + 14.0, card_rect.max.y - 12.0),
                    colors.accent,
                );
                #[cfg(test)]
                {
                    painted_origin_geom = Some(_geom);
                }
            }

            // Mini column tiles inside the card, below the header block.
            let inner_top = card_rect.min.y + header_h + 6.0;
            let inner_h = (card_rect.max.y - 30.0 - inner_top).max(40.0);
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
                    let mini_font = FontId::new(style::TEXT_META, FontFamily::Monospace);
                    let name_lines = wrap_text(
                        ui.ctx(),
                        &pname,
                        mini_font.clone(),
                        (mini_rect.width() - 12.0).max(1.0),
                        Some(((ph - 8.0) / (style::TEXT_META * 1.25)).floor().max(1.0) as usize),
                    );
                    painter
                        .with_clip_rect(mini_rect.shrink(1.0))
                        .text(
                            Pos2::new(mini_rect.min.x + 6.0, mini_rect.min.y + 6.0),
                            Align2::LEFT_TOP,
                            name_lines.join("\n"),
                            mini_font,
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

        #[cfg(test)]
        ui.ctx().data_mut(|data| {
            data.insert_temp(
                egui::Id::new("sibling_cards_painted_geometry"),
                (rect, painted_cards),
            );
            data.insert_temp(
                egui::Id::new("sibling_cards_viewport_offset"),
                self.zoom.cards_viewport_offset,
            );
            data.insert_temp(
                egui::Id::new("sibling_cards_origin_marker_geometry"),
                painted_origin_geom,
            );
        });
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
            .filter(|id| nodes.iter().any(|n| n.context_id == *id))
            .or_else(|| {
                nodes
                    .iter()
                    .find(|n| n.context_id == self.router.active().context_id)
                    .map(|n| n.context_id)
            })
            .or_else(|| nodes.first().map(|n| n.context_id))
            .unwrap_or_else(|| self.router.active().context_id);
        self.zoom.atlas_focused_context = Some(focused_ctx);

        // 1. Draw Motorways road edges (casing + road stroke, elbow paths for tree edges).
        for edge in &edges {
            let stroke = match edge.kind {
                AtlasEdgeKind::NeedsRollup => Stroke::new(style::EDGE_NEEDS_W, colors.zoom_needs),
                AtlasEdgeKind::Failed => Stroke::new(style::EDGE_ROAD_W, colors.zoom_failed),
                AtlasEdgeKind::PipeLink => Stroke::new(1.5_f32, colors.accent),
                AtlasEdgeKind::Spawn => Stroke::new(1.5_f32, colors.text_dim),
                AtlasEdgeKind::TreeRoad => Stroke::new(style::EDGE_ROAD_W, colors.border),
            };
            let casing = Stroke::new(stroke.width + 3.0_f32, colors.shadow_flat);
            let is_tree_edge = matches!(
                edge.kind,
                AtlasEdgeKind::TreeRoad | AtlasEdgeKind::NeedsRollup | AtlasEdgeKind::Failed
            );
            if is_tree_edge
                && (edge.from_pos.x - edge.to_pos.x).abs() > 1.0
                && (edge.to_pos.y - edge.from_pos.y).abs() > 4.0
            {
                let mid_y = (edge.from_pos.y + edge.to_pos.y) * 0.5;
                let p1 = Pos2::new(edge.from_pos.x, mid_y);
                let p2 = Pos2::new(edge.to_pos.x, mid_y);
                for seg in [
                    [edge.from_pos, p1],
                    [p1, p2],
                    [p2, edge.to_pos],
                ] {
                    painter.line_segment(seg, casing);
                    painter.line_segment(seg, stroke);
                }
            } else {
                painter.line_segment([edge.from_pos, edge.to_pos], casing);
                painter.line_segment([edge.from_pos, edge.to_pos], stroke);
            }
        }

        // 1b. Draw visible top-center hub pill when multiple roots exist.
        if edges.iter().any(|e| e.from_context_id == 0) {
            let hub_rect = Self::atlas_hub_rect(rect);
            painter.rect_filled(
                hub_rect.translate(Vec2::new(2.0, 2.0)),
                style::RADIUS_PILL,
                colors.shadow_flat,
            );
            painter.rect_filled(hub_rect, style::RADIUS_PILL, colors.tile_bg);
            painter.rect_stroke(
                hub_rect,
                style::RADIUS_PILL,
                Stroke::new(1.5_f32, colors.border),
                egui::StrokeKind::Inside,
            );
            painter.text(
                hub_rect.center(),
                Align2::CENTER_CENTER,
                "contexts",
                crate::ui::theme::font_medium(style::TEXT_CAPTION),
                colors.text_dim,
            );
        }

        let mut clicked_node: Option<u64> = None;
        #[cfg(test)]
        let mut painted_nodes = Vec::with_capacity(nodes.len());

        // 2. Draw context nodes with depth tinting (0..4 clamped).
        for node in &nodes {
            if !rect.intersects(node.rect.expand(40.0)) {
                continue; // Viewport culling for 50+ nodes (`SPEC.md` Q7).
            }
            let is_focused = node.context_id == focused_ctx;
            let nrect = node.rect;

            let state = if node.direct_asks > 0 || node.ask_rollup > 0 {
                CardState::NeedsYou
            } else if node.failed_count > 0 {
                CardState::Failed
            } else {
                self.context_card_state(node.context_id)
            };
            let badge = if node.direct_asks > 0 {
                Some((format!("◆{}", node.direct_asks), colors.zoom_needs))
            } else if node.failed_count > 0 {
                Some((format!("✕{}", node.failed_count), colors.zoom_failed))
            } else {
                None
            };
            let subtitle = if node.is_ghost { "◌ you are here" } else { "" };
            let detail = node.preview_lines.first().cloned().unwrap_or_default();
            let spec = CardSpec {
                state,
                title: &node.name,
                subtitle,
                detail: &detail,
                badge,
            };
            let layout = layout_card(ui.ctx(), CardForm::Atlas, &spec, nrect.width());
            // Grow to fit the wrapped text, keeping the node centered on its edges.
            let nrect = Rect::from_center_size(
                nrect.center(),
                Vec2::new(nrect.width(), nrect.height().max(layout.height)),
            );
            #[cfg(test)]
            painted_nodes.push((node.context_id, nrect, is_focused));
            paint_card(
                &painter,
                nrect,
                &layout,
                &spec,
                &colors,
                colors.depth_tint(node.depth),
                is_focused,
            );

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
                painter.with_clip_rect(peek_rect.shrink(1.0)).text(
                    Pos2::new(peek_rect.min.x + 14.0, peek_rect.min.y + 12.0),
                    Align2::LEFT_TOP,
                    wrap_text(
                        ui.ctx(),
                        &format!("PEEK · {} (read-only)", fnode.name),
                        crate::ui::theme::font_medium(style::TEXT_CAPTION),
                        peek_w - 28.0,
                        Some(1),
                    )
                    .join(""),
                    crate::ui::theme::font_medium(style::TEXT_CAPTION),
                    colors.accent,
                );
                let peek_painter = painter.with_clip_rect(peek_rect.shrink(1.0));
                let peek_font = FontId::new(style::TEXT_HINT, FontFamily::Monospace);
                let mut py = peek_rect.min.y + 38.0;
                for line in self.context_preview_lines(fnode.context_id, 6) {
                    for wrapped in wrap_text(
                        ui.ctx(),
                        &line,
                        peek_font.clone(),
                        peek_w - 28.0,
                        Some(2),
                    ) {
                        if py + 16.0 > peek_rect.max.y {
                            break;
                        }
                        peek_painter.text(
                            Pos2::new(peek_rect.min.x + 14.0, py),
                            Align2::LEFT_TOP,
                            wrapped,
                            peek_font.clone(),
                            colors.text_primary,
                        );
                        py += 16.0;
                    }
                }
            }
        }

        // Bottom key legend
        painter.text(
            Pos2::new(rect.center().x, rect.max.y - 16.0),
            Align2::CENTER_BOTTOM,
            "k parent · j child · h/l sibling · [/] sibling · n/N needs you · p peek · Enter dive · q back to ghost",
            FontId::new(style::TEXT_HINT, FontFamily::Proportional),
            colors.text_dim,
        );

        if let Some(cid) = clicked_node {
            self.zoom.atlas_focused_context = Some(cid);
            self.dive_from_tree_atlas();
        }

        #[cfg(test)]
        ui.ctx().data_mut(|data| {
            data.insert_temp(
                egui::Id::new("tree_atlas_painted_geometry"),
                (rect, painted_nodes),
            );
        });
    }
}

#[cfg(test)]
mod peek_strip_rect_tests {
    use super::*;

    #[test]
    fn peek_strip_rects_align_with_focused_frame_outer_and_stay_clear_of_visible_pane() {
        let gap = 4.0;
        let canvas = Rect::from_min_max(Pos2::new(240.0, 42.0), Pos2::new(1240.0, 796.0));
        let visible_pane = Rect::from_min_max(
            Pos2::new(canvas.min.x + style::STRIP_PEEK_W + gap, 68.0),
            Pos2::new(canvas.max.x - style::STRIP_PEEK_W - gap, 796.0),
        );
        let frame_outer = visible_pane.expand(gap);

        let (left, right) = peek_strip_rects(canvas, Some(frame_outer), true, true);
        let left = left.expect("left strip");
        let right = right.expect("right strip");

        for strip in [left, right] {
            assert_eq!(strip.min.y, frame_outer.min.y);
            assert_eq!(strip.max.y, frame_outer.max.y);
            assert!(strip.min.x >= canvas.min.x && strip.max.x <= canvas.max.x);
            assert_eq!(strip.width(), style::STRIP_PEEK_W);
            assert!(!strip.intersects(visible_pane));
        }
        assert_eq!(left.min.x, canvas.min.x);
        assert_eq!(right.max.x, canvas.max.x);
    }

    #[test]
    fn peek_strip_rects_fall_back_to_canvas_and_clamp_horizontally() {
        let canvas = Rect::from_min_max(Pos2::new(100.0, 38.0), Pos2::new(900.0, 760.0));
        let (left, right) = peek_strip_rects(canvas, None, true, false);
        let left = left.expect("left fallback strip");
        assert!(right.is_none());
        assert_eq!(left.min.y, canvas.min.y);
        assert_eq!(left.max.y, canvas.max.y);
        assert_eq!(left.min.x, canvas.min.x);
        assert_eq!(left.max.x, canvas.min.x + style::STRIP_PEEK_W);

        let narrow = Rect::from_min_max(Pos2::new(50.0, 10.0), Pos2::new(62.0, 90.0));
        let (n_left, n_right) = peek_strip_rects(narrow, None, true, true);
        let n_left = n_left.expect("narrow left");
        let n_right = n_right.expect("narrow right");
        assert!(n_left.min.x >= narrow.min.x && n_left.max.x <= narrow.max.x);
        assert!(n_right.min.x >= narrow.min.x && n_right.max.x <= narrow.max.x);
    }
}

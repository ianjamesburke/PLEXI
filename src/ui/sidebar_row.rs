//! The sidebar context row.
//!
//! `SidebarRow` is the context list's counterpart to [`crate::ui::list::ListRow`]:
//! same shared internals (selection painting, text elision, pixel snapping),
//! plus the affordances a list row has no business carrying — a leading index
//! gutter aligned to the title, a numeric badge slot distinct from the trailing
//! action, drag-reorder sensing, and pane pips grouped into per-window capsules.
//!
//! The row is two tiers, the same convention [`crate::ui::list::ListRow`] uses
//! when it has a secondary line: identity on top (index gutter, context name,
//! and the close action pinned to the row's top-right corner), pane state
//! below (per-window pip capsules, the overflow count, the notification badge,
//! and the root path filling whatever is left).
//!
//! Geometry is resolved exactly once, by [`RowGeometry::measure`], into explicit
//! slot rects in row-local space; [`RowGeometry::translated`] moves the whole set
//! onto the allocated rect. Painting and hit testing both read those rects, so no
//! position is ever reconstructed from a captured height after the fact, and the
//! row's width is a fixed budget that cannot exceed what the panel offered.

use crate::ui::list::{elide_to_width, paint_selection, paint_text_centered, selection_inset};
use crate::ui::style;
use crate::ui::theme::Colors;
use egui::emath::GuiRounding;
use egui::{Color32, CornerRadius, CursorIcon, Id, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};

/// Left inset before the index gutter. Every sidebar row is a top-level
/// context, so there is no nesting level to indent for.
const ROW_INDENT: f32 = 4.0;
/// Fixed-width left gutter that holds the context index number.
const GUTTER_W: f32 = 18.0;
/// Top and bottom padding added inside each row for vertical breathing room.
const ROW_PAD_V: f32 = style::SPACE_MD;
/// Right margin for row content, measured from the row edge. The selection
/// card insets by `SPACE_XS`, so this leaves visible air inside its outline.
const ROW_PAD_RIGHT: f32 = style::SPACE_MD;
/// Vertical gap between the identity tier and the pane-state tier. The two
/// tiers are separate readings of the row, so the gap has to be wide enough
/// that they do not scan as one wrapped line.
const TIER_GAP: f32 = style::SPACE_SM;

/// The close action is the row's card-level dismiss, pinned to the top-right
/// corner of the identity tier — a touch larger than body chrome so it reads
/// as closing the whole card, not editing a field.
const CLOSE_SLOT_W: f32 = 26.0;
const CLOSE_GLYPH_SIZE: f32 = style::TEXT_TITLE;
/// Gap between the pip strip and the notification badge. The badge is a
/// separate kind of thing from the pips and must never look glued to them.
const PIP_BADGE_GAP: f32 = style::SPACE_SM;
/// Gap before the root path, which fills whatever the pane state leaves.
const PATH_GAP: f32 = style::SPACE_SM;
/// Below this the path is dropped rather than elided down to a stub — pane
/// state is the tier's job, and "/pro…" is noise, not information.
const PATH_MIN_W: f32 = 48.0;
/// Gap between the last capsule and the "+N" overflow count.
const PIP_TEXT_GAP: f32 = style::SPACE_SM;

/// Status-pip radius. Shared with the portal minimap so activity dots are the
/// same size everywhere they appear (sidebar rows + portal previews).
pub(crate) const PANE_DOT_RADIUS: f32 = 4.0;
const PANE_DOT_SPACING: f32 = 11.0;
const PANE_DOT_MAX: usize = 8;
/// Air between the dots and the capsule's stroke, on each axis.
const WINDOW_GROUP_PAD_X: f32 = 7.0;
const WINDOW_GROUP_PAD_Y: f32 = 6.0;
const WINDOW_GROUP_GAP: f32 = 5.0;
const WINDOW_GROUP_RADIUS: CornerRadius = CornerRadius::same(4);
/// Width reserved for the "+N" label when the pane count exceeds the cap.
const PIP_OVERFLOW_W: f32 = 20.0;
const GROUP_STROKE_W: f32 = 1.0;
const GROUP_STROKE_W_RETURN: f32 = 1.5;
/// Return-target marker: a small triangle whose base sits flush on the
/// capsule's inner top edge and whose tip points down at the dots. It is a
/// fixed size rather than a fraction of the capsule's padding — the marker
/// should not grow when the capsule gets roomier — and the assertion below is
/// what keeps its tip clear of the dot row.
const PIN_HALF_W: f32 = 2.5;
const PIN_H: f32 = 3.0;
const _: () = assert!(
    PIN_H > 0.0 && PIN_H < WINDOW_GROUP_PAD_Y,
    "the pin must fit inside the capsule's top padding without touching the dot row"
);
const _: () = assert!(
    PIN_HALF_W * 2.0 < PANE_DOT_SPACING,
    "the pin must not reach the neighbouring dot"
);

pub(crate) fn with_alpha(c: Color32, alpha: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (c.a() as f32 * alpha) as u8)
}

static HOME_DIR: std::sync::OnceLock<String> = std::sync::OnceLock::new();

fn shorten_path(path: &str) -> String {
    let home = HOME_DIR.get_or_init(|| std::env::var("HOME").unwrap_or_default());
    let shortened = if !home.is_empty() {
        path.strip_prefix(home.as_str())
            .map_or_else(|| path.to_string(), |rest| format!("~{rest}"))
    } else {
        path.to_string()
    };
    let char_count = shortened.chars().count();
    if char_count > 40 {
        let tail: String = shortened
            .chars()
            .rev()
            .take(39)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        format!("\u{2026}{tail}")
    } else {
        shortened
    }
}

pub enum SidebarAction {
    None,
    Activate,
    Rename,
    Delete,
    DragStart,
    DragEnd,
}

pub struct PaneDots {
    pub count: usize,
    pub focused_idx: Option<usize>,
    /// Set of dot indices that are hidden (rendered as stroke-only outlines).
    pub hidden_set: std::collections::HashSet<usize>,
    /// Per-dot agent state (parallel to dot index). `None` means no agent.
    pub activities: Vec<Option<crate::app_protocol::AgentState>>,
    /// Contiguous pane ranges, one per spatial window in this context.
    pub windows: Vec<PaneDotWindow>,
}

pub struct PaneDotWindow {
    pub start: usize,
    pub count: usize,
    /// The window this context will restore when it is activated.
    pub is_return_target: bool,
    /// Whether this is also the globally active window right now.
    pub is_active: bool,
}

pub struct SidebarRow {
    pub is_active: bool,
    pub is_dragging: bool,
    pub any_dragging: bool,
    pub action_enabled: bool,
    pub ctx_name: String,
    pub ctx_index: Option<usize>,
    pub badge_count: usize,
    /// Root path, trailing the pane-state tier.
    pub subtitle: Option<String>,
    /// Pane pips, leading the pane-state tier.
    pub pane_dots: Option<PaneDots>,
    /// Whether this row supports drag reordering. When false, hover shows
    /// PointingHand instead of Grab and drag actions are suppressed.
    pub draggable: bool,
}

/// One window's capsule inside the pip strip, in strip-local coordinates.
struct PipGroup {
    /// Dot range this capsule covers, already clamped to the visible cap.
    start: usize,
    end: usize,
    is_return_target: bool,
    is_active: bool,
    x: f32,
    width: f32,
}

/// The measured pip strip: the single source of truth for its width, used both
/// to budget the title text and to paint the capsules.
struct PipLayout {
    width: f32,
    height: f32,
    groups: Vec<PipGroup>,
    /// Number of panes past the visible cap, when there are any.
    overflow: Option<usize>,
}

impl PipLayout {
    /// The widest strip that fits `max_width`. The visible dot count shrinks
    /// until it does — panes past it fold into the "+N" overflow — so a narrow
    /// panel loses dots rather than pushing the strip under the context name.
    fn measure(dots: &PaneDots, max_width: f32) -> Option<Self> {
        let mut cap = dots.count.min(PANE_DOT_MAX);
        loop {
            match Self::at_cap(dots, cap) {
                Some(layout) if layout.width <= max_width => return Some(layout),
                _ if cap == 0 => return None,
                _ => cap -= 1,
            }
        }
    }

    fn at_cap(dots: &PaneDots, capped: usize) -> Option<Self> {
        if dots.count == 0 {
            return None;
        }
        // A context always reports its spatial windows; a caller that supplies
        // none still gets one capsule, so there is exactly one layout path.
        let ranges: Vec<(usize, usize, bool, bool)> = if capped == 0 {
            Vec::new()
        } else if dots.windows.is_empty() {
            vec![(0, capped, false, false)]
        } else {
            dots.windows
                .iter()
                .filter(|window| window.count > 0 && window.start < capped)
                .map(|window| {
                    (
                        window.start,
                        (window.start + window.count).min(capped),
                        window.is_return_target,
                        window.is_active,
                    )
                })
                .collect()
        };

        let mut groups = Vec::with_capacity(ranges.len());
        let mut x = 0.0_f32;
        for (i, &(start, end, is_return_target, is_active)) in ranges.iter().enumerate() {
            let width = (end - start).saturating_sub(1) as f32 * PANE_DOT_SPACING
                + PANE_DOT_RADIUS * 2.0
                + WINDOW_GROUP_PAD_X * 2.0;
            groups.push(PipGroup {
                start,
                end,
                is_return_target,
                is_active,
                x,
                width,
            });
            x += width;
            if i + 1 < ranges.len() {
                x += WINDOW_GROUP_GAP;
            }
        }

        let overflow = (dots.count > capped).then(|| dots.count - capped);
        if overflow.is_some() {
            if !groups.is_empty() {
                x += PIP_TEXT_GAP;
            }
            x += PIP_OVERFLOW_W;
        }

        Some(Self {
            width: x,
            height: PANE_DOT_RADIUS * 2.0 + WINDOW_GROUP_PAD_Y * 2.0,
            groups,
            overflow,
        })
    }

    /// Center x of dot `idx`, in strip-local coordinates.
    fn dot_center_x(&self, idx: usize) -> Option<f32> {
        self.groups
            .iter()
            .find(|group| (group.start..group.end).contains(&idx))
            .map(|group| {
                group.x
                    + WINDOW_GROUP_PAD_X
                    + PANE_DOT_RADIUS
                    + (idx - group.start) as f32 * PANE_DOT_SPACING
            })
    }
}

/// Every rect the row paints or hit tests. Measured in row-local space (origin
/// at the row's top-left), then translated onto the allocated rect.
struct RowGeometry {
    rect: Rect,
    /// Index-number cell. Its center y is the title's center y, so the number
    /// shares the name's optical baseline instead of the whole row's.
    gutter: Rect,
    title: Rect,
    title_galley: std::sync::Arc<egui::Galley>,
    /// Close action, pinned to the identity tier's right edge.
    close: Option<Rect>,
    /// Strip origin (left edge, vertical center) plus its measured layout.
    pips: Option<(Pos2, PipLayout)>,
    /// Badge pill and its centered count text.
    badge: Option<(Rect, std::sync::Arc<egui::Galley>)>,
    /// Root path, right-aligned in whatever the pane-state tier has left.
    path: Option<(Rect, std::sync::Arc<egui::Galley>)>,
}

impl RowGeometry {
    fn measure(ui: &egui::Ui, row: &SidebarRow, width: f32) -> Self {
        let content_left = ROW_INDENT + GUTTER_W;
        let content_right = width - ROW_PAD_RIGHT;

        // ── Identity tier: gutter, name, close ────────────────────────────
        // The close action is the only thing that shares this line, so the
        // name's budget is one subtraction and does not vary with pane state.
        let close_left = row.action_enabled.then_some(content_right - CLOSE_SLOT_W);
        let title_max = (close_left.unwrap_or(content_right) - content_left).max(0.0);
        let title_galley = elided_galley(
            ui,
            &row.ctx_name,
            egui::FontId::proportional(style::TEXT_SIDEBAR_TITLE),
            title_max,
        );
        // The close slot is a hit target, not a text line: it is centered on
        // the title and allowed to overhang into the row's padding rather than
        // stretching the tier and bulking up every row.
        let identity_h = title_galley.size().y;
        let identity_top = ROW_PAD_V;
        let identity_center_y = identity_top + identity_h / 2.0;

        // ── Pane-state tier: pips, badge, path ────────────────────────────
        // Laid out left to right with fixed reservations, so the path is the
        // only elastic element and no two of them can ever collide.
        let lane = (content_right - content_left).max(0.0);
        let badge_galley = (row.badge_count > 0).then(|| {
            let label = if row.badge_count > 9 {
                "9+".to_string()
            } else {
                row.badge_count.to_string()
            };
            ui.fonts_mut(|f| {
                f.layout_no_wrap(
                    label,
                    egui::FontId::proportional(style::TEXT_META),
                    Color32::PLACEHOLDER,
                )
            })
        });
        let badge_size = badge_galley.as_ref().map(|galley| {
            let h = galley.size().y + style::BADGE_PAD_V * 2.0;
            Vec2::new((galley.size().x + style::BADGE_PAD_H * 2.0).max(h), h)
        });
        let badge_reserved = badge_size.map_or(0.0, |size| size.x + PIP_BADGE_GAP);

        let pips = row
            .pane_dots
            .as_ref()
            .and_then(|dots| PipLayout::measure(dots, lane - badge_reserved));
        let mut state_x = content_left + pips.as_ref().map_or(0.0, |layout| layout.width);
        let badge_left = badge_size.map(|size| {
            let left = if pips.is_some() {
                state_x + PIP_BADGE_GAP
            } else {
                state_x
            };
            state_x = left + size.x;
            left
        });

        let path_avail = (content_right - state_x - PATH_GAP).max(0.0);
        let path_galley = row
            .subtitle
            .as_ref()
            .filter(|_| path_avail >= PATH_MIN_W)
            .map(|path| {
                elided_galley(
                    ui,
                    &shorten_path(path),
                    egui::FontId::proportional(style::TEXT_META),
                    path_avail,
                )
            });

        let state_h = pips
            .as_ref()
            .map_or(0.0, |layout| layout.height)
            .max(badge_size.map_or(0.0, |size| size.y))
            .max(path_galley.as_ref().map_or(0.0, |g| g.size().y));
        // A row with no pane state at all is one tier tall, gap included.
        let state_top = identity_top + identity_h + if state_h > 0.0 { TIER_GAP } else { 0.0 };
        let state_center_y = state_top + state_h / 2.0;
        let height = state_top + state_h + ROW_PAD_V;

        let centered = |left: f32, size: Vec2| {
            Rect::from_min_size(Pos2::new(left, state_center_y - size.y / 2.0), size)
        };

        Self {
            rect: Rect::from_min_size(Pos2::ZERO, Vec2::new(width, height)),
            gutter: Rect::from_min_size(
                Pos2::new(ROW_INDENT, identity_top),
                Vec2::new(GUTTER_W, identity_h),
            ),
            title: Rect::from_min_size(
                Pos2::new(content_left, identity_center_y - identity_h / 2.0),
                title_galley.size(),
            ),
            title_galley,
            close: close_left.map(|left| {
                Rect::from_center_size(
                    Pos2::new(left + CLOSE_SLOT_W / 2.0, identity_center_y),
                    Vec2::splat(CLOSE_SLOT_W),
                )
            }),
            pips: pips.map(|layout| (Pos2::new(content_left, state_center_y), layout)),
            badge: badge_galley
                .zip(badge_left)
                .zip(badge_size)
                .map(|((galley, left), size)| (centered(left, size), galley)),
            path: path_galley.map(|galley| {
                let size = galley.size();
                (centered(content_right - size.x, size), galley)
            }),
        }
    }

    fn translated(self, offset: Vec2) -> Self {
        Self {
            rect: self.rect.translate(offset),
            gutter: self.gutter.translate(offset),
            title: self.title.translate(offset),
            title_galley: self.title_galley,
            close: self.close.map(|rect| rect.translate(offset)),
            pips: self.pips.map(|(origin, layout)| (origin + offset, layout)),
            badge: self
                .badge
                .map(|(rect, galley)| (rect.translate(offset), galley)),
            path: self
                .path
                .map(|(rect, galley)| (rect.translate(offset), galley)),
        }
    }
}

fn elided_galley(
    ui: &egui::Ui,
    text: &str,
    font_id: egui::FontId,
    max_width: f32,
) -> std::sync::Arc<egui::Galley> {
    let text = elide_to_width(ui, text, font_id.clone(), max_width);
    // PLACEHOLDER defers the color to paint time, so measurement never has to
    // know which of the row's alpha-modulated tones this text will end up in.
    ui.fonts_mut(|f| f.layout_no_wrap(text, font_id, Color32::PLACEHOLDER))
}

/// Paint the pip strip: one capsule per spatial window, the dots inside them,
/// the return-target pin, and the overflow count.
fn paint_pips(
    ui: &egui::Ui,
    dots: &PaneDots,
    origin: Pos2,
    layout: &PipLayout,
    colors: &Colors,
    row_alpha: f32,
    is_dragging: bool,
) {
    let t = ui.input(|i| i.time);
    let has_working = dots
        .activities
        .iter()
        .any(|s| matches!(s, Some(crate::app_protocol::AgentState::Working)));
    if has_working {
        // Pulse animation only needs ~10fps. An unconditional request_repaint
        // here is self-perpetuating and pins the whole window at display
        // refresh for as long as any agent is Working.
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(100));
    }

    let painter = ui.painter();
    let ppp = painter.pixels_per_point();
    let cy = origin.y.round_to_pixel_center(ppp);

    for group in &layout.groups {
        let group_rect = Rect::from_min_size(
            Pos2::new(origin.x + group.x, cy - layout.height / 2.0),
            Vec2::new(group.width, layout.height),
        )
        .round_to_pixels(ppp);
        let fill = if group.is_active {
            with_alpha(colors.accent, 0.16 * row_alpha)
        } else {
            // Keep group fill darker than a hovered row, so hover remains a
            // row-level wash instead of erasing window boundaries.
            with_alpha(colors.bg_darkest, 0.3 * row_alpha)
        };
        let stroke = if group.is_active {
            Stroke::new(GROUP_STROKE_W, with_alpha(colors.accent, row_alpha))
        } else if group.is_return_target {
            // `border` is the theme's quiet structural outline; a
            // high-contrast text color made this capsule compete with the
            // context name and the activity dots.
            Stroke::new(GROUP_STROKE_W_RETURN, with_alpha(colors.border, row_alpha))
        } else {
            Stroke::new(
                GROUP_STROKE_W,
                with_alpha(colors.text_dim, 0.72 * row_alpha),
            )
        };
        painter.rect_filled(group_rect, WINDOW_GROUP_RADIUS, fill);
        painter.rect_stroke(group_rect, WINDOW_GROUP_RADIUS, stroke, StrokeKind::Inside);

        for dot_i in group.start..group.end {
            let cx = origin.x
                + group.x
                + WINDOW_GROUP_PAD_X
                + PANE_DOT_RADIUS
                + (dot_i - group.start) as f32 * PANE_DOT_SPACING;
            let agent_state = dots.activities.get(dot_i).and_then(|s| s.as_ref());
            let focused = dots.focused_idx == Some(dot_i);
            let mut color = crate::ui::activity::pip_color(agent_state, focused, colors, t, dot_i)
                .gamma_multiply(row_alpha);
            if is_dragging && agent_state.is_none() && !focused {
                color = color.gamma_multiply(0.4);
            }
            let center = Pos2::new(cx, cy).round_to_pixel_center(ppp);
            if dots.hidden_set.contains(&dot_i) {
                painter.circle_stroke(center, PANE_DOT_RADIUS, Stroke::new(1.0_f32, color));
            } else {
                painter.circle_filled(center, PANE_DOT_RADIUS, color);
            }
        }

        // The pin is intentionally neutral: dot color describes agent activity,
        // while this direction marker describes where a context will return.
        // Its base sits on the capsule's inner top edge — the stroke is drawn
        // inside the rect, so that edge is `min.y + stroke.width` — and both are
        // on the same pixel grid, leaving no seam.
        if group.is_return_target {
            let Some(focused_idx) = dots.focused_idx else {
                continue;
            };
            // A focused pane past the visible cap has no dot; the overflow
            // label carries the highlight instead.
            let Some(local_cx) = layout
                .dot_center_x(focused_idx)
                .filter(|_| (group.start..group.end).contains(&focused_idx))
            else {
                continue;
            };
            let cx = (origin.x + local_cx).round_to_pixel_center(ppp);
            let base_y = (group_rect.min.y + stroke.width).round_to_pixels(ppp);
            let pin_color = with_alpha(
                colors.text_primary,
                if group.is_active {
                    row_alpha
                } else {
                    0.78 * row_alpha
                },
            );
            painter.add(egui::Shape::convex_polygon(
                vec![
                    Pos2::new(cx - PIN_HALF_W, base_y),
                    Pos2::new(cx + PIN_HALF_W, base_y),
                    Pos2::new(cx, base_y + PIN_H),
                ],
                pin_color,
                Stroke::NONE,
            ));
        }
    }

    if let Some(hidden) = layout.overflow {
        // The overflow count stands in for dots, so it is painted in a dot's
        // own color — never the accent the notification badge owns, which is
        // what made a collapsed pip strip read as a badge. A focused pane with
        // no dot of its own is the one this count represents, so it takes the
        // focused dot color.
        let collapsed_is_focused = dots
            .focused_idx
            .is_some_and(|idx| layout.dot_center_x(idx).is_none());
        let overflow_color =
            crate::ui::activity::pip_color(None, collapsed_is_focused, colors, t, 0)
                .gamma_multiply(row_alpha);
        crate::ui::snap::text_snapped(
            painter,
            Pos2::new(origin.x + layout.width - PIP_OVERFLOW_W, cy),
            egui::Align2::LEFT_CENTER,
            format!("+{hidden}"),
            egui::FontId::proportional(style::TEXT_META),
            overflow_color,
        );
    }
}

impl SidebarRow {
    pub fn show(
        self,
        ui: &mut egui::Ui,
        id: Id,
        colors: &Colors,
    ) -> (SidebarAction, egui::Response) {
        let row_alpha = if self.is_dragging { 0.4_f32 } else { 1.0_f32 };

        // The row claims exactly the width it was offered, never more: egui
        // stores a panel's content rect as the panel's size and reads it back
        // as the width next frame, so an overflowing row ramps the sidebar to
        // its `size_range` maximum and swallows user resizes (stint 0715).
        let width = ui.available_width();
        let geom = RowGeometry::measure(ui, &self, width);
        let (rect, _) = ui.allocate_exact_size(geom.rect.size(), Sense::hover());
        let geom = geom.translated(rect.min.to_vec2());

        let response = ui.interact(rect, id, Sense::click_and_drag());
        // The row paints its own text, so nothing else puts the context name
        // into the accessibility tree that scene `assert_label` and
        // `host_has_label` read. The full name is reported even when the
        // painted title is elided.
        response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::Button,
                true,
                self.is_active,
                &self.ctx_name,
            )
        });
        let hovered = response.hovered();

        // --- Background ---------------------------------------------------
        // A 4pt horizontal inset gives the pill breathing room from the
        // sidebar edges, matching the visual weight of palette rows.
        let card = Rect::from_min_max(
            Pos2::new(rect.min.x + style::SPACE_XS, rect.min.y),
            Pos2::new(rect.max.x - style::SPACE_XS, rect.max.y),
        );
        if self.is_active {
            if row_alpha < 1.0 {
                // Dragging: a dim solid fill, no outline.
                ui.painter().rect_filled(
                    rect,
                    CornerRadius::ZERO,
                    with_alpha(colors.bg_active, row_alpha),
                );
            } else {
                paint_selection(ui.painter(), card, colors);
            }
        } else if hovered && !self.is_dragging {
            ui.painter().rect_filled(
                selection_inset(card),
                style::RADIUS_SM,
                with_alpha(colors.bg_hover, row_alpha),
            );
        }

        // --- Index gutter ---------------------------------------------------
        if let Some(idx) = self.ctx_index.filter(|idx| *idx < 9) {
            paint_text_centered(
                ui,
                format!("{}", idx + 1),
                egui::FontId::proportional(style::TEXT_HINT),
                with_alpha(colors.text_dim, row_alpha),
                geom.gutter.center(),
            );
        }

        // --- Title + path ---------------------------------------------------
        // Context names are primary labels: inactive rows read constantly, so
        // they get the contrast-floored secondary tone, not raw `text_dim`
        // (stint 0528).
        let title_color = with_alpha(
            if self.is_active {
                colors.text_primary
            } else {
                colors.text_secondary(colors.bg_sidebar)
            },
            row_alpha,
        );
        crate::ui::snap::galley_snapped(
            ui.painter(),
            geom.title.min,
            geom.title_galley.clone(),
            title_color,
        );

        // --- Pips -------------------------------------------------------------
        if let (Some(dots), Some((origin, layout))) = (&self.pane_dots, &geom.pips) {
            paint_pips(
                ui,
                dots,
                *origin,
                layout,
                colors,
                row_alpha,
                self.is_dragging,
            );
        }

        // --- Badge ------------------------------------------------------------
        // A filled accent pill: taller than a pip, a different shape, and a
        // different color family, so a notification count can never be
        // mistaken for a collapsed pip strip sitting next to it.
        if let Some((badge_rect, galley)) = &geom.badge {
            let text_color = with_alpha(colors.text_on(colors.accent), row_alpha);
            ui.painter().rect_filled(
                *badge_rect,
                style::RADIUS_BADGE,
                with_alpha(colors.accent, row_alpha),
            );
            crate::ui::snap::galley_snapped(
                ui.painter(),
                Pos2::new(
                    badge_rect.center().x - galley.size().x / 2.0,
                    badge_rect.center().y - galley.size().y / 2.0,
                ),
                galley.clone(),
                text_color,
            );
        }

        // --- Path ---------------------------------------------------------
        if let Some((path_rect, galley)) = &geom.path {
            crate::ui::snap::galley_snapped(
                ui.painter(),
                path_rect.min,
                galley.clone(),
                with_alpha(colors.text_dim, row_alpha),
            );
        }

        // --- Close ------------------------------------------------------------
        // The glyph is centered in the same rect the pointer is tested against,
        // so the target is exactly where the X appears.
        let in_close = geom
            .close
            .is_some_and(|close| ui.rect_contains_pointer(close));
        if let Some(close) = geom.close {
            if hovered && !self.is_dragging {
                paint_text_centered(
                    ui,
                    "\u{2715}",
                    egui::FontId::proportional(CLOSE_GLYPH_SIZE),
                    with_alpha(
                        if in_close {
                            colors.text_primary
                        } else {
                            colors.text_dim
                        },
                        row_alpha,
                    ),
                    close.center(),
                );
            }
        }

        if in_close {
            response.clone().on_hover_text("Delete context");
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        } else {
            let content = Rect::from_min_max(
                rect.min,
                Pos2::new(geom.close.map_or(rect.max.x, |c| c.left()), rect.max.y),
            );
            if ui.rect_contains_pointer(content) || self.is_dragging {
                ui.ctx().set_cursor_icon(if self.draggable {
                    if self.any_dragging {
                        CursorIcon::Grabbing
                    } else {
                        CursorIcon::Grab
                    }
                } else {
                    CursorIcon::PointingHand
                });
            }
        }

        let action = if self.draggable && response.double_clicked() {
            SidebarAction::Rename
        } else if self.draggable && response.drag_started() {
            SidebarAction::DragStart
        } else if self.draggable && response.drag_stopped() {
            SidebarAction::DragEnd
        } else if response.clicked() && in_close && hovered {
            SidebarAction::Delete
        } else if response.clicked() {
            SidebarAction::Activate
        } else {
            SidebarAction::None
        };

        (action, response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_dots(pane_count: usize, windows: Vec<PaneDotWindow>) -> PaneDots {
        PaneDots {
            count: pane_count,
            focused_idx: Some(0),
            hidden_set: std::collections::HashSet::new(),
            activities: vec![None; pane_count],
            windows,
        }
    }

    fn one_window(pane_count: usize) -> Vec<PaneDotWindow> {
        vec![PaneDotWindow {
            start: 0,
            count: pane_count,
            is_return_target: true,
            is_active: true,
        }]
    }

    fn row(action_enabled: bool, badge_count: usize, pane_count: usize) -> SidebarRow {
        SidebarRow {
            is_active: true,
            is_dragging: false,
            any_dragging: false,
            action_enabled,
            ctx_name: "Default".to_string(),
            ctx_index: Some(0),
            badge_count,
            subtitle: Some("/Users/someone/code/project".to_string()),
            pane_dots: (pane_count > 0).then(|| test_dots(pane_count, one_window(pane_count))),
            draggable: true,
        }
    }

    /// Run `f` inside a real egui frame at `panel_w` points wide.
    fn in_frame<R>(panel_w: f32, f: impl FnOnce(&mut egui::Ui) -> R) -> R {
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(panel_w, 600.0))),
            ..Default::default()
        };
        let ctx = egui::Context::default();
        let mut f = Some(f);
        let mut out = None;
        let _ = ctx.run_ui(input, |ui| {
            if let Some(f) = f.take() {
                out = Some(f(ui));
            }
        });
        out.expect("frame body ran")
    }

    /// Lay one context row out in a `panel_w`-wide viewport and return the
    /// width the row actually claimed.
    fn measure_row(
        panel_w: f32,
        action_enabled: bool,
        badge_count: usize,
        pane_count: usize,
    ) -> f32 {
        let colors = Colors::from_config(&crate::config::ThemeConfig::default());
        in_frame(panel_w, |ui| {
            row(action_enabled, badge_count, pane_count)
                .show(ui, Id::new("row"), &colors)
                .1
                .rect
                .width()
        })
    }

    /// Stint 0715: a row wider than the space it was given inflates the
    /// enclosing panel, because egui stores a panel's content rect as the
    /// panel's size and reads it back as the width on the next frame. No
    /// combination of badge, pips, and close zone may exceed the budget.
    #[test]
    fn context_row_never_exceeds_available_width() {
        for &panel_w in &[160.0_f32, 220.0, 320.0, 400.0] {
            for &action_enabled in &[false, true] {
                for &badge_count in &[0_usize, 3, 42] {
                    for &pane_count in &[0_usize, 1, PANE_DOT_MAX + 3] {
                        let row_w = measure_row(panel_w, action_enabled, badge_count, pane_count);
                        assert!(
                            row_w <= panel_w,
                            "row overflowed: panel_w={panel_w} row_w={row_w} \
                             action_enabled={action_enabled} badge={badge_count} panes={pane_count}"
                        );
                    }
                }
            }
        }
    }

    /// The two tiers each hold their own things and nothing crosses between
    /// them: the close action owns the identity tier's right edge, and the
    /// pane-state tier lays pips, badge, and path out left to right without
    /// any pair ever overlapping, at every panel width.
    #[test]
    fn tiers_never_overlap_at_any_width() {
        for &panel_w in &[140.0_f32, 180.0, 220.0, 320.0, 480.0] {
            for &action_enabled in &[false, true] {
                for &badge_count in &[0_usize, 4, 42] {
                    for &pane_count in &[0_usize, 1, 5, PANE_DOT_MAX + 3] {
                        in_frame(panel_w, |ui| {
                            let item = row(action_enabled, badge_count, pane_count);
                            let geom = RowGeometry::measure(ui, &item, panel_w);
                            let label = format!(
                                "panel_w={panel_w} action={action_enabled} \
                                 badge={badge_count} panes={pane_count}"
                            );

                            // Identity tier.
                            if let Some(close) = geom.close {
                                assert!(
                                    geom.title.right() <= close.left() + f32::EPSILON,
                                    "name runs into the close action ({label})"
                                );
                                assert!(
                                    close.right() <= geom.rect.right(),
                                    "close escapes the row ({label})"
                                );
                                assert!(
                                    close.center().y < geom.rect.center().y,
                                    "close must sit on the identity tier ({label})"
                                );
                            }

                            // Pane-state tier, left to right.
                            let mut cursor = geom.gutter.right();
                            for (name, rect) in [
                                (
                                    "pips",
                                    geom.pips.as_ref().map(|(origin, layout)| {
                                        Rect::from_min_size(
                                            Pos2::new(origin.x, origin.y - layout.height / 2.0),
                                            Vec2::new(layout.width, layout.height),
                                        )
                                    }),
                                ),
                                ("badge", geom.badge.as_ref().map(|(rect, _)| *rect)),
                                ("path", geom.path.as_ref().map(|(rect, _)| *rect)),
                            ] {
                                let Some(rect) = rect else { continue };
                                assert!(
                                    rect.left() >= cursor - f32::EPSILON,
                                    "{name} overlaps what precedes it ({label})"
                                );
                                assert!(
                                    rect.right() <= geom.rect.right() + f32::EPSILON,
                                    "{name} escapes the row ({label})"
                                );
                                assert!(
                                    rect.center().y > geom.title.bottom(),
                                    "{name} must sit below the identity tier ({label})"
                                );
                                cursor = rect.right();
                            }
                        });
                    }
                }
            }
        }
    }

    /// The index number shares the title's optical center, not the whole
    /// row's — with a pane-state tier underneath, row-centering reads as the
    /// number sitting low beside the name.
    #[test]
    fn index_gutter_is_centered_on_the_title_line() {
        in_frame(220.0, |ui| {
            let item = row(true, 3, 4);
            let geom = RowGeometry::measure(ui, &item, 220.0);
            assert!(
                geom.pips.is_some(),
                "setup: this row has a pane-state tier below the title"
            );
            assert!(
                (geom.gutter.center().y - geom.title.center().y).abs() < 0.01,
                "gutter center {} must match title center {}",
                geom.gutter.center().y,
                geom.title.center().y
            );
            assert!(
                geom.gutter.center().y < geom.rect.center().y,
                "a two-tier row's title sits above the row center, so the \
                 number must too"
            );
        });
    }

    /// The badge cannot be mistaken for a collapsed pip strip: it is a taller
    /// filled pill, and it is separated from the pips by a visible gap rather
    /// than butted against them.
    #[test]
    fn badge_is_visually_distinct_from_collapsed_pips() {
        in_frame(220.0, |ui| {
            let item = row(true, 4, PANE_DOT_MAX + 3);
            let geom = RowGeometry::measure(ui, &item, 220.0);
            let (pip_origin, layout) = geom.pips.as_ref().expect("many panes render pips");
            let (badge, _) = geom.badge.as_ref().expect("a badge count renders a badge");
            assert!(
                layout.overflow.is_some(),
                "setup: this many panes must collapse into an overflow count"
            );
            assert!(
                badge.left() - (pip_origin.x + layout.width) >= PIP_BADGE_GAP - 0.01,
                "the badge must not read as glued to the pip strip"
            );
            assert!(
                badge.height() > PANE_DOT_RADIUS * 2.0 + 1.0,
                "the badge pill must be taller than a pip so the two never \
                 read as the same kind of thing"
            );
        });
    }

    /// The pip strip's width is measured once and reused: the budget the tier
    /// is laid out against is the same number the capsules are painted with.
    #[test]
    fn pip_strip_width_is_measured_once() {
        in_frame(320.0, |ui| {
            let item = row(true, 0, 5);
            let geom = RowGeometry::measure(ui, &item, 320.0);
            let (origin, layout) = geom.pips.as_ref().expect("five panes render pips");
            let last = layout.groups.last().expect("one capsule per window");
            assert!(
                (last.x + last.width - layout.width).abs() < 0.01,
                "capsules must fill exactly the width the strip reserved"
            );
            assert!(
                (origin.x - geom.gutter.right()).abs() < 0.01,
                "the strip must start at the content lane, under the name"
            );
        });
    }

    /// Regression: session restore can leave focus on a pane past PANE_DOT_MAX
    /// (the strip caps at 8 dots, focused_idx == 8). The pin must be skipped,
    /// not index out of bounds.
    #[test]
    fn pips_focused_pane_past_dot_cap_does_not_panic() {
        let dots = PaneDots {
            focused_idx: Some(PANE_DOT_MAX),
            ..test_dots(PANE_DOT_MAX + 1, one_window(PANE_DOT_MAX + 1))
        };
        let layout = PipLayout::measure(&dots, 200.0).expect("nine panes render pips");
        assert_eq!(layout.overflow, Some(1));
        assert!(layout.dot_center_x(PANE_DOT_MAX).is_none());

        let colors = Colors::from_config(&crate::config::ThemeConfig::default());
        let ctx = egui::Context::default();
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            egui::CentralPanel::default().show_inside(ui, |ui| {
                paint_pips(
                    ui,
                    &dots,
                    Pos2::new(10.0, 20.0),
                    &layout,
                    &colors,
                    1.0,
                    false,
                );
            });
        });
    }

    /// Multi-window contexts get one capsule per window, in order, separated by
    /// a visible gap — the boundary is what tells a user which panes will come
    /// back together.
    #[test]
    fn each_spatial_window_gets_its_own_capsule() {
        let dots = test_dots(
            5,
            vec![
                PaneDotWindow {
                    start: 0,
                    count: 2,
                    is_return_target: true,
                    is_active: true,
                },
                PaneDotWindow {
                    start: 2,
                    count: 3,
                    is_return_target: false,
                    is_active: false,
                },
            ],
        );
        let layout = PipLayout::measure(&dots, 200.0).expect("five panes render pips");
        assert_eq!(layout.groups.len(), 2);
        assert!(
            (layout.groups[1].x - (layout.groups[0].x + layout.groups[0].width))
                >= WINDOW_GROUP_GAP - 0.01,
            "capsules must be separated by the window gap"
        );
        for idx in 0..5 {
            assert!(
                layout.dot_center_x(idx).is_some(),
                "every visible dot belongs to a capsule"
            );
        }
    }
}

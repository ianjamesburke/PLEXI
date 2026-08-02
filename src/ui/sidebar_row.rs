use crate::ui::list::{paint_selection, paint_text_centered};
use crate::ui::style;
use crate::ui::theme::Colors;
use egui::emath::GuiRounding;
use egui::{Align, Color32, CornerRadius, CursorIcon, Id, Layout, Rect, Sense, Vec2};

pub const ACTION_ZONE_WIDTH: f32 = 22.0;

/// Fixed-width left gutter that holds the context index number.
const GUTTER_W: f32 = 18.0;
const ROW_INDENT: f32 = 4.0;

/// Top and bottom padding added inside each row for vertical breathing room.
const ROW_PAD_V: f32 = 7.0;

/// Status-pip radius. Shared with the portal minimap so activity dots are the
/// same size everywhere they appear (sidebar rows + portal previews).
pub(crate) const PANE_DOT_RADIUS: f32 = 4.0;
const PANE_DOT_SPACING: f32 = 11.0;
const PANE_DOT_MAX: usize = 8;
const WINDOW_GROUP_PAD_X: f32 = 5.0;
const WINDOW_GROUP_PAD_Y: f32 = 4.0;
const WINDOW_GROUP_GAP: f32 = 5.0;
const SIDEBAR_BADGE_W: f32 = 18.0;
/// Gap between the context name and the pane-pip strip on the headline.
const PIP_TEXT_GAP: f32 = 4.0;

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

pub struct ContextItem {
    pub is_active: bool,
    pub is_dragging: bool,
    pub any_dragging: bool,
    pub action_enabled: bool,
    pub ctx_name: String,
    pub ctx_index: Option<usize>,
    pub badge_count: usize,
    /// Root path shown on its own line below the name row.
    pub subtitle: Option<String>,
    /// Dots rendered below the name row representing panes.
    pub pane_dots: Option<PaneDots>,
    /// Whether this row supports drag reordering. When false, hover shows
    /// PointingHand instead of Grab and drag actions are suppressed.
    pub draggable: bool,
}

/// Render pane-indicator dots into the current horizontal layout position.
/// Shared by both the subtitle+pips path and the pips-only fallback path.
fn draw_pips(
    ui: &mut egui::Ui,
    pane_dots: &Option<PaneDots>,
    colors: &Colors,
    row_alpha: f32,
    is_dragging: bool,
) {
    let Some(dots) = pane_dots else { return };
    if dots.count == 0 {
        return;
    }
    let capped = dots.count.min(PANE_DOT_MAX);
    let groups: Vec<&PaneDotWindow> = dots
        .windows
        .iter()
        .filter(|group| group.count > 0 && group.start < capped)
        .collect();
    let group_count = groups.len().max(1);
    let mut dot_area_width = (capped as f32) * PANE_DOT_SPACING
        + (group_count as f32) * WINDOW_GROUP_PAD_X * 2.0
        + (group_count.saturating_sub(1) as f32) * WINDOW_GROUP_GAP;
    if dots.count > PANE_DOT_MAX {
        dot_area_width += 20.0;
    }
    let dot_size = Vec2::new(dot_area_width, PANE_DOT_RADIUS * 2.0 + 4.0);
    let (rect, _) = ui.allocate_exact_size(dot_size, Sense::hover());
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
    let cy = rect
        .center()
        .y
        .round_to_pixel_center(painter.pixels_per_point());
    let mut dot_centers = vec![0.0; capped];
    let mut group_rects = Vec::with_capacity(groups.len());
    let mut cursor_x = rect.min.x;
    if groups.is_empty() {
        for (dot_i, center) in dot_centers.iter_mut().enumerate() {
            *center = (cursor_x + WINDOW_GROUP_PAD_X + PANE_DOT_RADIUS)
                .round_to_pixel_center(painter.pixels_per_point());
            cursor_x += PANE_DOT_SPACING;
            if dot_i + 1 == capped {
                let group_rect = egui::Rect::from_min_max(
                    egui::pos2(rect.min.x, cy - PANE_DOT_RADIUS - WINDOW_GROUP_PAD_Y),
                    egui::pos2(
                        cursor_x - PANE_DOT_SPACING
                            + PANE_DOT_RADIUS * 2.0
                            + WINDOW_GROUP_PAD_X * 2.0,
                        cy + PANE_DOT_RADIUS + WINDOW_GROUP_PAD_Y,
                    ),
                )
                .round_to_pixels(painter.pixels_per_point());
                painter.rect_filled(
                    group_rect,
                    egui::CornerRadius::same(4),
                    with_alpha(colors.bg_hover, 0.7 * row_alpha),
                );
            }
        }
    } else {
        for (group_i, group) in groups.iter().enumerate() {
            let start = group.start;
            let end = (group.start + group.count).min(capped);
            let group_width = (end - start - 1) as f32 * PANE_DOT_SPACING
                + PANE_DOT_RADIUS * 2.0
                + WINDOW_GROUP_PAD_X * 2.0;
            let group_rect = egui::Rect::from_min_size(
                egui::pos2(cursor_x, cy - PANE_DOT_RADIUS - WINDOW_GROUP_PAD_Y),
                egui::vec2(
                    group_width,
                    PANE_DOT_RADIUS * 2.0 + WINDOW_GROUP_PAD_Y * 2.0,
                ),
            )
            .round_to_pixels(painter.pixels_per_point());
            let fill = if group.is_active {
                with_alpha(colors.accent, 0.16 * row_alpha)
            } else {
                // Keep group fill darker than a hovered row, so hover remains
                // a row-level wash instead of erasing window boundaries.
                with_alpha(colors.bg_darkest, 0.3 * row_alpha)
            };
            let stroke = if group.is_active {
                egui::Stroke::new(1.0_f32, with_alpha(colors.accent, row_alpha))
            } else if group.is_return_target {
                egui::Stroke::new(
                    1.5_f32,
                    // `border` is the theme's quiet structural outline; the
                    // previous high-contrast text color made this capsule
                    // compete with the context name and activity dots.
                    with_alpha(colors.border, row_alpha),
                )
            } else {
                egui::Stroke::new(1.0_f32, with_alpha(colors.text_dim, 0.72 * row_alpha))
            };
            painter.rect_filled(group_rect, egui::CornerRadius::same(4), fill);
            painter.rect_stroke(
                group_rect,
                egui::CornerRadius::same(4),
                stroke,
                egui::StrokeKind::Inside,
            );
            group_rects.push((group, group_rect));
            for (dot_i, center) in dot_centers
                .iter_mut()
                .enumerate()
                .take(end)
                .skip(start)
            {
                *center = (cursor_x
                    + WINDOW_GROUP_PAD_X
                    + PANE_DOT_RADIUS
                    + (dot_i - start) as f32 * PANE_DOT_SPACING)
                    .round_to_pixel_center(painter.pixels_per_point());
            }
            cursor_x += group_width;
            if group_i + 1 < groups.len() {
                cursor_x += WINDOW_GROUP_GAP;
            }
        }
    }
    for (dot_i, &cx) in dot_centers.iter().enumerate().take(capped) {
        let is_hidden = dots.hidden_set.contains(&dot_i);
        let agent_state = dots.activities.get(dot_i).and_then(|s| s.as_ref());
        let focused = dots.focused_idx == Some(dot_i);
        let mut color = crate::ui::activity::pip_color(agent_state, focused, colors, t, dot_i)
            .gamma_multiply(row_alpha);
        if is_dragging && agent_state.is_none() && !focused {
            color = color.gamma_multiply(0.4);
        }
        let center = egui::pos2(cx, cy).round_to_pixel_center(painter.pixels_per_point());
        if is_hidden {
            painter.circle_stroke(center, PANE_DOT_RADIUS, egui::Stroke::new(1.0_f32, color));
        } else {
            painter.circle_filled(center, PANE_DOT_RADIUS, color);
        }
    }
    // The pin is intentionally neutral: dot color describes agent activity,
    // while this tiny direction marker describes where a context will return.
    if let Some(focused_idx) = dots.focused_idx {
        if let Some((group, group_rect)) = group_rects.iter().find(|(group, _)| {
            group.is_return_target
                && (group.start..group.start + group.count).contains(&focused_idx)
        }) {
            // `focused_idx` indexes the full pane list, but `dot_centers` is
            // capped at PANE_DOT_MAX. A focused pane past the cap has no dot in
            // the strip — it is represented by the "+N" overflow label (which
            // highlights via `overflow_color` below), so the pin is skipped.
            if let Some(&cx) = dot_centers.get(focused_idx) {
                let tip_y = (cy - PANE_DOT_RADIUS - 0.5).max(group_rect.min.y + 2.0);
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
                        egui::pos2(cx - 2.5, group_rect.min.y + 1.5),
                        egui::pos2(cx + 2.5, group_rect.min.y + 1.5),
                        egui::pos2(cx, tip_y),
                    ],
                    pin_color,
                    egui::Stroke::NONE,
                ));
            }
        }
    }
    if dots.count > PANE_DOT_MAX {
        let overflow_x = cursor_x + PANE_DOT_RADIUS * 0.5;
        let overflow_color = if dots.focused_idx.is_some_and(|idx| idx >= PANE_DOT_MAX) {
            with_alpha(colors.accent, row_alpha)
        } else {
            with_alpha(colors.text_dim, 0.5)
        };
        crate::ui::snap::text_snapped(
            painter,
            egui::pos2(overflow_x, cy),
            egui::Align2::LEFT_CENTER,
            format!("+{}", dots.count - PANE_DOT_MAX),
            egui::FontId::proportional(style::TEXT_META),
            overflow_color,
        );
    }
}

impl ContextItem {
    pub fn draw(
        self,
        ui: &mut egui::Ui,
        id: Id,
        colors: &Colors,
    ) -> (SidebarAction, egui::Response) {
        let row_alpha = if self.is_dragging { 0.4_f32 } else { 1.0_f32 };

        // Reserve background shape slot before rendering content.
        let bg_idx = ui.painter().add(egui::Shape::Noop);

        // Every sidebar row is a top-level context, so the gutter is a fixed
        // There is no nesting level to indent for, so the compact inset leaves
        // room for the primary label rather than a second visual gutter.
        let indent = ROW_INDENT;

        let is_active = self.is_active;
        let is_dragging = self.is_dragging;
        let any_dragging = self.any_dragging;
        let action_enabled = self.action_enabled;
        let ctx_name = self.ctx_name.clone();
        let ctx_index = self.ctx_index;
        let badge_count = self.badge_count;
        let subtitle = self.subtitle.clone();
        let pane_dots = self.pane_dots;
        let accent_color = colors.accent;
        let text_primary = colors.text_primary;
        let text_dim = colors.text_dim;
        let bg_active = colors.bg_active;
        let bg_hover = colors.bg_hover;

        // Context names are primary labels: inactive rows read constantly, so
        // they get the contrast-floored secondary tone, not raw `text_dim`
        // (stint 0528).
        let text_color = with_alpha(
            if is_active {
                text_primary
            } else {
                colors.text_secondary(colors.bg_sidebar)
            },
            row_alpha,
        );

        let scope_out = ui.scope(|ui| {
            ui.set_width(ui.available_width());

            ui.add_space(ROW_PAD_V);

            // --- Outer row: left gutter (index number) + content column ---
            let y_before = ui.cursor().min.y;
            // name_row_h and close_rect are captured from inside the horizontal
            // closure so the number and delete hit target stay on the headline,
            // not the full two-line row.
            let mut name_row_h_capture = 0.0_f32;
            let mut close_rect_capture = None;
            ui.horizontal(|ui| {
                ui.add_space(indent);

                // Gutter: fixed-width column for the 1-9 index number.
                // Width is reserved here; the number is painted post-scope
                // centered across the full row height.
                ui.add_space(GUTTER_W);

                // Content column: name row + optional path row.
                // Pips live on the name row, right-aligned before the action zone.
                let name_row_h = ui
                    .vertical(|ui| {
                        // --- Name row ---
                        let name_y_before = ui.cursor().min.y;
                        ui.horizontal(|ui| {
                            // Every horizontal gap on this row is budgeted
                            // explicitly below, so implicit item spacing must be
                            // off: an unbudgeted gap makes the row wider than
                            // the panel, egui stores that overflowed content
                            // rect as the panel's size, and the panel re-reads
                            // it as its width next frame — a feedback loop that
                            // ramps the sidebar to its size_range maximum and
                            // swallows any user resize (stint 0715).
                            let gap = ui.spacing().item_spacing.x;
                            ui.spacing_mut().item_spacing.x = 0.0;

                            // Compute pip strip width so we can budget the name text.
                            let pip_strip_w = if let Some(ref dots) = pane_dots {
                                if dots.count > 0 {
                                    let capped = dots.count.min(PANE_DOT_MAX);
                                    let group_count = dots
                                        .windows
                                        .iter()
                                        .filter(|group| group.count > 0 && group.start < capped)
                                        .count()
                                        .max(1);
                                    let mut w = (capped as f32) * PANE_DOT_SPACING
                                        + (group_count as f32) * WINDOW_GROUP_PAD_X * 2.0
                                        + (group_count.saturating_sub(1) as f32) * WINDOW_GROUP_GAP;
                                    if dots.count > PANE_DOT_MAX {
                                        w += 20.0;
                                    }
                                    w + PIP_TEXT_GAP
                                } else {
                                    0.0
                                }
                            } else {
                                0.0
                            };

                            let badge_w = if badge_count > 0 {
                                SIDEBAR_BADGE_W
                            } else {
                                0.0
                            };
                            let close_w = if action_enabled {
                                ACTION_ZONE_WIDTH
                            } else {
                                0.0
                            };
                            // The trailing block keeps normal item spacing
                            // between the close zone and the badge, so that gap
                            // is part of its budget too.
                            let trailing_w = badge_w
                                + close_w
                                + if badge_w > 0.0 && close_w > 0.0 {
                                    gap
                                } else {
                                    0.0
                                };
                            let text_max =
                                (ui.available_width() - trailing_w - pip_strip_w).max(0.0);

                            let title_width = ui
                                .scope(|ui| {
                                    ui.set_max_width(text_max);
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(&ctx_name)
                                                .size(style::TEXT_SIDEBAR_TITLE)
                                                .color(text_color),
                                        )
                                        .selectable(false)
                                        .truncate(),
                                    )
                                    .rect
                                    .width()
                                })
                                .inner;
                            // Keep the trailing layout at the budget boundary
                            // even for short names, so it cannot begin after
                            // the pips and overlap the count.
                            ui.add_space((text_max - title_width).max(0.0));

                            // Pips stay beside the headline, before the trailing
                            // state. `pip_strip_w` budgeted this gap; with
                            // implicit spacing off it has to be added by hand.
                            if pip_strip_w > 0.0 {
                                ui.add_space(PIP_TEXT_GAP);
                            }
                            draw_pips(ui, &pane_dots, colors, row_alpha, is_dragging);

                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.spacing_mut().item_spacing.x = gap;
                                // The close target follows the count at the physical trailing edge.
                                if action_enabled {
                                    let (rect, _) = ui.allocate_exact_size(
                                        Vec2::new(ACTION_ZONE_WIDTH, ui.spacing().interact_size.y),
                                        Sense::hover(),
                                    );
                                    close_rect_capture = Some(rect);
                                }
                                if badge_count > 0 {
                                    let badge_text = if badge_count > 9 {
                                        "9+".to_string()
                                    } else {
                                        badge_count.to_string()
                                    };
                                    ui.add_sized(
                                        Vec2::new(SIDEBAR_BADGE_W, 0.0),
                                        egui::Label::new(
                                            egui::RichText::new(badge_text)
                                                .size(style::TEXT_META)
                                                .color(with_alpha(accent_color, row_alpha)),
                                        )
                                        .halign(Align::RIGHT),
                                    );
                                }
                            });
                        });
                        // Capture name-row height before the subtitle row is added.
                        let name_h = ui.cursor().min.y - name_y_before;

                        // --- Path row (subtitle only, no pips) ---
                        if let Some(ref path) = subtitle {
                            ui.horizontal(|ui| {
                                let path_max = (ui.available_width() - style::SPACE_SM).max(0.0);
                                ui.scope(|ui| {
                                    ui.set_max_width(path_max);
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(shorten_path(path))
                                                .size(style::TEXT_META)
                                                .color(with_alpha(text_dim, row_alpha)),
                                        )
                                        .selectable(false)
                                        .truncate(),
                                    );
                                });
                            });
                        }

                        name_h
                    })
                    .inner;
                name_row_h_capture = name_row_h;
            });

            ui.add_space(ROW_PAD_V);

            (
                ui.cursor().min.y - y_before,
                name_row_h_capture,
                close_rect_capture,
            )
        });

        let row_rect = scope_out.response.rect;
        let (_total_h, name_row_h, close_rect) = scope_out.inner;

        let response = ui.interact(row_rect, id, Sense::click_and_drag());
        let hovered = response.hovered();

        // Selection: use paint_selection (inset tinted pill + soft outline) to
        // match palette/picker rows. Hover keeps the flat sidebar fill.
        if is_active {
            // Clear the bg placeholder — paint_selection draws its own rect.
            ui.painter().set(bg_idx, egui::Shape::Noop);
            if row_alpha < 1.0 {
                // Dragging: just a dim solid fill, no outline.
                ui.painter().rect_filled(
                    row_rect,
                    CornerRadius::ZERO,
                    with_alpha(bg_active, row_alpha),
                );
            } else {
                // 4px horizontal inset gives the pill breathing room from the
                // sidebar edges, matching the visual weight of palette rows.
                let pill_rect = Rect::from_min_max(
                    egui::pos2(row_rect.min.x + 4.0, row_rect.min.y),
                    egui::pos2(row_rect.max.x - 4.0, row_rect.max.y),
                );
                paint_selection(ui.painter(), pill_rect, colors);
            }
        } else {
            let fill = if hovered && !is_dragging {
                with_alpha(bg_hover, row_alpha)
            } else {
                Color32::TRANSPARENT
            };
            let hover_rect = crate::ui::list::selection_inset(Rect::from_min_max(
                egui::pos2(row_rect.min.x + 4.0, row_rect.min.y),
                egui::pos2(row_rect.max.x - 4.0, row_rect.max.y),
            ));
            ui.painter().set(
                bg_idx,
                egui::Shape::rect_filled(hover_rect, crate::ui::style::RADIUS_SM, fill),
            );
        }

        // Paint the index number centered on the title line. Rows can include
        // subtitle/pip content below the title, so full-row centering makes the
        // number read as too low beside the context name.
        if let Some(idx) = ctx_index {
            if idx < 9 {
                let title_center_y = row_rect.min.y + ROW_PAD_V + name_row_h / 2.0;
                let gutter_rect = Rect::from_min_size(
                    egui::pos2(row_rect.min.x + indent, row_rect.min.y),
                    Vec2::new(GUTTER_W, ROW_PAD_V + name_row_h),
                );
                paint_text_centered(
                    ui,
                    format!("{}", idx + 1),
                    egui::FontId::proportional(style::TEXT_HINT),
                    with_alpha(text_dim, row_alpha),
                    egui::pos2(gutter_rect.center().x, title_center_y),
                );
            }
        }

        let in_action = close_rect.is_some_and(|rect| ui.rect_contains_pointer(rect));

        if let Some(close_rect) = close_rect {
            if hovered && !is_dragging {
                let glyph_color =
                    with_alpha(if in_action { text_primary } else { text_dim }, row_alpha);
                paint_text_centered(
                    ui,
                    "\u{2715}",
                    egui::FontId::proportional(style::TEXT_SIDEBAR_TITLE),
                    glyph_color,
                    close_rect.center(),
                );
            }
        }

        if in_action {
            response.clone().on_hover_text("Delete context");
        }

        if in_action {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        } else {
            let content_max_x = if action_enabled {
                row_rect.max.x - ACTION_ZONE_WIDTH
            } else {
                row_rect.max.x
            };
            let in_content = ui.rect_contains_pointer(Rect::from_min_max(
                row_rect.min,
                egui::pos2(content_max_x, row_rect.max.y),
            ));
            if in_content || is_dragging {
                ui.ctx().set_cursor_icon(if self.draggable {
                    if any_dragging {
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
        } else if response.clicked() && in_action && hovered {
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

    /// Lay one context row out in a `panel_w`-wide viewport and return the
    /// width the row actually claimed.
    fn measure_row(
        panel_w: f32,
        action_enabled: bool,
        badge_count: usize,
        pane_count: usize,
    ) -> f32 {
        let colors = Colors::from_config(&crate::config::ThemeConfig::default());
        let mut pane_dots = (pane_count > 0).then(|| PaneDots {
            count: pane_count,
            focused_idx: Some(0),
            hidden_set: std::collections::HashSet::new(),
            activities: vec![None; pane_count],
            windows: vec![PaneDotWindow {
                start: 0,
                count: pane_count,
                is_return_target: true,
                is_active: true,
            }],
        });
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                Vec2::new(panel_w, 600.0),
            )),
            ..Default::default()
        };
        let ctx = egui::Context::default();
        let mut row_w = 0.0_f32;
        let _ = ctx.run_ui(input, |ui| {
            let (_, response) = ContextItem {
                is_active: true,
                is_dragging: false,
                any_dragging: false,
                action_enabled,
                ctx_name: "Default".to_string(),
                ctx_index: Some(0),
                badge_count,
                subtitle: Some("/Users/someone/code/project".to_string()),
                pane_dots: pane_dots.take(),
                draggable: true,
            }
            .draw(ui, Id::new("row"), &colors);
            row_w = response.rect.width();
        });
        row_w
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

    /// Regression: session restore can leave focus on a pane past PANE_DOT_MAX
    /// (dot_centers len == 8, focused_idx == 8). The pin must be skipped, not
    /// index out of bounds.
    #[test]
    fn draw_pips_focused_pane_past_dot_cap_does_not_panic() {
        let dots = Some(PaneDots {
            count: PANE_DOT_MAX + 1,
            focused_idx: Some(PANE_DOT_MAX),
            hidden_set: std::collections::HashSet::new(),
            activities: vec![None; PANE_DOT_MAX + 1],
            windows: vec![PaneDotWindow {
                start: 0,
                count: PANE_DOT_MAX + 1,
                is_return_target: true,
                is_active: true,
            }],
        });
        let colors = Colors::from_config(&crate::config::ThemeConfig::default());
        let ctx = egui::Context::default();
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            egui::CentralPanel::default().show_inside(ui, |ui| {
                draw_pips(ui, &dots, &colors, 1.0, false);
            });
        });
    }
}

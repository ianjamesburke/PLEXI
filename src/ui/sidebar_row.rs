use crate::ui::list::paint_selection;
use crate::ui::theme::Colors;
use egui::{Align, Color32, CornerRadius, CursorIcon, Id, Layout, Rect, Sense, Vec2};

pub const ACTION_ZONE_WIDTH: f32 = 30.0;

/// Fixed-width left gutter that holds the context index number.
const GUTTER_W: f32 = 24.0;

/// Top and bottom padding added inside each row for vertical breathing room.
const ROW_PAD_V: f32 = 7.0;

const PANE_DOT_RADIUS: f32 = 4.0;
const PANE_DOT_SPACING: f32 = 11.0;
const PANE_DOT_MAX: usize = 8;

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
    /// Nesting depth for subcontexts (0 = top-level).
    pub indent: u32,
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
    let mut dot_area_width = (capped as f32) * PANE_DOT_SPACING;
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
    let cy = rect.center().y;
    for dot_i in 0..capped {
        let cx = rect.min.x + (dot_i as f32) * PANE_DOT_SPACING + PANE_DOT_RADIUS;
        let is_hidden = dots.hidden_set.contains(&dot_i);
        let agent_state = dots.activities.get(dot_i).and_then(|s| s.as_ref());
        let focused = dots.focused_idx == Some(dot_i);
        let mut color =
            crate::ui::activity::pip_color(agent_state, focused, colors, t).gamma_multiply(row_alpha);
        if is_dragging && agent_state.is_none() && !focused {
            color = color.gamma_multiply(0.4);
        }
        let center = egui::pos2(cx, cy);
        if is_hidden {
            painter.circle_stroke(center, PANE_DOT_RADIUS, egui::Stroke::new(1.0, color));
        } else {
            painter.circle_filled(center, PANE_DOT_RADIUS, color);
        }
    }
    if dots.count > PANE_DOT_MAX {
        let overflow_x =
            rect.min.x + (capped as f32) * PANE_DOT_SPACING + PANE_DOT_RADIUS * 0.5;
        let overflow_color =
            if dots.focused_idx.map_or(false, |idx| idx >= PANE_DOT_MAX) {
                with_alpha(colors.accent, row_alpha)
            } else {
                with_alpha(colors.text_dim, 0.5)
            };
        painter.text(
            egui::pos2(overflow_x, cy),
            egui::Align2::LEFT_CENTER,
            format!("+{}", dots.count - PANE_DOT_MAX),
            egui::FontId::proportional(8.0),
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

        // 6px base + 16px per nesting level — keeps the gutter number close to
        // the sidebar edge for top-level contexts.
        let indent = 6.0 + 16.0 * self.indent as f32;

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

        let text_color = with_alpha(if is_active { text_primary } else { text_dim }, row_alpha);

        let scope_out = ui.scope(|ui| {
            ui.set_width(ui.available_width());

            ui.add_space(ROW_PAD_V);

            // --- Outer row: left gutter (index number) + content column ---
            let y_before = ui.cursor().min.y;
            // name_row_h is captured from inside the horizontal closure and
            // returned as part of the scope inner so the action zone can be
            // limited to the header line height.
            let mut name_row_h_capture = 0.0_f32;
            ui.horizontal(|ui| {
                ui.add_space(indent);

                // Gutter: fixed-width column for the 1-9 index number.
                // Width is reserved here; the number is painted post-scope
                // centered across the full row height.
                ui.add_space(GUTTER_W);

                // Content column: name row + optional path row.
                // Pips live on the name row, right-aligned before the action zone.
                let name_row_h = ui.vertical(|ui| {
                    // --- Name row (includes pips right-aligned) ---
                    let name_y_before = ui.cursor().min.y;
                    ui.horizontal(|ui| {
                        // Compute pip strip width so we can budget the name text.
                        let pip_strip_w = if let Some(ref dots) = pane_dots {
                            if dots.count > 0 {
                                let capped = dots.count.min(PANE_DOT_MAX);
                                let mut w = (capped as f32) * PANE_DOT_SPACING;
                                if dots.count > PANE_DOT_MAX {
                                    w += 20.0;
                                }
                                w + 6.0 // gap between name text and dots
                            } else {
                                0.0
                            }
                        } else {
                            0.0
                        };

                        let right_reserve = if action_enabled {
                            ACTION_ZONE_WIDTH + 4.0
                        } else {
                            8.0
                        };
                        let badge_w = if badge_count > 0 { 26.0 } else { 0.0 };
                        let text_max = (ui.available_width()
                            - right_reserve
                            - badge_w
                            - pip_strip_w)
                            .max(0.0);

                        ui.scope(|ui| {
                            ui.set_max_width(text_max);
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(&ctx_name)
                                        .size(13.0)
                                        .color(text_color),
                                )
                                .selectable(false)
                                .truncate(),
                            );
                        });

                        // Pips: rendered inline right of the name, before badge/action.
                        draw_pips(ui, &pane_dots, &colors, row_alpha, is_dragging);

                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.add_space(if action_enabled { ACTION_ZONE_WIDTH } else { 0.0 });
                            if badge_count > 0 {
                                let badge_text = if badge_count > 9 {
                                    "9+".to_string()
                                } else {
                                    badge_count.to_string()
                                };
                                ui.label(
                                    egui::RichText::new(badge_text)
                                        .size(10.0)
                                        .color(with_alpha(accent_color, row_alpha)),
                                );
                            }
                        });
                    });
                    // Capture name-row height before the subtitle row is added.
                    let name_h = ui.cursor().min.y - name_y_before;

                    // --- Path row (subtitle only, no pips) ---
                    if let Some(ref path) = subtitle {
                        ui.horizontal(|ui| {
                            let path_max = (ui.available_width() - 8.0).max(0.0);
                            ui.scope(|ui| {
                                ui.set_max_width(path_max);
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(shorten_path(path))
                                            .size(10.0)
                                            .color(with_alpha(text_dim, row_alpha * 0.7)),
                                    )
                                    .selectable(false)
                                    .truncate(),
                                );
                            });
                        });
                    }

                    name_h
                }).inner;
                name_row_h_capture = name_row_h;
            });

            ui.add_space(ROW_PAD_V);

            (ui.cursor().min.y - y_before, name_row_h_capture)
        });

        let row_rect = scope_out.response.rect;
        let (_total_h, name_row_h) = scope_out.inner;

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
            let hover_rect = crate::ui::list::selection_inset(row_rect);
            ui.painter().set(
                bg_idx,
                egui::Shape::rect_filled(hover_rect, crate::ui::style::RADIUS_SM, fill),
            );
        }

        // Paint the index number centered vertically in the gutter.
        if let Some(idx) = ctx_index {
            if idx < 9 {
                let gutter_rect = Rect::from_min_size(
                    egui::pos2(row_rect.min.x + indent, row_rect.min.y),
                    Vec2::new(GUTTER_W, row_rect.height()),
                );
                ui.painter().text(
                    gutter_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    format!("{}", idx + 1),
                    egui::FontId::proportional(11.0),
                    with_alpha(text_dim, row_alpha),
                );
            }
        }

        let action_zone = if action_enabled {
            Some(Rect::from_min_max(
                egui::pos2(row_rect.max.x - ACTION_ZONE_WIDTH, row_rect.min.y),
                egui::pos2(row_rect.max.x, row_rect.min.y + name_row_h),
            ))
        } else {
            None
        };

        let in_action = action_zone.map_or(false, |az| ui.rect_contains_pointer(az));

        if let Some(az) = action_zone {
            if hovered && !is_dragging {
                let glyph_color =
                    with_alpha(if in_action { text_primary } else { text_dim }, row_alpha);
                ui.painter().text(
                    az.center(),
                    egui::Align2::CENTER_CENTER,
                    "\u{2715}",
                    egui::FontId::proportional(13.0),
                    glyph_color,
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
                ui.ctx().set_cursor_icon(if any_dragging {
                    CursorIcon::Grabbing
                } else {
                    CursorIcon::Grab
                });
            }
        }

        let action = if response.double_clicked() {
            SidebarAction::Rename
        } else if response.drag_started() {
            SidebarAction::DragStart
        } else if response.drag_stopped() {
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

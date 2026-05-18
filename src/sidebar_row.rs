use egui::{Align, Color32, CornerRadius, CursorIcon, Id, Layout, Rect, Sense, Vec2};
use crate::theme::Colors;

pub const ACTION_ZONE_WIDTH: f32 = 30.0;

const PANE_DOT_RADIUS: f32 = 2.5;
const PANE_DOT_SPACING: f32 = 8.0;
const PANE_DOT_MAX: usize = 6;

pub(crate) fn with_alpha(c: Color32, alpha: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (c.a() as f32 * alpha) as u8)
}

static HOME_DIR: std::sync::OnceLock<String> = std::sync::OnceLock::new();

fn shorten_path(path: &str) -> String {
    let home = HOME_DIR.get_or_init(|| std::env::var("HOME").unwrap_or_default());
    let shortened = if !home.is_empty() {
        path.strip_prefix(home.as_str()).map_or_else(|| path.to_string(), |rest| format!("~{rest}"))
    } else {
        path.to_string()
    };
    let char_count = shortened.chars().count();
    if char_count > 40 {
        let tail: String = shortened.chars().rev().take(39).collect::<Vec<_>>().into_iter().rev().collect();
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
}

pub struct ContextItem {
    pub is_active: bool,
    pub is_dragging: bool,
    pub any_dragging: bool,
    pub action_enabled: bool,
    pub ctx_depth: u32,
    pub ctx_name: String,
    pub ctx_index: Option<usize>,
    pub badge_count: usize,
    pub subtitle: Option<String>,
    pub pane_dots: Option<PaneDots>,
}

impl ContextItem {
    pub fn draw(self, ui: &mut egui::Ui, id: Id, colors: &Colors) -> (SidebarAction, egui::Response) {
        let row_alpha = if self.is_dragging { 0.4_f32 } else { 1.0_f32 };

        // Reserve background shape slot before rendering (same pattern as selectable_row in widgets.rs).
        // Frame inner_margin interferes with ScrollArea height measurement — use scope + shape slot instead.
        let bg_idx = ui.painter().add(egui::Shape::Noop);

        let indent = 20.0 + self.ctx_depth as f32 * 12.0;

        // Capture fields for use inside closure
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

        let text_color = with_alpha(
            if is_active { text_primary } else { text_dim },
            row_alpha,
        );

        let name_row_height = ui.scope(|ui| {
            ui.set_width(ui.available_width());

            // --- Section 1: Name row ---
            let y_before_name = ui.cursor().min.y;
            ui.horizontal(|ui| {
                ui.add_space(indent);
                if let Some(idx) = ctx_index {
                    if idx < 9 {
                        ui.label(
                            egui::RichText::new(format!("{}", idx + 1))
                                .size(11.0)
                                .color(with_alpha(text_dim, row_alpha)),
                        );
                    }
                }
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(&ctx_name).size(12.0).color(text_color)
                    )
                    .selectable(false),
                );
                if badge_count > 0 {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.add_space(8.0);
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
                    });
                }
            });
            let name_row_height = ui.cursor().min.y - y_before_name;

            // --- Section 2: Subtitle ---
            if let Some(ref path) = subtitle {
                ui.horizontal(|ui| {
                    ui.add_space(indent);
                    ui.label(
                        egui::RichText::new(shorten_path(path))
                            .size(9.5)
                            .color(with_alpha(text_dim, row_alpha)),
                    );
                });
            }

            // --- Section 3: Pane dots ---
            if let Some(ref dots) = pane_dots {
                if dots.count > 1 {
                    ui.horizontal(|ui| {
                        ui.add_space(indent);
                        let capped = dots.count.min(PANE_DOT_MAX);
                        let dot_area_width = (capped as f32) * PANE_DOT_SPACING;
                        let dot_size = Vec2::new(dot_area_width, PANE_DOT_RADIUS * 2.0 + 4.0);
                        let (rect, _) = ui.allocate_exact_size(dot_size, Sense::hover());
                        let painter = ui.painter();
                        let cy = rect.center().y;
                        for dot_i in 0..capped {
                            let cx = rect.min.x + (dot_i as f32) * PANE_DOT_SPACING + PANE_DOT_RADIUS;
                            let color = if dots.focused_idx == Some(dot_i) {
                                with_alpha(accent_color, row_alpha)
                            } else {
                                with_alpha(text_dim, if is_dragging { 0.15 } else { 0.35 })
                            };
                            painter.circle_filled(egui::pos2(cx, cy), PANE_DOT_RADIUS, color);
                        }
                        if dots.count > PANE_DOT_MAX {
                            let overflow_x = rect.min.x + (capped as f32) * PANE_DOT_SPACING + PANE_DOT_RADIUS * 0.5;
                            painter.text(
                                egui::pos2(overflow_x, cy),
                                egui::Align2::LEFT_CENTER,
                                format!("+{}", dots.count - PANE_DOT_MAX),
                                egui::FontId::proportional(8.0),
                                with_alpha(text_dim, 0.5),
                            );
                        }
                    });
                }
            }

            name_row_height
        });

        let row_rect = name_row_height.response.rect;
        let name_row_h = name_row_height.inner;

        // Interact on the full row
        let response = ui.interact(row_rect, id, Sense::click_and_drag());
        let hovered = response.hovered();

        // Determine background fill
        let fill = if is_active {
            with_alpha(bg_active, row_alpha)
        } else if hovered && !is_dragging {
            with_alpha(colors.bg_sidebar_hover, row_alpha)
        } else {
            Color32::TRANSPARENT
        };

        // Paint background behind content
        ui.painter().set(bg_idx, egui::Shape::rect_filled(row_rect, CornerRadius::ZERO, fill));

        // Active accent bar — full row height
        if is_active {
            ui.painter().rect_filled(
                Rect::from_min_size(row_rect.min, Vec2::new(3.0, row_rect.height())),
                CornerRadius::ZERO,
                with_alpha(accent_color, row_alpha),
            );
        }

        // Action zone — rightmost portion of the name row
        let action_zone = if action_enabled {
            Some(Rect::from_min_max(
                egui::pos2(row_rect.max.x - ACTION_ZONE_WIDTH, row_rect.min.y),
                egui::pos2(row_rect.max.x, row_rect.min.y + name_row_h),
            ))
        } else {
            None
        };

        let in_action = action_zone.map_or(false, |az| ui.rect_contains_pointer(az));

        // Action glyph (✕) — shown on hover
        if let Some(az) = action_zone {
            if hovered && !is_dragging {
                let glyph_color = with_alpha(
                    if in_action { text_primary } else { text_dim },
                    row_alpha,
                );
                ui.painter().text(
                    az.center(),
                    egui::Align2::CENTER_CENTER,
                    "\u{2715}",
                    egui::FontId::proportional(13.0),
                    glyph_color,
                );
            }
        }

        // Tooltip for action zone
        if in_action {
            response.clone().on_hover_text("Delete context");
        }

        // Cursor — single authority
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
                ui.ctx().set_cursor_icon(if any_dragging { CursorIcon::Grabbing } else { CursorIcon::Grab });
            }
        }

        // Action priority chain
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

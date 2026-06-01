use egui::{Align, Color32, CornerRadius, CursorIcon, Id, Layout, Rect, Sense, Vec2};
use crate::app_protocol::UiNode;
use crate::theme::Colors;

pub const ACTION_ZONE_WIDTH: f32 = 30.0;

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
    /// Returned when clicking a sub-context (ctx_depth > 0) — caller pushes depth stack.
    ZoomActivate,
    Rename,
    Delete,
    DragStart,
    DragEnd,
    /// Returned when clicking an expanded pane row — caller focuses that pane.
    ActivatePane(u64),
}

pub struct PaneRowItem {
    pub pane_id: u64,
    pub label: String,
    pub is_focused: bool,
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
    /// Root path shown inline in the name row (truncated). Does not add height.
    pub subtitle: Option<String>,
    /// Total pane count shown as a dim chip in the name row.
    pub pane_count: usize,
    /// Expanded pane list rendered inside the scope when `is_expanded`.
    pub pane_rows: Vec<PaneRowItem>,
    pub is_expanded: bool,
}

impl ContextItem {
    pub fn draw(self, ui: &mut egui::Ui, id: Id, colors: &Colors) -> (SidebarAction, egui::Response) {
        let row_alpha = if self.is_dragging { 0.4_f32 } else { 1.0_f32 };

        // Reserve background shape slot before rendering content.
        let bg_idx = ui.painter().add(egui::Shape::Noop);

        let indent = 20.0 + self.ctx_depth as f32 * 12.0;

        let is_active = self.is_active;
        let is_dragging = self.is_dragging;
        let any_dragging = self.any_dragging;
        let action_enabled = self.action_enabled;
        let ctx_name = self.ctx_name.clone();
        let ctx_index = self.ctx_index;
        let badge_count = self.badge_count;
        let subtitle = self.subtitle.clone();
        let pane_count = self.pane_count;
        let pane_rows = self.pane_rows;
        let is_expanded = self.is_expanded;
        let ctx_depth = self.ctx_depth;
        let accent_color = colors.accent;
        let text_primary = colors.text_primary;
        let text_dim = colors.text_dim;
        let bg_active = colors.bg_active;
        let bg_sidebar_hover = colors.bg_sidebar_hover;

        let text_color = with_alpha(
            if is_active { text_primary } else { text_dim },
            row_alpha,
        );

        let scope_out = ui.scope(|ui| {
            ui.set_width(ui.available_width());
            let hover_pos = ui.input(|i| i.pointer.hover_pos());

            // --- Name row — single fixed-height row regardless of path or pane count ---
            let y_before = ui.cursor().min.y;
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

                // Reserve space on the right for: action zone, pane count chip, badge.
                let right_reserve = if action_enabled { ACTION_ZONE_WIDTH + 4.0 } else { 8.0 };
                let badge_w = if badge_count > 0 { 26.0 } else { 0.0 };
                let count_w = if pane_count > 0 { 18.0 } else { 0.0 };
                let text_max = (ui.available_width() - right_reserve - badge_w - count_w).max(0.0);

                // Name + subtitle share truncation zone so neither wraps and adds height.
                ui.scope(|ui| {
                    ui.set_max_width(text_max);
                    ui.add(
                        egui::Label::new(egui::RichText::new(&ctx_name).size(12.0).color(text_color))
                            .selectable(false)
                            .truncate(),
                    );
                    if let Some(ref path) = subtitle {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(format!(" — {}", shorten_path(path)))
                                    .size(10.0)
                                    .color(with_alpha(text_dim, row_alpha * 0.7)),
                            )
                            .selectable(false)
                            .truncate(),
                        );
                    }
                });

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.add_space(if action_enabled { ACTION_ZONE_WIDTH } else { 0.0 });
                    if badge_count > 0 {
                        let badge_text = if badge_count > 9 { "9+".to_string() } else { badge_count.to_string() };
                        ui.label(
                            egui::RichText::new(badge_text)
                                .size(10.0)
                                .color(with_alpha(accent_color, row_alpha)),
                        );
                    }
                    if pane_count > 0 {
                        ui.label(
                            egui::RichText::new(pane_count.to_string())
                                .size(10.0)
                                .color(with_alpha(text_dim, row_alpha * 0.5)),
                        );
                    }
                });
            });
            let name_row_h = ui.cursor().min.y - y_before;

            // --- Pane rows — rendered inside scope to keep bg_idx deferred shape aligned ---
            let mut pane_row_rects: Vec<(Rect, u64)> = Vec::new();
            if is_expanded && !pane_rows.is_empty() {
                let pane_indent = indent + 16.0;
                for row in &pane_rows {
                    let hover_bg_idx = ui.painter().add(egui::Shape::Noop);
                    let pane_label_color = if row.is_focused {
                        with_alpha(text_primary, row_alpha)
                    } else {
                        with_alpha(text_dim, row_alpha * 0.75)
                    };
                    // Pane label — rendered via component tree (E2 migration pattern).
                    // The focus indicator glyph and truncating label are each a
                    // `UiNode::Text`; both are laid out inside a horizontal stack so
                    // visual output is identical to the previous direct-egui path.
                    // TODO E3/E4: full migration of ContextItem name row pending.
                    let label_node = build_pane_label_node(
                        &row.label,
                        row.is_focused,
                        pane_label_color,
                        with_alpha(accent_color, row_alpha),
                    );
                    let row_scope = ui.scope(|ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.add_space(pane_indent);
                            crate::render_components::render_component_tree(ui, &label_node, colors);
                        });
                    });
                    let pane_rect = row_scope.response.rect;
                    if !is_dragging {
                        if hover_pos.map_or(false, |p| pane_rect.contains(p)) {
                            ui.painter().set(
                                hover_bg_idx,
                                egui::Shape::rect_filled(pane_rect, CornerRadius::ZERO, with_alpha(bg_sidebar_hover, 0.6)),
                            );
                        }
                    }
                    pane_row_rects.push((pane_rect, row.pane_id));
                }
            }

            (name_row_h, pane_row_rects)
        });

        let row_rect = scope_out.response.rect;
        let (name_row_h, pane_row_rects) = scope_out.inner;

        let response = ui.interact(row_rect, id, Sense::click_and_drag());
        let hovered = response.hovered();

        // Check pane row clicks by pointer position — avoids egui interaction conflicts.
        let pane_action: Option<SidebarAction> = response
            .interact_pointer_pos()
            .filter(|_| response.clicked())
            .and_then(|pos| {
                pane_row_rects
                    .iter()
                    .find(|(rect, _)| rect.contains(pos))
                    .map(|(_, pane_id)| SidebarAction::ActivatePane(*pane_id))
            });

        let fill = if is_active {
            with_alpha(bg_active, row_alpha)
        } else if hovered && !is_dragging {
            with_alpha(bg_sidebar_hover, row_alpha)
        } else {
            Color32::TRANSPARENT
        };

        ui.painter().set(bg_idx, egui::Shape::rect_filled(row_rect, CornerRadius::ZERO, fill));

        if is_active {
            ui.painter().rect_filled(
                Rect::from_min_size(row_rect.min, Vec2::new(3.0, row_rect.height())),
                CornerRadius::ZERO,
                with_alpha(accent_color, row_alpha),
            );
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
                ui.ctx().set_cursor_icon(if any_dragging { CursorIcon::Grabbing } else { CursorIcon::Grab });
            }
        }

        let action = if let Some(pa) = pane_action {
            pa
        } else if response.double_clicked() {
            SidebarAction::Rename
        } else if response.drag_started() {
            SidebarAction::DragStart
        } else if response.drag_stopped() {
            SidebarAction::DragEnd
        } else if response.clicked() && in_action && hovered {
            SidebarAction::Delete
        } else if response.clicked() && ctx_depth > 0 {
            SidebarAction::ZoomActivate
        } else if response.clicked() {
            SidebarAction::Activate
        } else {
            SidebarAction::None
        };

        (action, response)
    }
}

/// Build a `UiNode` for a single pane row label inside an expanded context.
///
/// Returns a horizontal `Stack` with:
/// - a focus indicator glyph (`▸` or a blank spacer) as `UiNode::Text`
/// - the pane label as `UiNode::Text`
///
/// Extracted so the node construction is testable without a live egui context.
pub(crate) fn build_pane_label_node(
    label: &str,
    is_focused: bool,
    label_color: Color32,
    accent_color: Color32,
) -> UiNode {
    let color_hex = |c: Color32| {
        format!("#{:02x}{:02x}{:02x}{:02x}", c.r(), c.g(), c.b(), c.a())
    };

    let indicator = UiNode::Text {
        text: if is_focused { "▸".to_string() } else { "   ".to_string() },
        size: 9.0,
        color: color_hex(if is_focused { accent_color } else { Color32::TRANSPARENT }),
        bold: false,
        monospace: false,
    };

    let name = UiNode::Text {
        text: label.to_string(),
        size: 11.0,
        color: color_hex(label_color),
        bold: false,
        monospace: false,
    };

    UiNode::Stack {
        direction: crate::app_protocol::StackDirection::Horizontal,
        children: vec![indicator, name],
        gap: 0.0,
        padding: crate::app_protocol::UiPadding::default(),
    }
}

#[cfg(test)]
mod sidebar_row_component_tree_tests {
    use super::*;

    /// `build_pane_label_node` returns a horizontal Stack with two Text children.
    #[test]
    fn pane_label_node_structure() {
        let label_color = Color32::from_rgb(0xaa, 0xbb, 0xcc);
        let accent_color = Color32::from_rgb(0x11, 0x22, 0x33);
        let node = build_pane_label_node("my pane", true, label_color, accent_color);
        if let UiNode::Stack { direction, children, gap, .. } = node {
            assert_eq!(direction, crate::app_protocol::StackDirection::Horizontal);
            assert_eq!(children.len(), 2);
            assert_eq!(gap, 0.0);
            // First child: focus indicator glyph
            if let UiNode::Text { text, size, .. } = &children[0] {
                assert_eq!(text, "▸");
                assert_eq!(*size, 9.0);
            } else {
                panic!("expected UiNode::Text for indicator");
            }
            // Second child: pane label
            if let UiNode::Text { text, size, .. } = &children[1] {
                assert_eq!(text, "my pane");
                assert_eq!(*size, 11.0);
            } else {
                panic!("expected UiNode::Text for label");
            }
        } else {
            panic!("expected UiNode::Stack");
        }
    }

    /// Non-focused rows use a blank spacer as the indicator.
    #[test]
    fn pane_label_node_unfocused_spacer() {
        let node = build_pane_label_node("pane2", false, Color32::WHITE, Color32::WHITE);
        if let UiNode::Stack { children, .. } = node {
            if let UiNode::Text { text, .. } = &children[0] {
                assert_ne!(text, "▸", "unfocused row must not show the focus glyph");
            } else {
                panic!("expected UiNode::Text");
            }
        } else {
            panic!("expected UiNode::Stack");
        }
    }

    /// Empty label produces a valid node without panicking.
    #[test]
    fn pane_label_node_empty_label() {
        let node = build_pane_label_node("", false, Color32::WHITE, Color32::WHITE);
        if let UiNode::Stack { children, .. } = node {
            if let UiNode::Text { text, .. } = &children[1] {
                assert_eq!(text, "");
            } else {
                panic!("expected UiNode::Text for label");
            }
        } else {
            panic!("expected UiNode::Stack");
        }
    }
}

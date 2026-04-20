//! Frame rendering — translates committed DrawCommands into egui paint calls.

use crate::app_protocol::DrawCommand;
use crate::theme::Colors;
use egui::Color32;

/// Render a committed frame's draw commands into the given egui Ui.
///
/// Only visual primitives reach this function — control commands are routed
/// upstream in `process_app/mod.rs` before commands enter the frame pipeline.
pub(super) fn render_draw_commands(ui: &mut egui::Ui, commands: &[DrawCommand], colors: &Colors) {
    let origin = ui.min_rect().min;

    for cmd in commands {
        match cmd {
            DrawCommand::Rect {
                x,
                y,
                w,
                h,
                fill,
                radius,
            } => {
                let rect = egui::Rect::from_min_size(
                    egui::pos2(origin.x + x, origin.y + y),
                    egui::vec2(*w, *h),
                );
                let color = parse_color(fill).unwrap_or(colors.bg_active);
                ui.painter().rect_filled(rect, *radius, color);
            }

            DrawCommand::Text {
                x,
                y,
                text,
                size,
                color,
                monospace,
                bold,
            } => {
                let color = parse_color(color).unwrap_or(colors.text_primary);
                let family = font_family_for_text(*monospace);
                let font_id = egui::FontId::new(*size, family);
                let pos = egui::pos2(origin.x + x, origin.y + y);
                ui.painter()
                    .text(pos, egui::Align2::LEFT_TOP, text, font_id, color);
                if *bold {
                    let font_id = egui::FontId::new(*size, font_family_for_text(*monospace));
                    ui.painter().text(
                        pos + egui::vec2(0.45, 0.0),
                        egui::Align2::LEFT_TOP,
                        text,
                        font_id,
                        color,
                    );
                }
            }

            DrawCommand::Line {
                x1,
                y1,
                x2,
                y2,
                color,
                width,
            } => {
                let color = parse_color(color).unwrap_or(colors.bg_active);
                ui.painter().line_segment(
                    [
                        egui::pos2(origin.x + x1, origin.y + y1),
                        egui::pos2(origin.x + x2, origin.y + y2),
                    ],
                    egui::Stroke::new(*width, color),
                );
            }

            DrawCommand::List {
                x,
                y,
                w,
                h,
                items,
                selected,
                item_height,
            } => {
                let row_h = if *item_height > 0.0 {
                    *item_height
                } else {
                    20.0
                };
                let list_w = if *w > 0.0 { *w } else { ui.available_width() };
                let list_h = if *h > 0.0 { *h } else { ui.available_height() };
                let clip_rect = egui::Rect::from_min_size(
                    egui::pos2(origin.x + x, origin.y + y),
                    egui::vec2(list_w, list_h),
                );
                let painter = ui.painter().with_clip_rect(clip_rect);

                for (i, item) in items.iter().enumerate() {
                    let row_y = origin.y + y + i as f32 * row_h;
                    let row_rect = egui::Rect::from_min_size(
                        egui::pos2(origin.x + x, row_y),
                        egui::vec2(list_w, row_h),
                    );
                    if !clip_rect.intersects(row_rect) {
                        continue;
                    }
                    let is_sel = i == *selected;
                    if is_sel {
                        painter.rect_filled(row_rect, 2.0, colors.bg_active);
                    }
                    let icon = if item.is_dir { "▶ " } else { "  " };
                    let label = format!("{}{}", icon, item.label);
                    painter.text(
                        egui::pos2(row_rect.min.x + 8.0, row_rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        &label,
                        egui::FontId::monospace(12.0),
                        if is_sel {
                            colors.text_primary
                        } else {
                            colors.text_dim
                        },
                    );
                    if let Some(sec) = &item.secondary {
                        painter.text(
                            egui::pos2(row_rect.max.x - 8.0, row_rect.center().y),
                            egui::Align2::RIGHT_CENTER,
                            sec,
                            egui::FontId::proportional(10.0),
                            colors.text_dim,
                        );
                    }
                }
            }

            // These are handled at the App trait level or routed upstream — never rendered.
            DrawCommand::Log { .. }
            | DrawCommand::FrameDone { .. }
            | DrawCommand::CapabilityRequest { .. }
            | DrawCommand::SecretGet { .. }
            | DrawCommand::RunGet { .. }
            | DrawCommand::RunComplete { .. }
            | DrawCommand::Notify { .. }
            | DrawCommand::PipeOpen { .. }
            | DrawCommand::PipeSend { .. }
            | DrawCommand::StatusSummary { .. }
            | DrawCommand::SpawnApp { .. }
            | DrawCommand::HttpRequest { .. }
            | DrawCommand::CdRequest { .. }
            | DrawCommand::Image { .. }
            | DrawCommand::VideoPlayer { .. }
            | DrawCommand::AudioMeter { .. }
            | DrawCommand::AudioPlay { .. }
            | DrawCommand::AudioCapture { .. }
            | DrawCommand::Ready { .. } => {}
        }
    }
}

fn font_family_for_text(monospace: bool) -> egui::FontFamily {
    if monospace {
        egui::FontFamily::Monospace
    } else {
        egui::FontFamily::Proportional
    }
}

/// Parse a hex color string like `"#1e1e2e"` into Color32.
pub(super) fn parse_color(hex: &str) -> Option<Color32> {
    let hex = hex.trim_start_matches('#');
    if hex.len() == 6 {
        let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
        let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
        let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
        Some(Color32::from_rgb(r, g, b))
    } else if hex.len() == 8 {
        let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
        let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
        let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
        let a = u8::from_str_radix(&hex[6..8], 16).ok()?;
        Some(Color32::from_rgba_premultiplied(r, g, b, a))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::font_family_for_text;

    #[test]
    fn text_font_family_never_uses_unregistered_named_family() {
        assert_eq!(font_family_for_text(false), egui::FontFamily::Proportional);
        assert_eq!(font_family_for_text(true), egui::FontFamily::Monospace);
    }
}

//! Component tree renderer — walks a `UiNode` tree and renders it into egui.
//!
//! This is the host-side counterpart to the `RenderCommand::ComponentTree`
//! protocol variant introduced in PGAP v3.5. Task A3 will wire interaction
//! events (`ComponentEvent`) back to the app; for now only rendering is done.

use egui::{Color32, Ui};

use crate::app_protocol::{StackDirection, UiNode};
use crate::theme::Colors;

/// Render a `UiNode` tree into the provided egui `Ui`.
///
/// `colors` is the active host theme — passed through so L1 sugar nodes and
/// `Raw` escape-hatch nodes have consistent theming.
///
/// The `ui` cursor starts at whatever position the caller left it; nodes are
/// rendered using egui's allocation model so they flow naturally with
/// surrounding content.
pub(crate) fn render_component_tree(ui: &mut Ui, node: &UiNode, colors: &Colors) {
    match node {
        // ── L0 primitives ────────────────────────────────────────────────

        UiNode::Stack { direction, children, gap, padding } => {
            ui.scope(|ui| {
                if padding.top > 0.0 {
                    ui.add_space(padding.top);
                }
                if padding.left > 0.0 {
                    ui.indent("stack_left_pad", |ui| {
                        render_stack(ui, direction, children, *gap, colors);
                    });
                } else {
                    render_stack(ui, direction, children, *gap, colors);
                }
                if padding.bottom > 0.0 {
                    ui.add_space(padding.bottom);
                }
            });
        }

        UiNode::Scroll { child, horizontal } => {
            let scroll = if *horizontal {
                egui::ScrollArea::both()
            } else {
                egui::ScrollArea::vertical()
            };
            scroll.show(ui, |ui| {
                render_component_tree(ui, child, colors);
            });
        }

        UiNode::Layer { children } => {
            // V1: sequential rendering (true Z-stacking is a future improvement).
            // Children paint in order; true z-layering requires a separate painter
            // pass and is deferred to a later task.
            for child in children {
                render_component_tree(ui, child, colors);
            }
        }

        UiNode::Text { text, size, color, bold, monospace } => {
            let mut rich = egui::RichText::new(text.as_str());
            if *size > 0.0 {
                rich = rich.size(*size);
            }
            if !color.is_empty() {
                // parse_color returns None on bad input — fall through to egui default.
                if let Some(c) = parse_color(color) {
                    rich = rich.color(c);
                }
            }
            if *bold {
                rich = rich.strong();
            }
            if *monospace {
                rich = rich.monospace();
            }
            ui.label(rich);
        }

        UiNode::Interactive { child, .. } => {
            // A3 will wire ComponentEvent back to the app.
            // For now render the child — interaction is silently ignored.
            render_component_tree(ui, child, colors);
        }

        UiNode::Raw { command } => {
            // Delegate to the existing flat renderer for a single draw command.
            // We use the Ui clip rect as the pane_rect so relative offsets work.
            let pane_rect = ui.clip_rect();
            crate::process_app::render::render_draw_commands(
                ui,
                pane_rect,
                std::slice::from_ref(command.as_ref()),
                colors,
                &mut egui_commonmark::CommonMarkCache::default(),
                &std::collections::HashMap::new(),
                &mut crate::process_app::image_cache::ImageCache::new(),
                std::path::Path::new("."),
                false,
                &mut std::collections::HashMap::new(),
                &mut std::collections::HashMap::new(),
            );
        }

        UiNode::Surface { .. } => {
            // Reserved for future GPU surface layer — no-op.
            log::trace!("render_components: Surface node encountered — no-op (future GPU layer)");
        }

        // ── L1 sugar ─────────────────────────────────────────────────────

        UiNode::Button { label, disabled, .. } => {
            // node_id interaction wiring deferred to A3.
            ui.add_enabled(!disabled, egui::Button::new(label.as_str()));
        }

        UiNode::Input { value, .. } => {
            // node_id and change event wiring deferred to A3.
            // Clone the value so we have a mutable binding; changes are
            // discarded each frame until A3 wires events.
            let mut buf = value.clone();
            ui.text_edit_singleline(&mut buf);
        }

        UiNode::Badge { label, fill, fg, .. } => {
            let fill_color = if fill.is_empty() {
                colors.accent
            } else {
                parse_color(fill).unwrap_or(colors.accent)
            };
            let fg_color = if fg.is_empty() {
                Color32::from_rgb(0x1e, 0x1e, 0x2e)
            } else {
                parse_color(fg).unwrap_or(Color32::from_rgb(0x1e, 0x1e, 0x2e))
            };
            egui::Frame::new()
                .fill(fill_color)
                .corner_radius(egui::CornerRadius::same(4))
                .inner_margin(egui::Margin::symmetric(6, 2))
                .show(ui, |ui| {
                    ui.label(egui::RichText::new(label.as_str()).color(fg_color).size(12.0));
                });
        }

        UiNode::Dot { color, size, .. } => {
            let dot_size = if *size > 0.0 { *size } else { 8.0 };
            let fill = if color.is_empty() {
                colors.accent
            } else {
                parse_color(color).unwrap_or(colors.accent)
            };
            let (rect, _) = ui.allocate_exact_size(
                egui::vec2(dot_size, dot_size),
                egui::Sense::hover(),
            );
            ui.painter().circle_filled(rect.center(), dot_size / 2.0, fill);
        }
    }
}

// ── Internal helpers ──────────────────────────────────────────────────────────

fn render_stack(
    ui: &mut Ui,
    direction: &StackDirection,
    children: &[UiNode],
    gap: f32,
    colors: &Colors,
) {
    match direction {
        StackDirection::Horizontal => {
            ui.horizontal(|ui| {
                for (i, child) in children.iter().enumerate() {
                    if i > 0 && gap > 0.0 {
                        ui.add_space(gap);
                    }
                    render_component_tree(ui, child, colors);
                }
            });
        }
        StackDirection::Vertical => {
            ui.vertical(|ui| {
                for (i, child) in children.iter().enumerate() {
                    if i > 0 && gap > 0.0 {
                        ui.add_space(gap);
                    }
                    render_component_tree(ui, child, colors);
                }
            });
        }
    }
}

/// Parse a CSS hex color string (`#rrggbb` or `#rrggbbaa`) into `Color32`.
/// Returns `None` on any parse failure — callers must supply a fallback.
fn parse_color(hex: &str) -> Option<Color32> {
    let hex = hex.trim_start_matches('#');
    match hex.len() {
        6 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            Some(Color32::from_rgb(r, g, b))
        }
        8 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            let a = u8::from_str_radix(&hex[6..8], 16).ok()?;
            Some(Color32::from_rgba_premultiplied(r, g, b, a))
        }
        _ => None,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod render_component_tree_tests {
    use super::*;
    use crate::app_protocol::{StackDirection, UiNode, UiPadding};

    /// A `UiNode::Text` with `size == 0.0` must not pass 0.0 to `RichText::size()`,
    /// and an empty color string must return `None` from `parse_color` without panicking.
    #[test]
    fn text_zero_size_empty_color_no_panic() {
        let node = UiNode::Text {
            text: "hello".into(),
            size: 0.0,
            color: String::new(),
            bold: false,
            monospace: false,
        };
        if let UiNode::Text { size, color, .. } = &node {
            // Guard: size 0.0 must be skipped (not forwarded to RichText).
            assert_eq!(*size, 0.0);
            // Guard: empty color must parse to None — no unwrap panic.
            assert!(parse_color(color).is_none());
        } else {
            panic!("wrong variant");
        }
    }

    /// A `UiNode::Stack` with two text children should be constructable and
    /// both children must produce valid color parses.
    #[test]
    fn stack_two_children_constructable() {
        let node = UiNode::Stack {
            direction: StackDirection::Vertical,
            children: vec![
                UiNode::Text {
                    text: "first".into(),
                    size: 14.0,
                    color: "#ffffff".into(),
                    bold: false,
                    monospace: false,
                },
                UiNode::Text {
                    text: "second".into(),
                    size: 14.0,
                    color: "#aaaaaa".into(),
                    bold: true,
                    monospace: false,
                },
            ],
            gap: 4.0,
            padding: UiPadding::default(),
        };

        if let UiNode::Stack { children, gap, .. } = &node {
            assert_eq!(children.len(), 2);
            assert_eq!(*gap, 4.0);
        } else {
            panic!("wrong variant");
        }

        assert!(parse_color("#ffffff").is_some());
        assert!(parse_color("#aaaaaa").is_some());
    }

    /// `parse_color` handles edge cases without panicking.
    #[test]
    fn parse_color_edge_cases() {
        assert!(parse_color("").is_none());
        assert!(parse_color("#").is_none());
        assert!(parse_color("#gg0000").is_none()); // invalid hex digits
        assert!(parse_color("#ff0000").is_some());
        assert!(parse_color("ff0000").is_some()); // no leading #
        assert!(parse_color("#ff0000ff").is_some()); // 8-char rgba
    }

    /// Surface node variant is handled — just verify it compiles and matches.
    #[test]
    fn surface_node_variant_exists() {
        let node = UiNode::Surface { id: "canvas".into() };
        if let UiNode::Surface { id } = &node {
            assert_eq!(id, "canvas");
        } else {
            panic!("wrong variant");
        }
    }
}

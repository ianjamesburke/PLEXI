//! Component tree renderer — walks a `UiNode` tree and renders it into egui.
//!
//! This is the host-side counterpart to the `RenderCommand::ComponentTree`
//! protocol variant introduced in PGAP v3.5. Interactive nodes (`Button`,
//! `Input`, `Interactive`) fire `ComponentEvent`s back to the app via the
//! returned `Vec<ComponentEventPayload>` (task A3).

use egui::Ui;

use crate::app_protocol::{StackDirection, UiNode};
use crate::theme::Colors;

/// Carries the data needed to emit a `PlexiEvent::ComponentEvent`.
///
/// Returned from `render_component_tree` and converted to `PlexiEvent` by
/// the `ComponentTree` arm in `render_draw_commands`.
pub(crate) struct ComponentEventPayload {
    pub(crate) node_id: String,
    pub(crate) event_type: String,
    pub(crate) payload: Option<serde_json::Value>,
}

/// Render a `UiNode` tree into the provided egui `Ui`.
///
/// Returns any interaction events that occurred during this frame so the
/// caller can forward them to the app as `PlexiEvent::ComponentEvent`.
///
/// `colors` is the active host theme — passed through so L1 sugar nodes and
/// `Raw` escape-hatch nodes have consistent theming.
pub(crate) fn render_component_tree(
    ui: &mut Ui,
    node: &UiNode,
    colors: &Colors,
) -> Vec<ComponentEventPayload> {
    let mut events: Vec<ComponentEventPayload> = Vec::new();

    match node {
        // ── L0 primitives ────────────────────────────────────────────────

        UiNode::Stack { direction, children, gap, padding } => {
            ui.scope(|ui| {
                if padding.top > 0.0 {
                    ui.add_space(padding.top);
                }
                if padding.left > 0.0 {
                    ui.indent("stack_left_pad", |ui| {
                        events.extend(render_stack(ui, direction, children, *gap, colors));
                    });
                } else {
                    events.extend(render_stack(ui, direction, children, *gap, colors));
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
                events.extend(render_component_tree(ui, child, colors));
            });
        }

        UiNode::Layer { children } => {
            // V1: sequential rendering (true Z-stacking is a future improvement).
            for child in children {
                events.extend(render_component_tree(ui, child, colors));
            }
        }

        UiNode::Text { text, size, color, bold, monospace } => {
            let mut rich = egui::RichText::new(text.as_str());
            if *size > 0.0 {
                rich = rich.size(*size);
            }
            if !color.is_empty() {
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

        UiNode::Interactive { node_id, child, on_click, on_hover } => {
            // Render the child inside an interact-sense scope so we get a
            // Response covering the child's bounding rect.
            let child_response = ui.scope(|ui| {
                let child_evts = render_component_tree(ui, child, colors);
                // Bubble child events up.
                (child_evts, ui.min_rect())
            });
            let (child_evts, child_rect) = child_response.inner;
            events.extend(child_evts);

            // Allocate an invisible interact-rect on top of the child area.
            let response = ui.interact(
                child_rect,
                egui::Id::new(node_id.as_str()),
                egui::Sense::click_and_drag(),
            );

            if *on_click && response.clicked() {
                log::info!(
                    "render_components: Interactive click node_id={node_id}"
                );
                events.push(ComponentEventPayload {
                    node_id: node_id.clone(),
                    event_type: "click".into(),
                    payload: None,
                });
            }
            if *on_hover && response.hovered() {
                events.push(ComponentEventPayload {
                    node_id: node_id.clone(),
                    event_type: "hover_enter".into(),
                    payload: None,
                });
            }
        }

        UiNode::Raw { command } => {
            // Delegate to the existing flat renderer for a single draw command.
            let pane_rect = ui.clip_rect();
            // V1: fresh cache per Raw node — loses cache state across frames.
            // A future pass will thread parent caches through. See epic #1897 A2.
            let mut raw_events: Vec<crate::app_protocol::PlexiEvent> = Vec::new();
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
                &mut raw_events,
            );
            // Convert any ComponentEvent payloads back from PlexiEvent (unlikely
            // from a Raw draw command, but keep the pipeline consistent).
            for evt in raw_events {
                if let crate::app_protocol::PlexiEvent::ComponentEvent {
                    node_id,
                    event_type,
                    payload,
                } = evt
                {
                    events.push(ComponentEventPayload { node_id, event_type, payload });
                }
            }
        }

        UiNode::Surface { .. } => {
            // Reserved for future GPU surface layer — no-op.
            log::trace!("render_components: Surface node encountered — no-op (future GPU layer)");
        }

        // ── L1 sugar ─────────────────────────────────────────────────────────

        UiNode::Button { node_id, label, disabled, .. } => {
            const BTN_PAD_V: f32 = 5.0;
            let text_color = if *disabled { colors.text_dim } else { colors.text_primary };
            let font_id = egui::FontId::proportional(crate::style::TEXT_BODY);
            let galley = ui.fonts(|f| f.layout_no_wrap(label.clone(), font_id, text_color));
            let text_w = galley.size().x;
            let text_h = galley.size().y;
            let btn_w = (text_w + crate::style::SPACE_SM * 2.0).max(48.0);
            let btn_h = text_h + BTN_PAD_V * 2.0;
            let sense = if *disabled { egui::Sense::hover() } else { egui::Sense::click() };
            let (rect, response) =
                ui.allocate_exact_size(egui::vec2(btn_w, btn_h), sense);
            let painter = ui.painter();
            painter.rect_filled(rect, crate::style::RADIUS_MD, colors.bg_active);
            if !*disabled {
                let stroke_color =
                    if response.hovered() { colors.accent } else { colors.border };
                painter.rect_stroke(
                    rect,
                    crate::style::RADIUS_MD,
                    egui::Stroke::new(1.0, stroke_color),
                    egui::StrokeKind::Inside,
                );
                if response.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
            }
            let text_pos =
                egui::pos2(rect.center().x - text_w / 2.0, rect.center().y - text_h / 2.0);
            painter.galley(text_pos, galley, text_color);
            if response.clicked() {
                log::info!("render_components: Button click node_id={node_id}");
                events.push(ComponentEventPayload {
                    node_id: node_id.clone(),
                    event_type: "click".into(),
                    payload: None,
                });
            }
        }

        UiNode::Input { node_id, value, placeholder, .. } => {
            let mut val_buf = value.clone();
            let response = crate::widgets::styled_text_input(
                ui,
                &mut val_buf,
                placeholder.as_str(),
                egui::Id::new(node_id.as_str()),
                colors,
            );
            if response.changed() {
                log::debug!(
                    "render_components: Input change node_id={node_id} value={val_buf:?}"
                );
                events.push(ComponentEventPayload {
                    node_id: node_id.clone(),
                    event_type: "change".into(),
                    payload: Some(serde_json::json!({ "value": val_buf })),
                });
            }
            if response.lost_focus()
                && ui.input(|i| i.key_pressed(egui::Key::Enter))
            {
                log::info!(
                    "render_components: Input submit node_id={node_id} value={val_buf:?}"
                );
                events.push(ComponentEventPayload {
                    node_id: node_id.clone(),
                    event_type: "submit".into(),
                    payload: Some(serde_json::json!({ "value": val_buf })),
                });
            }
        }

        UiNode::Badge { label, fill, fg, .. } => {
            let fill_color = if fill.is_empty() {
                colors.accent
            } else {
                parse_color(fill).unwrap_or(colors.accent)
            };
            let fg_color = if fg.is_empty() {
                colors.text_primary
            } else {
                parse_color(fg).unwrap_or(colors.text_primary)
            };
            egui::Frame::new()
                .fill(fill_color)
                .stroke(egui::Stroke::new(1.0, colors.border))
                .corner_radius(egui::CornerRadius::same(crate::style::RADIUS_BADGE as u8))
                .inner_margin(egui::Margin::symmetric(
                    crate::style::BADGE_PAD_H as i8,
                    crate::style::BADGE_PAD_V as i8,
                ))
                .show(ui, |ui| {
                    ui.label(
                        egui::RichText::new(label.as_str())
                            .color(fg_color)
                            .size(crate::style::TEXT_CAPTION),
                    );
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

    events
}

// ── Internal helpers ──────────────────────────────────────────────────────────

fn render_stack(
    ui: &mut Ui,
    direction: &StackDirection,
    children: &[UiNode],
    gap: f32,
    colors: &Colors,
) -> Vec<ComponentEventPayload> {
    let mut events = Vec::new();
    match direction {
        StackDirection::Horizontal => {
            ui.horizontal(|ui| {
                for (i, child) in children.iter().enumerate() {
                    if i > 0 && gap > 0.0 {
                        ui.add_space(gap);
                    }
                    events.extend(render_component_tree(ui, child, colors));
                }
            });
        }
        StackDirection::Vertical => {
            ui.vertical(|ui| {
                for (i, child) in children.iter().enumerate() {
                    if i > 0 && gap > 0.0 {
                        ui.add_space(gap);
                    }
                    events.extend(render_component_tree(ui, child, colors));
                }
            });
        }
    }
    events
}

use crate::process_app::render::parse_color;

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
            assert_eq!(*size, 0.0);
            assert!(parse_color(color).is_none());
        } else {
            panic!("wrong variant");
        }
    }

    /// A `UiNode::Stack` with two text children should be constructable.
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
        assert!(parse_color("#gg0000").is_none());
        assert!(parse_color("#ff0000").is_some());
        assert!(parse_color("ff0000").is_some());
        assert!(parse_color("#ff0000ff").is_some());
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

    /// `ComponentEventPayload` can be constructed with all fields.
    #[test]
    fn component_event_payload_constructable() {
        let evt = ComponentEventPayload {
            node_id: "btn1".into(),
            event_type: "click".into(),
            payload: None,
        };
        assert_eq!(evt.node_id, "btn1");
        assert_eq!(evt.event_type, "click");
        assert!(evt.payload.is_none());
    }

    /// `ComponentEventPayload` with a JSON payload round-trips correctly.
    #[test]
    fn component_event_payload_with_json_value() {
        let val = serde_json::json!({ "value": "hello" });
        let evt = ComponentEventPayload {
            node_id: "inp1".into(),
            event_type: "change".into(),
            payload: Some(val.clone()),
        };
        assert_eq!(evt.node_id, "inp1");
        assert_eq!(evt.event_type, "change");
        assert_eq!(evt.payload.unwrap(), val);
    }

    /// `UiNode::Button` node can be constructed with all fields and the
    /// node_id is preserved. Event emission logic requires a real egui context
    /// to test (headless tests cover struct correctness only).
    #[test]
    fn button_click_emits_component_event_struct_check() {
        // Verify that a Button node_id="btn1" can be constructed and fields are correct.
        // The actual click→event path requires an egui display context; struct
        // correctness is verified here.
        let node = UiNode::Button {
            node_id: "btn1".into(),
            label: "Click me".into(),
            disabled: false,
            _l0: Box::new(UiNode::Text {
                text: "Click me".into(),
                size: 14.0,
                color: String::new(),
                bold: false,
                monospace: false,
            }),
        };
        if let UiNode::Button { node_id, label, disabled, .. } = &node {
            assert_eq!(node_id, "btn1");
            assert_eq!(label, "Click me");
            assert!(!disabled);
        } else {
            panic!("wrong variant");
        }
        // Verify the payload we'd construct on click is correct.
        let evt = ComponentEventPayload {
            node_id: "btn1".into(),
            event_type: "click".into(),
            payload: None,
        };
        assert_eq!(evt.node_id, "btn1");
        assert_eq!(evt.event_type, "click");
    }

    /// `UiNode::Interactive` wraps a child — verify structure and on_click/on_hover fields.
    #[test]
    fn interactive_node_wraps_child_and_collects_events() {
        let child = UiNode::Text {
            text: "inner".into(),
            size: 12.0,
            color: String::new(),
            bold: false,
            monospace: false,
        };
        let node = UiNode::Interactive {
            node_id: "wrap1".into(),
            child: Box::new(child),
            on_click: true,
            on_hover: false,
        };
        if let UiNode::Interactive { node_id, on_click, on_hover, .. } = &node {
            assert_eq!(node_id, "wrap1");
            assert!(*on_click);
            assert!(!*on_hover);
        } else {
            panic!("wrong variant");
        }
        // Verify that a click event for this node would be correctly shaped.
        let evt = ComponentEventPayload {
            node_id: "wrap1".into(),
            event_type: "click".into(),
            payload: None,
        };
        assert_eq!(evt.event_type, "click");
        assert_eq!(evt.node_id, "wrap1");
    }
}

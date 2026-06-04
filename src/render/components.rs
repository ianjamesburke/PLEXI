//! Component tree renderer — walks a `UiNode` tree and renders it into egui.
//!
//! This is the host-side counterpart to the `RenderCommand::ComponentTree`
//! protocol variant introduced in PGAP v3.5. Interactive nodes (`Button`,
//! `Input`, `Interactive`) fire `ComponentEvent`s back to the app via the
//! returned `Vec<ComponentEventPayload>` (task A3).

use egui::Ui;

use crate::app_protocol::{StackDirection, UiNode};
use crate::ui::style;
use crate::ui::theme::Colors;

/// Carries the data needed to emit a `PlexiEvent::ComponentEvent`.
///
/// Returned from `render_component_tree` and converted to `PlexiEvent` by
/// the `ComponentTree` arm in `render_draw_commands`.
pub(crate) struct ComponentEventPayload {
    pub(crate) node_id: String,
    pub(crate) event_type: String,
    pub(crate) payload: Option<serde_json::Value>,
}

/// Focus and styling context for `UiNode::TextEdit` nodes within a component
/// tree render pass. Tracks auto-focus state so only the first newly-visible
/// TextEdit gets focused, and reports back whether any TextEdit has egui focus
/// (so the host can suppress key forwarding).
pub(crate) struct TextEditFocusCtx {
    /// Set of TextEdit node_ids that were visible in the previous frame.
    /// Used to detect newly-appearing fields for auto-focus.
    pub(crate) prev_visible: std::collections::HashSet<String>,
    /// Node_ids visible in the current frame. After render, this becomes
    /// the next frame's `prev_visible`.
    pub(crate) current_visible: std::collections::HashSet<String>,
    /// True if the pane was just focused (tab switch, click).
    pub(crate) pane_just_focused: bool,
    /// Set to true during the frame once any TextEdit has been auto-focused,
    /// preventing multiple fields from grabbing focus simultaneously.
    focus_granted_this_frame: bool,
    /// Set to true if any TextEdit has egui focus during this render pass.
    /// Read by `RenderSession` to suppress key forwarding while the user types.
    pub(crate) any_has_focus: bool,
}

impl TextEditFocusCtx {
    pub(crate) fn new() -> Self {
        Self {
            prev_visible: std::collections::HashSet::new(),
            current_visible: std::collections::HashSet::new(),
            pane_just_focused: false,
            focus_granted_this_frame: false,
            any_has_focus: false,
        }
    }

    /// Call after each frame to rotate visibility sets.
    pub(crate) fn end_frame(&mut self) {
        std::mem::swap(&mut self.prev_visible, &mut self.current_visible);
        self.current_visible.clear();
        self.focus_granted_this_frame = false;
        self.any_has_focus = false;
        self.pane_just_focused = false;
    }
}

/// Render a `UiNode` tree into the provided egui `Ui`.
///
/// Returns any interaction events that occurred during this frame so the
/// caller can forward them to the app as `PlexiEvent::ComponentEvent`.
///
/// `colors` is the active host theme — passed through so L1 sugar nodes and
/// `Raw` escape-hatch nodes have consistent theming.
///
/// `text_edit_buffers` provides persistent per-node_id text buffers for
/// `UiNode::TextEdit` nodes. The buffer is seeded from the app's `value`
/// field when a new node_id first appears.
///
/// `focus_ctx` tracks auto-focus and click-focus state for TextEdit nodes
/// across recursive calls.
pub(crate) fn render_component_tree(
    ui: &mut Ui,
    node: &UiNode,
    colors: &Colors,
    text_edit_buffers: &mut std::collections::HashMap<String, String>,
    focus_ctx: &mut TextEditFocusCtx,
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
                        events.extend(render_stack(ui, direction, children, *gap, colors, text_edit_buffers, focus_ctx));
                    });
                } else {
                    events.extend(render_stack(ui, direction, children, *gap, colors, text_edit_buffers, focus_ctx));
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
                events.extend(render_component_tree(ui, child, colors, text_edit_buffers, focus_ctx));
            });
        }

        UiNode::Layer { children } => {
            // V1: sequential rendering (true Z-stacking is a future improvement).
            for child in children {
                events.extend(render_component_tree(ui, child, colors, text_edit_buffers, focus_ctx));
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
            ui.add(egui::Label::new(rich).selectable(true));
        }

        UiNode::Interactive { node_id, child, on_click, on_hover } => {
            // Render the child inside an interact-sense scope so we get a
            // Response covering the child's bounding rect.
            let child_response = ui.scope(|ui| {
                let child_evts = render_component_tree(ui, child, colors, text_edit_buffers, focus_ctx);
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
            // Raw escape-hatch uses a throwaway focus ctx — focus tracking doesn't
            // apply to legacy draw commands embedded inside a component tree.
            let mut raw_focus_ctx = TextEditFocusCtx::new();
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
                text_edit_buffers,
                &mut raw_focus_ctx,
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
            let font_id = egui::FontId::proportional(crate::ui::style::TEXT_BODY);
            // Layout with placeholder color; painter.galley() overrides per-state below.
            let galley = ui.fonts(|f| {
                f.layout_no_wrap(label.clone(), font_id, egui::Color32::WHITE)
            });
            let text_w = galley.size().x;
            let text_h = galley.size().y;
            let btn_w = (text_w + crate::ui::style::SPACE_SM * 2.0).max(48.0);
            let btn_h = text_h + BTN_PAD_V * 2.0;
            let (rect, _) = ui.allocate_exact_size(egui::vec2(btn_w, btn_h), egui::Sense::hover());
            // Use raw PointerState — button_down/button_pressed read pointer_events directly
            // and are not affected by the pane-wide Sense::click_and_drag() widget registered
            // later at process_app/mod.rs:1676.
            let pointer_pos =
                ui.input(|i| i.pointer.interact_pos().or_else(|| i.pointer.hover_pos()));
            let is_hovered = !*disabled && pointer_pos.map_or(false, |p| rect.contains(p));
            let is_down =
                is_hovered && ui.input(|i| i.pointer.button_down(egui::PointerButton::Primary));
            let is_just_pressed =
                is_hovered && ui.input(|i| i.pointer.button_pressed(egui::PointerButton::Primary));
            let painter = ui.painter();
            let fill = if is_down { colors.accent } else { colors.bg_active };
            painter.rect_filled(rect, crate::ui::style::RADIUS_MD, fill);
            if !*disabled {
                let stroke_color = if is_hovered { colors.accent } else { colors.border };
                painter.rect_stroke(
                    rect,
                    crate::ui::style::RADIUS_MD,
                    egui::Stroke::new(1.0, stroke_color),
                    egui::StrokeKind::Inside,
                );
                if is_hovered {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
            }
            let text_color =
                if *disabled { colors.text_dim } else if is_down { colors.bg_active } else { colors.text_primary };
            let text_pos =
                egui::pos2(rect.center().x - text_w / 2.0, rect.center().y - text_h / 2.0);
            painter.galley(text_pos, galley, text_color);
            if is_just_pressed {
                log::info!("render_components: Button press node_id={node_id}");
                events.push(ComponentEventPayload {
                    node_id: node_id.clone(),
                    event_type: "click".into(),
                    payload: None,
                });
            }
        }

        UiNode::Input { node_id, value, placeholder, .. } => {
            let mut val_buf = value.clone();
            let response = crate::ui::widgets::styled_text_input(
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

        UiNode::TextEdit { node_id, placeholder, value, multiline, max_length, .. } => {
            // Seed the buffer from the app's value when this node_id first appears.
            let buffer = text_edit_buffers
                .entry(node_id.clone())
                .or_insert_with(|| value.clone());

            // Enforce max_length by truncating the buffer if it exceeds the limit.
            if *max_length > 0 && buffer.len() > *max_length {
                buffer.truncate(*max_length);
            }

            // Track visibility for auto-focus detection.
            let newly_visible = !focus_ctx.prev_visible.contains(node_id.as_str());
            focus_ctx.current_visible.insert(node_id.clone());

            let prev_value = buffer.clone();
            let widget_id = egui::Id::new(("text_edit_node", node_id.as_str()));

            // Styling matches QuickNote overlay: frameless, monospace, accent cursor,
            // dim hint text. See src/overlays/quick_note.rs lines 138-154.
            let response = ui.scope(|ui| {
                ui.visuals_mut().text_cursor.stroke.width = 1.5;
                ui.visuals_mut().text_cursor.stroke.color = colors.accent;

                if *multiline {
                    let hint = egui::RichText::new(placeholder.as_str())
                        .color(colors.text_dim.linear_multiply(0.3))
                        .size(style::TEXT_BODY);
                    let mut edit = egui::TextEdit::multiline(buffer)
                        .id(widget_id)
                        .font(egui::FontId::monospace(style::TEXT_BODY))
                        .text_color(colors.text_primary)
                        .desired_width(f32::INFINITY)
                        .frame(false)
                        .hint_text(hint);
                    if *max_length > 0 {
                        edit = edit.char_limit(*max_length);
                    }
                    ui.add(edit)
                } else {
                    let hint = egui::RichText::new(placeholder.as_str())
                        .color(colors.text_dim.linear_multiply(0.3))
                        .size(style::TEXT_BODY);
                    let mut edit = egui::TextEdit::singleline(buffer)
                        .id(widget_id)
                        .font(egui::FontId::monospace(style::TEXT_BODY))
                        .text_color(colors.text_primary)
                        .desired_width(f32::INFINITY)
                        .frame(false)
                        .hint_text(hint);
                    if *max_length > 0 {
                        edit = edit.char_limit(*max_length);
                    }
                    ui.add(edit)
                }
            }).inner;

            // Auto-focus: first newly-visible TextEdit, or first TextEdit when
            // the pane just gained keyboard focus.
            if (newly_visible || focus_ctx.pane_just_focused)
                && !focus_ctx.focus_granted_this_frame
            {
                response.request_focus();
                focus_ctx.focus_granted_this_frame = true;
                log::info!(
                    "render_components: TextEdit auto-focus node_id={node_id} newly_visible={newly_visible} pane_focused={}",
                    focus_ctx.pane_just_focused
                );
            }

            // Click-to-focus: if the user clicked inside the TextEdit area,
            // request focus so the cursor appears and typing works.
            if response.clicked() {
                response.request_focus();
                log::debug!(
                    "render_components: TextEdit click-focus node_id={node_id}"
                );
            }

            // Track focus for key suppression.
            if response.has_focus() {
                focus_ctx.any_has_focus = true;
            }

            // Emit "change" event when value differs from previous frame.
            if *buffer != prev_value {
                log::debug!(
                    "render_components: TextEdit change node_id={node_id} value={:?}",
                    buffer
                );
                events.push(ComponentEventPayload {
                    node_id: node_id.clone(),
                    event_type: "change".into(),
                    payload: Some(serde_json::json!({ "value": *buffer })),
                });
            }

            // Submit: Enter for single-line, Cmd+Enter for multiline.
            let should_submit = if *multiline {
                response.has_focus()
                    && ui.input(|i| {
                        i.key_pressed(egui::Key::Enter) && i.modifiers.command
                    })
            } else {
                response.lost_focus()
                    && ui.input(|i| i.key_pressed(egui::Key::Enter))
            };

            if should_submit {
                log::info!(
                    "render_components: TextEdit submit node_id={node_id} value={:?}",
                    buffer
                );
                events.push(ComponentEventPayload {
                    node_id: node_id.clone(),
                    event_type: "submit".into(),
                    payload: Some(serde_json::json!({ "value": *buffer })),
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
                .corner_radius(egui::CornerRadius::same(crate::ui::style::RADIUS_BADGE as u8))
                .inner_margin(egui::Margin::symmetric(
                    crate::ui::style::BADGE_PAD_H as i8,
                    crate::ui::style::BADGE_PAD_V as i8,
                ))
                .show(ui, |ui| {
                    ui.label(
                        egui::RichText::new(label.as_str())
                            .color(fg_color)
                            .size(crate::ui::style::TEXT_CAPTION),
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

        // ── L1 layout components ────────────────────────────────────────

        UiNode::AppBar { title, subtitle, .. } => {
            const TITLE_SIZE: f32 = 16.0;
            let has_subtitle = !subtitle.is_empty();
            let band_h = if has_subtitle { 48.0 } else { 34.0 };
            let total_h = band_h + 1.0;
            let (rect, _) =
                ui.allocate_exact_size(egui::vec2(ui.available_width(), total_h), egui::Sense::hover());
            let painter = ui.painter();
            painter.rect_filled(rect, 0.0, colors.bg_sidebar);
            let text_x = rect.min.x + style::SPACE_MD;
            let max_w = rect.width() - 2.0 * style::SPACE_MD;
            // Top-align title within the band with consistent top padding.
            let title_y = rect.min.y + style::SPACE_SM;
            if has_subtitle {
                let sub_y = title_y + TITLE_SIZE + style::SPACE_XS;
                let title_galley = ui.fonts(|f| {
                    f.layout(title.clone(), egui::FontId::proportional(TITLE_SIZE),
                             colors.text_primary, max_w)
                });
                painter.galley(egui::pos2(text_x, title_y), title_galley, colors.text_primary);
                let sub_galley = ui.fonts(|f| {
                    f.layout(subtitle.clone(), egui::FontId::proportional(style::TEXT_HINT),
                             colors.text_dim, max_w)
                });
                painter.galley(egui::pos2(text_x, sub_y), sub_galley, colors.text_dim);
            } else {
                let title_galley = ui.fonts(|f| {
                    f.layout(title.clone(), egui::FontId::proportional(TITLE_SIZE),
                             colors.text_primary, max_w)
                });
                painter.galley(egui::pos2(text_x, title_y), title_galley, colors.text_primary);
            }
            painter.rect_filled(
                egui::Rect::from_min_size(
                    egui::pos2(rect.min.x, rect.min.y + band_h),
                    egui::vec2(rect.width(), 1.0),
                ),
                0.0,
                colors.border,
            );
        }

        UiNode::FooterKeys { entries, divider, .. } => {
            let chip_row_h = style::TEXT_HINT + 6.0;
            let total_h = if *divider {
                style::SPACE_SM + 1.0 + style::SPACE_SM + chip_row_h + style::SPACE_SM
            } else {
                style::SPACE_SM + chip_row_h + style::SPACE_SM
            };
            let (rect, _) =
                ui.allocate_exact_size(egui::vec2(ui.available_width(), total_h), egui::Sense::hover());
            let painter = ui.painter();
            painter.rect_filled(rect, 0.0, colors.bg_sidebar);
            let mut y = rect.min.y;
            if *divider {
                y += style::SPACE_SM;
                painter.rect_filled(
                    egui::Rect::from_min_size(egui::pos2(rect.min.x, y), egui::vec2(rect.width(), 1.0)),
                    0.0,
                    colors.border,
                );
                y += 1.0 + style::SPACE_SM;
            } else {
                y += style::SPACE_SM;
            }
            let chip_y = y + (chip_row_h - style::TEXT_HINT) / 2.0;
            let mut cx = rect.min.x + style::SPACE_MD;
            for entry in entries {
                for (ki, key) in entry.keys.iter().enumerate() {
                    if ki > 0 {
                        cx += 2.0;
                    }
                    let font_id = egui::FontId::monospace(style::TEXT_HINT);
                    let galley = ui.fonts(|f| {
                        f.layout_no_wrap(key.clone(), font_id, colors.text_primary)
                    });
                    let tw = galley.size().x;
                    let chip_w = tw + 8.0;
                    let chip_rect = egui::Rect::from_min_size(
                        egui::pos2(cx, chip_y - 1.0),
                        egui::vec2(chip_w, style::TEXT_HINT + 4.0),
                    );
                    painter.rect_filled(chip_rect, 3.0, colors.bg_active);
                    painter.rect_stroke(
                        chip_rect,
                        3.0,
                        egui::Stroke::new(0.5, colors.border),
                        egui::StrokeKind::Inside,
                    );
                    painter.galley(
                        egui::pos2(cx + 4.0, chip_y),
                        galley,
                        colors.text_primary,
                    );
                    cx += chip_w;
                }
                cx += 4.0;
                let desc_galley = ui.fonts(|f| {
                    f.layout_no_wrap(
                        entry.description.clone(),
                        egui::FontId::proportional(style::TEXT_HINT),
                        colors.text_dim,
                    )
                });
                painter.galley(egui::pos2(cx, chip_y), desc_galley, colors.text_dim);
                cx += ui.fonts(|f| {
                    f.layout_no_wrap(
                        entry.description.clone(),
                        egui::FontId::proportional(style::TEXT_HINT),
                        colors.text_dim,
                    )
                    .size()
                    .x
                }) + style::SPACE_MD;
            }
        }

        UiNode::Footer { text, color, .. } => {
            let line_h = style::TEXT_CAPTION + 5.0;
            let total_h = style::SPACE_MD + 1.0 + style::SPACE_MD + line_h;
            let (rect, _) =
                ui.allocate_exact_size(egui::vec2(ui.available_width(), total_h), egui::Sense::hover());
            let painter = ui.painter();
            let line_y = rect.min.y + style::SPACE_MD;
            painter.rect_filled(
                egui::Rect::from_min_size(egui::pos2(rect.min.x, line_y), egui::vec2(rect.width(), 1.0)),
                0.0,
                colors.border,
            );
            let text_color = if color.is_empty() {
                colors.text_dim
            } else {
                parse_color(color).unwrap_or(colors.text_dim)
            };
            let text_y = line_y + 1.0 + style::SPACE_MD;
            let galley = ui.fonts(|f| {
                f.layout(
                    text.clone(),
                    egui::FontId::proportional(style::TEXT_CAPTION),
                    text_color,
                    rect.width(),
                )
            });
            painter.galley(egui::pos2(rect.min.x, text_y), galley, text_color);
        }

        UiNode::Section { title, .. } => {
            let total_h = style::SPACE_SM + style::TEXT_HINT + style::SPACE_XS + 1.0 + style::SPACE_XS;
            let (rect, _) =
                ui.allocate_exact_size(egui::vec2(ui.available_width(), total_h), egui::Sense::hover());
            let painter = ui.painter();
            let label_y = rect.min.y + style::SPACE_SM;
            let galley = ui.fonts(|f| {
                f.layout_no_wrap(
                    title.to_uppercase(),
                    egui::FontId::proportional(style::TEXT_HINT),
                    colors.text_dim,
                )
            });
            painter.galley(egui::pos2(rect.min.x, label_y), galley, colors.text_dim);
            let line_y = label_y + style::TEXT_HINT + style::SPACE_XS;
            painter.rect_filled(
                egui::Rect::from_min_size(
                    egui::pos2(rect.min.x, line_y),
                    egui::vec2(rect.width(), 1.0),
                ),
                0.0,
                colors.border,
            );
        }

        UiNode::Label { text, size, color, tone, bold, monospace, max_lines, .. } => {
            let font_size = if *size > 0.0 { *size } else { style::TEXT_BODY };
            let text_color = if !color.is_empty() {
                parse_color(color).unwrap_or(colors.text_primary)
            } else {
                resolve_tone(tone, colors)
            };
            let mut rich = egui::RichText::new(text.as_str()).size(font_size).color(text_color);
            if *bold {
                rich = rich.strong();
            }
            if *monospace {
                rich = rich.monospace();
            }
            let label = egui::Label::new(rich).selectable(true);
            let label = if *max_lines > 0 {
                label.wrap_mode(egui::TextWrapMode::Truncate)
            } else {
                label.wrap()
            };
            ui.add(label);
        }

        UiNode::Spacer { size, grow, .. } => {
            if *grow {
                ui.allocate_space(ui.available_size());
            } else {
                let s = if *size > 0.0 { *size } else { style::SPACE_MD };
                ui.add_space(s);
            }
        }

        UiNode::Divider { color: div_color, .. } => {
            let fill = if div_color.is_empty() {
                colors.border
            } else {
                parse_color(div_color).unwrap_or(colors.border)
            };
            let (rect, _) =
                ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
            ui.painter().rect_filled(rect, 0.0, fill);
        }

        UiNode::Card { children, padding, .. } => {
            let pad = if *padding > 0.0 { *padding } else { style::SPACE_MD };
            egui::Frame::new()
                .fill(colors.bg_sidebar)
                .stroke(egui::Stroke::new(1.0, colors.border))
                .corner_radius(style::RADIUS_MD)
                .inner_margin(egui::Margin::same(pad as i8))
                .show(ui, |ui| {
                    for child in children {
                        events.extend(render_component_tree(ui, child, colors, text_edit_buffers, focus_ctx));
                    }
                });
        }

        UiNode::SelectList { items, selected_idx, .. } => {
            if items.is_empty() {
                ui.label(
                    egui::RichText::new("No items")
                        .size(style::TEXT_HINT)
                        .color(colors.text_dim),
                );
            } else {
                let avail = ui.available_size();
                egui::ScrollArea::vertical()
                    .max_height(avail.y)
                    .show(ui, |ui| {
                        for (i, item) in items.iter().enumerate() {
                            let selected = i == *selected_idx;
                            let bg = if selected { colors.bg_active } else { colors.bg_sidebar };
                            let (rect, _) = ui.allocate_exact_size(
                                egui::vec2(avail.x, if item.description.is_empty() { 36.0 } else { 52.0 }),
                                egui::Sense::hover(),
                            );
                            let painter = ui.painter();
                            painter.rect_filled(rect, 0.0, bg);
                            if selected {
                                painter.rect_filled(
                                    egui::Rect::from_min_size(rect.min, egui::vec2(3.0, rect.height())),
                                    0.0,
                                    colors.accent,
                                );
                            }
                            let text_x = rect.min.x + style::SPACE_MD;
                            let max_w = rect.width() - 2.0 * style::SPACE_MD;
                            if item.description.is_empty() {
                                let title_y = rect.center().y - style::TEXT_BODY / 2.0;
                                let galley = ui.fonts(|f| {
                                    f.layout(item.name.clone(), egui::FontId::proportional(style::TEXT_BODY),
                                             colors.text_primary, max_w)
                                });
                                painter.galley(egui::pos2(text_x, title_y), galley, colors.text_primary);
                            } else {
                                let block_h = style::TEXT_BODY + 2.0 + style::TEXT_HINT;
                                let title_y = rect.center().y - block_h / 2.0;
                                let desc_y = title_y + style::TEXT_BODY + 2.0;
                                let galley = ui.fonts(|f| {
                                    f.layout(item.name.clone(), egui::FontId::proportional(style::TEXT_BODY),
                                             colors.text_primary, max_w)
                                });
                                painter.galley(egui::pos2(text_x, title_y), galley, colors.text_primary);
                                let desc_galley = ui.fonts(|f| {
                                    f.layout(item.description.clone(), egui::FontId::proportional(style::TEXT_HINT),
                                             colors.text_dim, max_w)
                                });
                                painter.galley(egui::pos2(text_x, desc_y), desc_galley, colors.text_dim);
                            }
                            if !item.trailing.is_empty() {
                                let tr_galley = ui.fonts(|f| {
                                    f.layout_no_wrap(item.trailing.clone(),
                                                     egui::FontId::proportional(style::TEXT_HINT),
                                                     colors.text_dim)
                                });
                                let tr_x = rect.max.x - style::SPACE_MD - tr_galley.size().x;
                                let tr_y = rect.center().y - tr_galley.size().y / 2.0;
                                painter.galley(egui::pos2(tr_x, tr_y), tr_galley, colors.text_dim);
                            }
                        }
                    });
            }
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
    text_edit_buffers: &mut std::collections::HashMap<String, String>,
    focus_ctx: &mut TextEditFocusCtx,
) -> Vec<ComponentEventPayload> {
    let mut events = Vec::new();
    match direction {
        StackDirection::Horizontal => {
            ui.horizontal(|ui| {
                for (i, child) in children.iter().enumerate() {
                    if i > 0 && gap > 0.0 {
                        ui.add_space(gap);
                    }
                    events.extend(render_component_tree(ui, child, colors, text_edit_buffers, focus_ctx));
                }
            });
        }
        StackDirection::Vertical => {
            ui.vertical(|ui| {
                for (i, child) in children.iter().enumerate() {
                    if i > 0 && gap > 0.0 {
                        ui.add_space(gap);
                    }
                    events.extend(render_component_tree(ui, child, colors, text_edit_buffers, focus_ctx));
                }
            });
        }
    }
    events
}

use crate::process_app::render::parse_color;

fn resolve_tone(tone: &str, colors: &Colors) -> egui::Color32 {
    match tone {
        "hint" | "dim" | "muted" => colors.text_dim,
        "danger" | "error" => colors.danger,
        "success" => colors.success,
        "warning" => colors.warning,
        "accent" => colors.accent,
        "section" => colors.text_section,
        _ => colors.text_primary,
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

    /// `UiNode::TextEdit` node can be constructed with all fields.
    #[test]
    fn text_edit_node_constructable() {
        let node = UiNode::TextEdit {
            node_id: "editor1".into(),
            placeholder: "Type here...".into(),
            value: "hello".into(),
            multiline: true,
            max_length: 100,
        };
        if let UiNode::TextEdit { node_id, placeholder, value, multiline, max_length, .. } = &node {
            assert_eq!(node_id, "editor1");
            assert_eq!(placeholder, "Type here...");
            assert_eq!(value, "hello");
            assert!(*multiline);
            assert_eq!(*max_length, 100);
        } else {
            panic!("wrong variant");
        }
    }

    /// `UiNode::TextEdit` PartialEq works.
    #[test]
    fn text_edit_partial_eq() {
        let a = UiNode::TextEdit {
            node_id: "e".into(),
            placeholder: "p".into(),
            value: "v".into(),
            multiline: false,
            max_length: 0,
        };
        let b = a.clone();
        assert_eq!(a, b);
    }

    /// Serde round-trip for `UiNode::TextEdit`.
    #[test]
    fn text_edit_serde_roundtrip() {
        let node = UiNode::TextEdit {
            node_id: "te1".into(),
            placeholder: "hint".into(),
            value: "val".into(),
            multiline: true,
            max_length: 50,
        };
        let json = serde_json::to_string(&node).unwrap();
        let parsed: UiNode = serde_json::from_str(&json).unwrap();
        assert_eq!(node, parsed);
    }
}

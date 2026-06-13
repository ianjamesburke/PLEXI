//! Component tree renderer — walks a `UiNode` tree and renders it into egui.
//!
//! This is the host-side counterpart to the `RenderCommand::ComponentTree`
//! protocol variant introduced in PGAP v3.5. Interactive nodes (`Button`,
//! `Input`, `Interactive`) fire `ComponentEvent`s back to the app via the
//! returned `Vec<ComponentEventPayload>` (task A3).

use egui::Ui;

use crate::app_protocol::{PinnedEdge, StackDirection, UiNode};
use crate::ui::style;
use crate::ui::theme::Colors;

/// Persistent per-pane render resources threaded into `UiNode::Raw` nodes.
///
/// Raw escape-hatch nodes delegate to `render_draw_commands`, which needs the
/// pane's long-lived caches (markdown, images) and per-widget state maps.
/// Threading these from the parent render pass instead of creating throwaway
/// instances per node per frame avoids re-allocating caches every frame and
/// lets Raw-embedded markdown/images/list views keep state across frames,
/// matching top-level draw-command behavior.
pub(crate) struct RawNodeCaches<'a> {
    pub(crate) commonmark_cache: &'a mut egui_commonmark::CommonMarkCache,
    pub(crate) image_cache: &'a mut crate::process_app::image_cache::ImageCache,
    pub(crate) audio_peaks: &'a std::collections::HashMap<String, f32>,
    pub(crate) workspace_root: &'a std::path::Path,
    pub(crate) net_http_granted: bool,
    pub(crate) list_view_scroll_offsets: &'a mut std::collections::HashMap<String, f32>,
    pub(crate) list_view_last_aligned_sel: &'a mut std::collections::HashMap<String, usize>,
}

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
    raw_caches: &mut RawNodeCaches<'_>,
) -> Vec<ComponentEventPayload> {
    let mut events: Vec<ComponentEventPayload> = Vec::new();

    match node {
        // ── L0 primitives ────────────────────────────────────────────────
        UiNode::Stack {
            direction,
            children,
            gap,
            padding,
        } => {
            egui::Frame::new()
                .inner_margin(egui::Margin {
                    left: padding.left as i8,
                    right: padding.right as i8,
                    top: padding.top as i8,
                    bottom: padding.bottom as i8,
                })
                .show(ui, |ui| {
                    events.extend(render_stack(
                        ui,
                        direction,
                        children,
                        *gap,
                        colors,
                        text_edit_buffers,
                        focus_ctx,
                        raw_caches,
                    ));
                });
        }

        UiNode::Scroll { child, horizontal } => {
            let scroll = if *horizontal {
                egui::ScrollArea::both()
            } else {
                egui::ScrollArea::vertical()
            };
            scroll.show(ui, |ui| {
                events.extend(render_component_tree(
                    ui,
                    child,
                    colors,
                    text_edit_buffers,
                    focus_ctx,
                    raw_caches,
                ));
            });
        }

        UiNode::Layer { children } => {
            // V1: sequential rendering (true Z-stacking is a future improvement).
            for child in children {
                events.extend(render_component_tree(
                    ui,
                    child,
                    colors,
                    text_edit_buffers,
                    focus_ctx,
                    raw_caches,
                ));
            }
        }

        UiNode::Text {
            text,
            size,
            color,
            bold,
            monospace,
        } => {
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

        UiNode::Interactive {
            node_id,
            child,
            on_click,
            on_hover,
        } => {
            // Render the child inside an interact-sense scope so we get a
            // Response covering the child's bounding rect.
            let child_response = ui.scope(|ui| {
                let child_evts = render_component_tree(
                    ui,
                    child,
                    colors,
                    text_edit_buffers,
                    focus_ctx,
                    raw_caches,
                );
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
                log::info!("render_components: Interactive click node_id={node_id}");
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

        UiNode::Pinned { edge, child } => {
            // Bottom-pinned nodes are handled by render_stack's partition pass.
            // When Pinned appears outside a vertical Stack (e.g. at tree root), render inline.
            log::trace!(
                "render_components: Pinned {:?} rendered inline (no enclosing vertical Stack)",
                edge
            );
            events.extend(render_component_tree(
                ui,
                child,
                colors,
                text_edit_buffers,
                focus_ctx,
                raw_caches,
            ));
        }

        UiNode::Raw { command } => {
            // Delegate to the existing flat renderer for a single draw command,
            // threading the pane's persistent caches so markdown/image/list
            // state survives across frames instead of being rebuilt per node.
            let pane_rect = ui.clip_rect();
            let mut raw_events: Vec<crate::app_protocol::PlexiEvent> = Vec::new();
            // Raw escape-hatch uses a throwaway focus ctx — focus tracking doesn't
            // apply to legacy draw commands embedded inside a component tree.
            // (Fresh HashSets are allocation-free until first insert.)
            let mut raw_focus_ctx = TextEditFocusCtx::new();
            crate::process_app::render::render_draw_commands(
                ui,
                pane_rect,
                std::slice::from_ref(command.as_ref()),
                colors,
                &mut *raw_caches.commonmark_cache,
                raw_caches.audio_peaks,
                &mut *raw_caches.image_cache,
                raw_caches.workspace_root,
                raw_caches.net_http_granted,
                &mut *raw_caches.list_view_scroll_offsets,
                &mut *raw_caches.list_view_last_aligned_sel,
                &mut raw_events,
                text_edit_buffers,
                &mut raw_focus_ctx,
            );
            // Convert any ComponentEvent payloads back from PlexiEvent (unlikely
            // from a Raw draw command, but keep the pipeline consistent).
            // Non-ComponentEvent PlexiEvents are intentionally dropped here,
            // preserving pre-existing Raw-node behavior.
            for evt in raw_events {
                if let crate::app_protocol::PlexiEvent::ComponentEvent {
                    node_id,
                    event_type,
                    payload,
                } = evt
                {
                    events.push(ComponentEventPayload {
                        node_id,
                        event_type,
                        payload,
                    });
                }
            }
        }

        UiNode::Surface { .. } => {
            // Reserved for future GPU surface layer — no-op.
            log::trace!("render_components: Surface node encountered — no-op (future GPU layer)");
        }

        // ── L1 sugar ─────────────────────────────────────────────────────────
        UiNode::Button {
            node_id,
            label,
            disabled,
            ..
        } => {
            const BTN_PAD_V: f32 = 5.0;
            let font_id = egui::FontId::proportional(crate::ui::style::TEXT_BODY);
            // Layout with placeholder color; painter.galley() overrides per-state below.
            let galley =
                ui.fonts(|f| f.layout_no_wrap(label.clone(), font_id, egui::Color32::WHITE));
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
            let fill = if is_down {
                colors.accent
            } else {
                colors.bg_active
            };
            painter.rect_filled(rect, crate::ui::style::RADIUS_MD, fill);
            if !*disabled {
                let stroke_color = if is_hovered {
                    colors.accent
                } else {
                    colors.border
                };
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
            let text_color = if *disabled {
                colors.text_dim
            } else if is_down {
                colors.bg_active
            } else {
                colors.text_primary
            };
            let text_pos = egui::pos2(
                rect.center().x - text_w / 2.0,
                rect.center().y - text_h / 2.0,
            );
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

        UiNode::Input {
            node_id,
            value,
            placeholder,
            ..
        } => {
            let mut val_buf = value.clone();
            let response = crate::ui::text_field::styled_text_input(
                ui,
                &mut val_buf,
                placeholder.as_str(),
                egui::Id::new(node_id.as_str()),
                colors,
            );
            if response.changed() {
                log::debug!("render_components: Input change node_id={node_id} value={val_buf:?}");
                events.push(ComponentEventPayload {
                    node_id: node_id.clone(),
                    event_type: "change".into(),
                    payload: Some(serde_json::json!({ "value": val_buf })),
                });
            }
            if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                log::info!("render_components: Input submit node_id={node_id} value={val_buf:?}");
                events.push(ComponentEventPayload {
                    node_id: node_id.clone(),
                    event_type: "submit".into(),
                    payload: Some(serde_json::json!({ "value": val_buf })),
                });
            }
        }

        UiNode::TextEdit {
            node_id,
            placeholder,
            value,
            multiline,
            max_length,
            ..
        } => {
            // Seed the buffer from the app's value when this node_id first appears.
            // contains_key-then-insert avoids cloning node_id on the steady-state
            // (already-seeded) path, unlike `entry(node_id.clone())`.
            if !text_edit_buffers.contains_key(node_id.as_str()) {
                text_edit_buffers.insert(node_id.clone(), value.clone());
            }
            let buffer = text_edit_buffers
                .get_mut(node_id.as_str())
                .expect("text_edit_buffers: buffer seeded above");

            // Enforce max_length by truncating the buffer if it exceeds the limit.
            if *max_length > 0 && buffer.len() > *max_length {
                buffer.truncate(*max_length);
            }

            // Track visibility for auto-focus detection.
            let newly_visible = !focus_ctx.prev_visible.contains(node_id.as_str());
            focus_ctx.current_visible.insert(node_id.clone());

            let widget_id = egui::Id::new(("text_edit_node", node_id.as_str()));

            // Styling matches QuickNote overlay: frameless, monospace, accent caret,
            // dim hint text. See src/overlays/quick_note.rs lines 138-154.
            let response = ui
                .scope(|ui| {
                    // egui's caret is hidden (transparent, non-blinking);
                    // draw_text_caret paints a glyph-height replacement on top.
                    ui.visuals_mut().text_cursor.blink = false;
                    ui.visuals_mut().text_cursor.stroke.color = egui::Color32::TRANSPARENT;
                    let font_id = egui::FontId::monospace(style::TEXT_BODY);
                    let row_height = ui.fonts(|f| f.row_height(&font_id));

                    let output = if *multiline {
                        let hint = egui::RichText::new(placeholder.as_str())
                            .color(colors.text_dim.linear_multiply(0.3))
                            .size(style::TEXT_BODY);
                        let mut edit = egui::TextEdit::multiline(buffer)
                            .id(widget_id)
                            .font(font_id.clone())
                            .text_color(colors.text_primary)
                            .desired_width(f32::INFINITY)
                            .frame(false)
                            .hint_text(hint);
                        if *max_length > 0 {
                            edit = edit.char_limit(*max_length);
                        }
                        edit.show(ui)
                    } else {
                        let hint = egui::RichText::new(placeholder.as_str())
                            .color(colors.text_dim.linear_multiply(0.3))
                            .size(style::TEXT_BODY);
                        let mut edit = egui::TextEdit::singleline(buffer)
                            .id(widget_id)
                            .font(font_id.clone())
                            .text_color(colors.text_primary)
                            .desired_width(f32::INFINITY)
                            .frame(false)
                            .hint_text(hint);
                        if *max_length > 0 {
                            edit = edit.char_limit(*max_length);
                        }
                        edit.show(ui)
                    };
                    crate::ui::text_field::draw_text_caret(
                        ui,
                        &output,
                        style::TEXT_BODY,
                        row_height,
                        egui::Stroke::new(1.5, colors.accent),
                    );
                    output.response
                })
                .inner;

            // Auto-focus: first newly-visible TextEdit, or first TextEdit when
            // the pane just gained keyboard focus.
            if (newly_visible || focus_ctx.pane_just_focused) && !focus_ctx.focus_granted_this_frame
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
                log::debug!("render_components: TextEdit click-focus node_id={node_id}");
            }

            // Track focus for key suppression.
            if response.has_focus() {
                focus_ctx.any_has_focus = true;
            }

            // Emit "change" event when the TextEdit mutated the value this
            // frame. response.changed() avoids the per-frame full-buffer
            // clone+compare the old prev_value snapshot required.
            if response.changed() {
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
                    && ui.input(|i| i.key_pressed(egui::Key::Enter) && i.modifiers.command)
            } else {
                response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))
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

        UiNode::Badge {
            label, fill, fg, ..
        } => {
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
                .corner_radius(egui::CornerRadius::same(
                    crate::ui::style::RADIUS_BADGE as u8,
                ))
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
            let (rect, _) =
                ui.allocate_exact_size(egui::vec2(dot_size, dot_size), egui::Sense::hover());
            ui.painter()
                .circle_filled(rect.center(), dot_size / 2.0, fill);
        }

        // ── L1 layout components ────────────────────────────────────────
        UiNode::Column {
            children,
            gap,
            padding_top,
            padding,
        } => {
            log::info!(
                "render_components: Column {} children gap={gap} padding_top={padding_top} padding={padding}",
                children.len()
            );
            // Skip top padding when the first child is AppBar (it's full-bleed chrome)
            let effective_top = match children.first() {
                Some(UiNode::AppBar { .. }) => 0,
                _ => *padding_top as i8,
            };
            egui::Frame::new()
                .inner_margin(egui::Margin {
                    left: *padding as i8,
                    right: *padding as i8,
                    top: effective_top,
                    bottom: 0,
                })
                .show(ui, |ui| {
                    // Expand the inner ui to fill available height so render_stack's
                    // sticky-footer partition can see the full remaining pane height.
                    let h = ui.available_height();
                    ui.set_min_height(h);
                    events.extend(render_stack(
                        ui,
                        &StackDirection::Vertical,
                        children,
                        *gap,
                        colors,
                        text_edit_buffers,
                        focus_ctx,
                        raw_caches,
                    ));
                });
        }

        UiNode::AppBar {
            title, subtitle, ..
        } => {
            const TITLE_SIZE: f32 = 16.0;
            let has_subtitle = !subtitle.is_empty();
            let band_h = if has_subtitle { 48.0 } else { 34.0 };
            let total_h = band_h + 1.0;
            let (rect, _) = ui.allocate_exact_size(
                egui::vec2(ui.available_width(), total_h),
                egui::Sense::hover(),
            );
            let painter = ui.painter();
            // Full-bleed: expand to clip rect width so AppBar spans the pane edge-to-edge
            let clip = ui.clip_rect();
            let full_rect = egui::Rect::from_min_size(
                egui::pos2(clip.min.x, rect.min.y),
                egui::vec2(clip.width(), total_h),
            );
            painter.rect_filled(full_rect, 0.0, colors.bg_sidebar);
            let text_x = full_rect.min.x + style::SPACE_MD;
            let max_w = full_rect.width() - 2.0 * style::SPACE_MD;
            let title_y = full_rect.min.y + style::SPACE_SM;
            if has_subtitle {
                let sub_y = title_y + TITLE_SIZE + style::SPACE_XS;
                let title_galley = ui.fonts(|f| {
                    f.layout(
                        title.clone(),
                        egui::FontId::proportional(TITLE_SIZE),
                        colors.text_primary,
                        max_w,
                    )
                });
                painter.galley(
                    egui::pos2(text_x, title_y),
                    title_galley,
                    colors.text_primary,
                );
                let sub_galley = ui.fonts(|f| {
                    f.layout(
                        subtitle.clone(),
                        egui::FontId::proportional(style::TEXT_HINT),
                        colors.text_dim,
                        max_w,
                    )
                });
                painter.galley(egui::pos2(text_x, sub_y), sub_galley, colors.text_dim);
            } else {
                let title_galley = ui.fonts(|f| {
                    f.layout(
                        title.clone(),
                        egui::FontId::proportional(TITLE_SIZE),
                        colors.text_primary,
                        max_w,
                    )
                });
                painter.galley(
                    egui::pos2(text_x, title_y),
                    title_galley,
                    colors.text_primary,
                );
            }
            painter.rect_filled(
                egui::Rect::from_min_size(
                    egui::pos2(full_rect.min.x, full_rect.min.y + band_h),
                    egui::vec2(full_rect.width(), 1.0),
                ),
                0.0,
                colors.border,
            );
        }

        UiNode::FooterKeys {
            entries, divider, ..
        } => {
            let chip_h = style::TEXT_HINT + 4.0;
            let chip_row_h = style::TEXT_HINT + 6.0;
            let total_h = if *divider {
                1.0 + style::SPACE_SM + chip_row_h + style::SPACE_SM
            } else {
                style::SPACE_SM + chip_row_h + style::SPACE_SM
            };
            let (rect, _) = ui.allocate_exact_size(
                egui::vec2(ui.available_width(), total_h),
                egui::Sense::hover(),
            );

            // Full-bleed: expand to clip rect width so footer spans the pane edge-to-edge
            let clip = ui.clip_rect();
            let full_rect = egui::Rect::from_min_size(
                egui::pos2(clip.min.x, rect.min.y),
                egui::vec2(clip.width(), total_h),
            );
            ui.painter().rect_filled(full_rect, 0.0, colors.bg_sidebar);

            if *divider {
                ui.painter().rect_filled(
                    egui::Rect::from_min_size(
                        egui::pos2(full_rect.min.x, full_rect.min.y),
                        egui::vec2(full_rect.width(), 1.0),
                    ),
                    0.0,
                    colors.border,
                );
            }

            // Measure total content width so we can center the group horizontally.
            let chip_font = egui::FontId::monospace(style::TEXT_HINT);
            let desc_font = egui::FontId::proportional(style::TEXT_HINT);
            let mut content_w: f32 = 0.0;
            for (ei, entry) in entries.iter().enumerate() {
                for (ki, key) in entry.keys.iter().enumerate() {
                    if ki > 0 {
                        content_w += 2.0;
                    }
                    let tw = ui.fonts(|f| {
                        f.layout_no_wrap(key.clone(), chip_font.clone(), colors.text_primary)
                            .size()
                            .x
                    });
                    content_w += (tw + 8.0).max(chip_h);
                }
                content_w += 4.0;
                content_w += ui.fonts(|f| {
                    f.layout_no_wrap(
                        entry.description.clone(),
                        desc_font.clone(),
                        colors.text_dim,
                    )
                    .size()
                    .x
                });
                if ei + 1 < entries.len() {
                    content_w += style::SPACE_MD;
                }
            }

            // Center the content group inside the full-bleed band; clamp left edge.
            let left =
                (full_rect.center().x - content_w / 2.0).max(full_rect.min.x + style::SPACE_MD);
            let content_rect = egui::Rect::from_min_size(
                egui::pos2(left, full_rect.center().y - chip_h / 2.0),
                egui::vec2(content_w, chip_h),
            );

            ui.allocate_new_ui(egui::UiBuilder::new().max_rect(content_rect), |ui| {
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    for (ei, entry) in entries.iter().enumerate() {
                        for (ki, key) in entry.keys.iter().enumerate() {
                            if ki > 0 {
                                ui.add_space(2.0);
                            }
                            crate::ui::shortcuts::key_chip(ui, key, colors, chip_font.clone());
                        }
                        ui.add_space(4.0);
                        ui.label(
                            egui::RichText::new(entry.description.clone())
                                .size(style::TEXT_HINT)
                                .color(colors.text_dim),
                        );
                        if ei + 1 < entries.len() {
                            ui.add_space(style::SPACE_MD);
                        }
                    }
                });
            });
        }

        UiNode::Footer { text, color, .. } => {
            let line_h = style::TEXT_CAPTION + 5.0;
            let total_h = style::SPACE_MD + 1.0 + style::SPACE_MD + line_h;
            let (rect, _) = ui.allocate_exact_size(
                egui::vec2(ui.available_width(), total_h),
                egui::Sense::hover(),
            );
            let line_y = rect.min.y + style::SPACE_MD;
            ui.painter().rect_filled(
                egui::Rect::from_min_size(
                    egui::pos2(rect.min.x, line_y),
                    egui::vec2(rect.width(), 1.0),
                ),
                0.0,
                colors.border,
            );
            let text_color = if color.is_empty() {
                colors.text_dim
            } else {
                parse_color(color).unwrap_or(colors.text_dim)
            };
            // Content area sits below the divider line, vertically centered in
            // the remaining height. egui handles DPI/rounding internally.
            let content_rect = egui::Rect::from_min_size(
                egui::pos2(rect.min.x, line_y + 1.0),
                egui::vec2(rect.width(), style::SPACE_MD + line_h),
            );
            ui.allocate_new_ui(egui::UiBuilder::new().max_rect(content_rect), |ui| {
                ui.with_layout(
                    egui::Layout::centered_and_justified(egui::Direction::TopDown),
                    |ui| {
                        ui.label(
                            egui::RichText::new(text.clone())
                                .size(style::TEXT_CAPTION)
                                .color(text_color),
                        );
                    },
                );
            });
        }

        UiNode::Section { title, .. } => {
            let total_h =
                style::SPACE_SM + style::TEXT_HINT + style::SPACE_XS + 1.0 + style::SPACE_XS;
            let (rect, _) = ui.allocate_exact_size(
                egui::vec2(ui.available_width(), total_h),
                egui::Sense::hover(),
            );
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

        UiNode::Label {
            text,
            size,
            color,
            tone,
            bold,
            monospace,
            max_lines,
            ..
        } => {
            let font_size = if *size > 0.0 { *size } else { style::TEXT_BODY };
            let text_color = if !color.is_empty() {
                parse_color(color).unwrap_or(colors.text_primary)
            } else {
                resolve_tone(tone, colors)
            };
            let mut rich = egui::RichText::new(text.as_str())
                .size(font_size)
                .color(text_color);
            if *bold {
                rich = rich.strong();
            }
            if *monospace {
                rich = rich.monospace();
            }
            let label = egui::Label::new(rich).selectable(true).wrap();
            if *max_lines > 0 {
                let line_h = font_size + 4.0;
                let max_h = *max_lines as f32 * line_h;
                ui.scope(|ui| {
                    ui.set_max_height(max_h);
                    ui.add(label);
                });
            } else {
                ui.add(label);
            }
        }

        UiNode::Spacer { size, grow, .. } => {
            if *grow {
                ui.allocate_space(ui.available_size());
            } else {
                let s = if *size > 0.0 { *size } else { style::SPACE_MD };
                ui.add_space(s);
            }
        }

        UiNode::Divider {
            color: div_color, ..
        } => {
            let fill = if div_color.is_empty() {
                colors.border
            } else {
                parse_color(div_color).unwrap_or(colors.border)
            };
            let (rect, _) =
                ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
            ui.painter().rect_filled(rect, 0.0, fill);
        }

        UiNode::Card {
            children, padding, ..
        } => {
            let pad = if *padding > 0.0 {
                *padding
            } else {
                style::SPACE_MD
            };
            egui::Frame::new()
                .fill(colors.bg_sidebar)
                .stroke(egui::Stroke::new(1.0, colors.border))
                .corner_radius(style::RADIUS_MD)
                .inner_margin(egui::Margin::same(pad as i8))
                .show(ui, |ui| {
                    for child in children {
                        events.extend(render_component_tree(
                            ui,
                            child,
                            colors,
                            text_edit_buffers,
                            focus_ctx,
                            raw_caches,
                        ));
                    }
                });
        }

        UiNode::SelectList {
            items,
            selected_idx,
            ..
        } => {
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
                            let bg = if selected {
                                colors.bg_active
                            } else {
                                colors.bg_sidebar
                            };
                            let (rect, _) = ui.allocate_exact_size(
                                egui::vec2(
                                    avail.x,
                                    if item.description.is_empty() {
                                        36.0
                                    } else {
                                        52.0
                                    },
                                ),
                                egui::Sense::hover(),
                            );
                            let painter = ui.painter();
                            painter.rect_filled(rect, 0.0, bg);
                            if selected {
                                painter.rect_filled(
                                    egui::Rect::from_min_size(
                                        rect.min,
                                        egui::vec2(3.0, rect.height()),
                                    ),
                                    0.0,
                                    colors.accent,
                                );
                            }
                            let text_x = rect.min.x + style::SPACE_MD;
                            let max_w = rect.width() - 2.0 * style::SPACE_MD;
                            if item.description.is_empty() {
                                let title_y = rect.center().y - style::TEXT_BODY / 2.0;
                                let galley = ui.fonts(|f| {
                                    f.layout(
                                        item.name.clone(),
                                        egui::FontId::proportional(style::TEXT_BODY),
                                        colors.text_primary,
                                        max_w,
                                    )
                                });
                                painter.galley(
                                    egui::pos2(text_x, title_y),
                                    galley,
                                    colors.text_primary,
                                );
                            } else {
                                let block_h = style::TEXT_BODY + 2.0 + style::TEXT_HINT;
                                let title_y = rect.center().y - block_h / 2.0;
                                let desc_y = title_y + style::TEXT_BODY + 2.0;
                                let galley = ui.fonts(|f| {
                                    f.layout(
                                        item.name.clone(),
                                        egui::FontId::proportional(style::TEXT_BODY),
                                        colors.text_primary,
                                        max_w,
                                    )
                                });
                                painter.galley(
                                    egui::pos2(text_x, title_y),
                                    galley,
                                    colors.text_primary,
                                );
                                let desc_galley = ui.fonts(|f| {
                                    f.layout(
                                        item.description.clone(),
                                        egui::FontId::proportional(style::TEXT_HINT),
                                        colors.text_dim,
                                        max_w,
                                    )
                                });
                                painter.galley(
                                    egui::pos2(text_x, desc_y),
                                    desc_galley,
                                    colors.text_dim,
                                );
                            }
                            if !item.trailing.is_empty() {
                                let tr_galley = ui.fonts(|f| {
                                    f.layout_no_wrap(
                                        item.trailing.clone(),
                                        egui::FontId::proportional(style::TEXT_HINT),
                                        colors.text_dim,
                                    )
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

/// Returns the known fixed height of a node that can be bottom-pinned, or `None`
/// if the height cannot be determined without rendering.
fn bottom_pin_height(node: &UiNode) -> Option<f32> {
    match node {
        UiNode::FooterKeys { divider, .. } => {
            let chip_row_h = style::TEXT_HINT + 6.0;
            Some(if *divider {
                1.0 + style::SPACE_SM + chip_row_h + style::SPACE_SM
            } else {
                style::SPACE_SM + chip_row_h + style::SPACE_SM
            })
        }
        UiNode::Footer { .. } => {
            Some(style::SPACE_MD + 1.0 + style::SPACE_MD + style::TEXT_CAPTION + 5.0)
        }
        _ => None,
    }
}

fn render_stack(
    ui: &mut Ui,
    direction: &StackDirection,
    children: &[UiNode],
    gap: f32,
    colors: &Colors,
    text_edit_buffers: &mut std::collections::HashMap<String, String>,
    focus_ctx: &mut TextEditFocusCtx,
    raw_caches: &mut RawNodeCaches<'_>,
) -> Vec<ComponentEventPayload> {
    let mut events = Vec::new();
    match direction {
        StackDirection::Horizontal => {
            ui.horizontal(|ui| {
                for (i, child) in children.iter().enumerate() {
                    if i > 0 && gap > 0.0 {
                        ui.add_space(gap);
                    }
                    events.extend(render_component_tree(
                        ui,
                        child,
                        colors,
                        text_edit_buffers,
                        focus_ctx,
                        raw_caches,
                    ));
                }
            });
        }
        StackDirection::Vertical => {
            // Partition bottom-pinned children (those with a known fixed height) out of
            // the body. They are rendered at the bottom of the available rect by
            // constraining the body height first.
            //
            // Two sources of pinning:
            //   1. Explicit: `Pinned { edge: Bottom, child }` wrapper
            //   2. Implicit: FooterKeys/Footer at the tail of the children list
            //      (the SDK doesn't wrap them in Pinned, but they always pin to bottom)
            let mut pinned_bottom: Vec<(f32, &UiNode)> = Vec::new();
            let mut body_children: Vec<&UiNode> = Vec::new();

            for child in children {
                if let UiNode::Pinned {
                    edge: PinnedEdge::Bottom,
                    child: inner,
                } = child
                {
                    if let Some(h) = bottom_pin_height(inner) {
                        pinned_bottom.push((h, inner.as_ref()));
                        continue;
                    } else {
                        log::warn!(
                            "render_components: Pinned{{Bottom}} wraps a node with unknown intrinsic height — rendering inline; add it to bottom_pin_height()"
                        );
                    }
                }
                body_children.push(child);
            }

            // Auto-pin: pull FooterKeys/Footer off the tail of body_children
            while let Some(last) = body_children.last() {
                if let Some(h) = bottom_pin_height(last) {
                    pinned_bottom.push((h, body_children.pop().unwrap()));
                } else {
                    break;
                }
            }
            // Reverse so they render in original order (we popped from the end)
            pinned_bottom.reverse();

            if !pinned_bottom.is_empty() {
                let total_pinned_h: f32 = pinned_bottom.iter().map(|(h, _)| h).sum();
                let body_h = (ui.available_height() - total_pinned_h).max(0.0);

                // Body gets a constrained-height allocation; pinned fills the rest below.
                // set_min_height forces the cursor to advance by the full body_h even when
                // content is short, ensuring the footer renders flush to the bottom.
                ui.allocate_ui(egui::vec2(ui.available_width(), body_h), |ui| {
                    ui.set_min_height(body_h);
                    for (i, child) in body_children.iter().enumerate() {
                        if i > 0 && gap > 0.0 {
                            ui.add_space(gap);
                        }
                        events.extend(render_component_tree(
                            ui,
                            child,
                            colors,
                            text_edit_buffers,
                            focus_ctx,
                            raw_caches,
                        ));
                    }
                });
                for (_, inner) in &pinned_bottom {
                    events.extend(render_component_tree(
                        ui,
                        inner,
                        colors,
                        text_edit_buffers,
                        focus_ctx,
                        raw_caches,
                    ));
                }
                log::info!(
                    "render_components: render_stack vertical pinned body_h={body_h:.0} footer_h={total_pinned_h:.0}"
                );
            } else {
                ui.vertical(|ui| {
                    for (i, child) in children.iter().enumerate() {
                        if i > 0 && gap > 0.0 {
                            ui.add_space(gap);
                        }
                        events.extend(render_component_tree(
                            ui,
                            child,
                            colors,
                            text_edit_buffers,
                            focus_ctx,
                            raw_caches,
                        ));
                    }
                });
            }
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
        let node = UiNode::Surface {
            id: "canvas".into(),
        };
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
        if let UiNode::Button {
            node_id,
            label,
            disabled,
            ..
        } = &node
        {
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
        if let UiNode::Interactive {
            node_id,
            on_click,
            on_hover,
            ..
        } = &node
        {
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
        if let UiNode::TextEdit {
            node_id,
            placeholder,
            value,
            multiline,
            max_length,
            ..
        } = &node
        {
            assert_eq!(node_id, "editor1");
            assert_eq!(placeholder, "Type here...");
            assert_eq!(value, "hello");
            assert!(*multiline);
            assert_eq!(*max_length, 100);
        } else {
            panic!("wrong variant");
        }
    }

    /// `UiNode::Pinned` can be constructed and matches correctly.
    #[test]
    fn pinned_node_constructable() {
        use crate::app_protocol::PinnedEdge;
        let inner = UiNode::Text {
            text: "footer".into(),
            size: 12.0,
            color: String::new(),
            bold: false,
            monospace: false,
        };
        let node = UiNode::Pinned {
            edge: PinnedEdge::Bottom,
            child: Box::new(inner),
        };
        if let UiNode::Pinned { edge, .. } = &node {
            assert_eq!(*edge, PinnedEdge::Bottom);
        } else {
            panic!("wrong variant");
        }
    }

    /// `UiNode::Column` can be constructed with children.
    #[test]
    fn column_node_constructable() {
        let node = UiNode::Column {
            children: vec![UiNode::Text {
                text: "body".into(),
                size: 14.0,
                color: String::new(),
                bold: false,
                monospace: false,
            }],
            gap: 8.0,
            padding_top: 0.0,
            padding: crate::ui::style::SPACE_XL,
        };
        if let UiNode::Column { children, gap, .. } = &node {
            assert_eq!(children.len(), 1);
            assert_eq!(*gap, 8.0);
        } else {
            panic!("wrong variant");
        }
    }

    /// Serde round-trip for `UiNode::Pinned`.
    #[test]
    fn pinned_serde_roundtrip() {
        use crate::app_protocol::PinnedEdge;
        let node = UiNode::Pinned {
            edge: PinnedEdge::Bottom,
            child: Box::new(UiNode::Footer {
                text: "status".into(),
                color: String::new(),
            }),
        };
        let json = serde_json::to_string(&node).unwrap();
        assert!(json.contains("\"type\":\"pinned\""), "json={json}");
        assert!(json.contains("\"bottom\""), "json={json}");
        let parsed: UiNode = serde_json::from_str(&json).unwrap();
        assert_eq!(node, parsed);
    }

    /// Serde round-trip for `UiNode::Column`.
    #[test]
    fn column_serde_roundtrip() {
        let node = UiNode::Column {
            children: vec![UiNode::AppBar {
                title: "T".into(),
                subtitle: String::new(),
            }],
            gap: 4.0,
            padding_top: 8.0,
            padding: 0.0,
        };
        let json = serde_json::to_string(&node).unwrap();
        assert!(json.contains("\"type\":\"column\""), "json={json}");
        let parsed: UiNode = serde_json::from_str(&json).unwrap();
        assert_eq!(node, parsed);
    }

    /// `bottom_pin_height` returns correct heights for FooterKeys and Footer.
    #[test]
    fn bottom_pin_height_known_nodes() {
        use super::bottom_pin_height;
        use crate::ui::style;
        let fk_with_div = UiNode::FooterKeys {
            entries: vec![],
            divider: true,
        };
        let fk_no_div = UiNode::FooterKeys {
            entries: vec![],
            divider: false,
        };
        let footer = UiNode::Footer {
            text: "s".into(),
            color: String::new(),
        };
        let label = UiNode::Label {
            text: "x".into(),
            size: 0.0,
            color: String::new(),
            tone: String::new(),
            bold: false,
            monospace: false,
            max_lines: 0,
        };

        let chip_row_h = style::TEXT_HINT + 6.0;
        let expected_with_div = 1.0 + style::SPACE_SM + chip_row_h + style::SPACE_SM;
        let expected_no_div = style::SPACE_SM + chip_row_h + style::SPACE_SM;
        let expected_footer = style::SPACE_MD + 1.0 + style::SPACE_MD + style::TEXT_CAPTION + 5.0;

        assert_eq!(bottom_pin_height(&fk_with_div), Some(expected_with_div));
        assert_eq!(bottom_pin_height(&fk_no_div), Some(expected_no_div));
        assert_eq!(bottom_pin_height(&footer), Some(expected_footer));
        assert_eq!(bottom_pin_height(&label), None);
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

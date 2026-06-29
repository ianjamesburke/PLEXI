//! Component tree renderer — walks a `UiNode` tree and renders it into egui.
//!
//! This is the host-side counterpart to the `RenderCommand::ComponentTree`
//! protocol variant introduced in PGAP v3.5. Interactive nodes (`Button`,
//! `Input`, `Interactive`) fire `ComponentEvent`s back to the app via the
//! returned `Vec<ComponentEventPayload>` (task A3).

use std::sync::atomic::{AtomicBool, Ordering};

use egui::Ui;

use crate::app_protocol::{PinnedEdge, StackDirection, UiNode};
use crate::render::app_chrome::{self, AppChrome};
use crate::ui::theme::Colors;
use crate::ui::{button, style};

static APP_CHROME_INFO_LOGGED: AtomicBool = AtomicBool::new(false);

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

/// Result from rendering a component tree — includes both interaction events
/// and layout feedback (e.g. the actual rendered canvas dimensions).
pub(crate) struct ComponentTreeResult {
    pub(crate) events: Vec<ComponentEventPayload>,
    /// Actual rendered canvas dimensions (0×0 if tree had no Canvas node).
    pub(crate) canvas_width: f32,
    pub(crate) canvas_height: f32,
    /// Hit regions collected from Canvas nodes during rendering.
    pub(crate) hit_regions: Vec<(egui::Rect, String)>,
}

/// Validate the same vertical shell slots used by the component renderer.
///
/// This is intentionally geometry-level, not pixel-diff-level: app checks use it
/// to prove that current scaffold chrome resolves to non-overlapping body,
/// action, and footer regions at a given viewport size.
pub(crate) fn validate_shell_layout(
    root: &UiNode,
    viewport_width: f32,
    viewport_height: f32,
) -> Vec<String> {
    let mut errors = Vec::new();
    if viewport_width <= 0.0 || viewport_height <= 0.0 {
        errors.push(format!(
            "viewport must be positive, got {viewport_width:.0}x{viewport_height:.0}"
        ));
        return errors;
    }

    let ctx = egui::Context::default();
    ctx.begin_pass(egui::RawInput::default());
    egui::CentralPanel::default().show(&ctx, |ui| {
        errors.extend(validate_shell_layout_inner(
            ui,
            root,
            viewport_width,
            viewport_height,
        ));
    });
    let _ = ctx.end_pass();

    errors
}

fn validate_shell_layout_inner(
    ui: &egui::Ui,
    root: &UiNode,
    viewport_width: f32,
    viewport_height: f32,
) -> Vec<String> {
    match root {
        UiNode::Column {
            children,
            gap,
            padding_top,
            padding,
        } => {
            let effective_top = match children.first() {
                Some(UiNode::AppBar { .. }) => 0.0,
                _ => (*padding_top).max(0.0),
            };
            let content_padding = semantic_shell_padding(children, *padding).max(0.0);
            let content_width = viewport_width - content_padding * 2.0;
            validate_vertical_shell(
                ui,
                children,
                *gap,
                content_width,
                viewport_height - effective_top,
            )
        }
        UiNode::Stack {
            direction: StackDirection::Vertical,
            children,
            gap,
            padding,
        } => {
            let content_width = viewport_width - padding.left.max(0.0) - padding.right.max(0.0);
            let content_height = viewport_height - padding.top.max(0.0) - padding.bottom.max(0.0);
            validate_vertical_shell(ui, children, *gap, content_width, content_height)
        }
        UiNode::Stack { .. } => vec!["root stack must be vertical for shell layout".to_string()],
        _ => vec!["root must be a column or vertical stack for shell layout".to_string()],
    }
}

fn validate_vertical_shell(
    ui: &egui::Ui,
    children: &[UiNode],
    gap: f32,
    content_width: f32,
    content_height: f32,
) -> Vec<String> {
    const EPSILON: f32 = 0.5;

    let mut errors = Vec::new();
    if content_width < -EPSILON {
        errors.push(format!(
            "horizontal padding exceeds viewport width by {:.1}px",
            -content_width
        ));
    }
    if content_height < -EPSILON {
        errors.push(format!(
            "shell padding exceeds viewport height by {:.1}px",
            -content_height
        ));
        return errors;
    }
    if gap < 0.0 {
        errors.push(format!("shell gap must be non-negative, got {gap:.1}"));
    }

    let mut pinned_bottom: Vec<(usize, f32, &UiNode)> = Vec::new();
    let mut body_children: Vec<(usize, &UiNode)> = Vec::new();

    for (idx, child) in children.iter().enumerate() {
        if let UiNode::Pinned {
            edge: PinnedEdge::Bottom,
            child: inner,
        } = child
        {
            if let Some(h) = bottom_pin_height(ui, inner) {
                pinned_bottom.push((idx, h, inner.as_ref()));
                continue;
            }
        }
        body_children.push((idx, child));
    }

    while let Some((idx, last)) = body_children.last().copied() {
        if let Some(h) = bottom_pin_height(ui, last) {
            pinned_bottom.push((idx, h, last));
            body_children.pop();
        } else {
            break;
        }
    }
    pinned_bottom.reverse();

    let action_idx = children.iter().position(is_action_bar_node);
    let footer_idx = children.iter().position(is_footer_shell_node);

    let Some(action_idx) = action_idx else {
        errors.push("missing action_bar in shell body".to_string());
        return errors;
    };
    let Some(footer_idx) = footer_idx else {
        errors.push("missing bottom footer in shell".to_string());
        return errors;
    };
    if action_idx > footer_idx {
        errors.push("action_bar appears after footer in shell order".to_string());
    }

    let total_footer_h: f32 = pinned_bottom.iter().map(|(_, h, _)| *h).sum();
    if total_footer_h <= 0.0 {
        errors.push("footer has no resolved height".to_string());
    }
    if total_footer_h > content_height + EPSILON {
        errors.push(format!(
            "footer would extend below viewport: footer {:.1}px > shell {:.1}px",
            total_footer_h, content_height
        ));
    }

    let body_h = content_height - total_footer_h;
    if body_h < -EPSILON {
        errors.push(format!(
            "body slot is negative: shell {:.1}px - footer {:.1}px = {:.1}px",
            content_height, total_footer_h, body_h
        ));
        return errors;
    }
    let body_h = body_h.max(0.0);

    let effective_gap = gap.max(0.0);
    let gap_total = effective_gap * body_children.len().saturating_sub(1) as f32;
    let mut fixed_total = gap_total;
    let mut grow_count = 0usize;
    let mut unresolved = Vec::new();

    for (idx, child) in &body_children {
        if vertical_grow_node(child) {
            grow_count += 1;
        } else if let Some(h) = vertical_fixed_height(ui, child) {
            fixed_total += h;
        } else {
            unresolved.push(*idx);
        }
    }

    if !unresolved.is_empty() {
        errors.push(format!(
            "body child height is unresolved at column index(es) {:?}",
            unresolved
        ));
    }
    if fixed_total > body_h + EPSILON {
        errors.push(format!(
            "body fixed content exceeds body slot: fixed {:.1}px > body {:.1}px",
            fixed_total, body_h
        ));
        if grow_count > 0 {
            errors.push(format!(
                "grow area is negative: body {:.1}px - fixed {:.1}px = {:.1}px",
                body_h,
                fixed_total,
                body_h - fixed_total
            ));
        }
    }

    let grow_h = if grow_count > 0 {
        ((body_h - fixed_total).max(0.0)) / grow_count as f32
    } else {
        0.0
    };
    let mut cursor = 0.0;
    let mut action_slot = None;

    for (body_pos, (idx, child)) in body_children.iter().enumerate() {
        if body_pos > 0 {
            cursor += effective_gap;
        }
        let h = if vertical_grow_node(child) {
            grow_h
        } else {
            vertical_fixed_height(ui, child).unwrap_or(0.0)
        };
        if *idx == action_idx {
            action_slot = Some((cursor, cursor + h));
        }
        cursor += h;
    }

    if let Some((action_top, action_bottom)) = action_slot {
        if action_top < -EPSILON {
            errors.push(format!(
                "action_bar starts above body slot at {action_top:.1}px"
            ));
        }
        if action_bottom > body_h + EPSILON {
            errors.push(format!(
                "action_bar overlaps footer: action bottom {:.1}px > footer top {:.1}px",
                action_bottom, body_h
            ));
        }
    } else {
        errors.push("action_bar is not in resolved body slot".to_string());
    }

    let footer_bottom = body_h + total_footer_h;
    if footer_bottom > content_height + EPSILON {
        errors.push(format!(
            "footer bottom exceeds viewport: footer bottom {:.1}px > shell {:.1}px",
            footer_bottom, content_height
        ));
    }

    errors
}

fn is_action_bar_node(node: &UiNode) -> bool {
    matches!(node, UiNode::ActionBar { .. })
}

fn is_footer_shell_node(node: &UiNode) -> bool {
    match node {
        UiNode::Pinned {
            edge: PinnedEdge::Bottom,
            child,
        } => matches!(
            child.as_ref(),
            UiNode::FooterKeys { .. } | UiNode::Footer { .. }
        ),
        UiNode::FooterKeys { .. } | UiNode::Footer { .. } => true,
        _ => false,
    }
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
/// Returns a `ComponentTreeResult` with interaction events and canvas dimensions.
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
) -> ComponentTreeResult {
    let mut canvas_width = 0.0f32;
    let mut canvas_height = 0.0f32;
    let mut hit_regions: Vec<(egui::Rect, String)> = Vec::new();
    let events = render_component_tree_inner(
        ui,
        node,
        colors,
        text_edit_buffers,
        focus_ctx,
        raw_caches,
        &mut canvas_width,
        &mut canvas_height,
        &mut hit_regions,
    );
    ComponentTreeResult {
        events,
        canvas_width,
        canvas_height,
        hit_regions,
    }
}

fn render_component_tree_inner(
    ui: &mut Ui,
    node: &UiNode,
    colors: &Colors,
    text_edit_buffers: &mut std::collections::HashMap<String, String>,
    focus_ctx: &mut TextEditFocusCtx,
    raw_caches: &mut RawNodeCaches<'_>,
    canvas_w: &mut f32,
    canvas_h: &mut f32,
    hit_regions: &mut Vec<(egui::Rect, String)>,
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
                        0.0,
                        colors,
                        text_edit_buffers,
                        focus_ctx,
                        raw_caches,
                        canvas_w,
                        canvas_h,
                        hit_regions,
                    ));
                });
        }

        UiNode::Scroll { child, horizontal } => {
            let size = ui.max_rect().size();
            let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
            let scroll = if *horizontal {
                egui::ScrollArea::both()
            } else {
                egui::ScrollArea::vertical()
            };
            ui.allocate_new_ui(egui::UiBuilder::new().max_rect(rect), |ui| {
                ui.set_clip_rect(rect);
                ui.set_min_width(rect.width());
                ui.set_max_width(rect.width());
                ui.set_min_height(rect.height());
                ui.set_max_height(rect.height());
                scroll
                    .max_width(rect.width())
                    .max_height(rect.height())
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        events.extend(render_component_tree_inner(
                            ui,
                            child,
                            colors,
                            text_edit_buffers,
                            focus_ctx,
                            raw_caches,
                            canvas_w,
                            canvas_h,
                            hit_regions,
                        ));
                    });
            });
        }

        UiNode::Sized {
            width,
            height,
            child,
        } => {
            let w = width.unwrap_or_else(|| ui.available_width()).max(0.0);
            let h = height.unwrap_or_else(|| ui.available_height()).max(0.0);
            log::trace!(
                "render Sized: w={w} h={h} avail_w={} avail_h={}",
                ui.available_width(),
                ui.available_height()
            );
            ui.allocate_ui(egui::vec2(w, h), |ui| {
                ui.set_min_width(w);
                ui.set_max_width(w);
                ui.set_min_height(h);
                ui.set_max_height(h);
                events.extend(render_component_tree_inner(
                    ui,
                    child,
                    colors,
                    text_edit_buffers,
                    focus_ctx,
                    raw_caches,
                    canvas_w,
                    canvas_h,
                    hit_regions,
                ));
            });
        }

        UiNode::Layer { children } => {
            // V1: sequential rendering (true Z-stacking is a future improvement).
            for child in children {
                events.extend(render_component_tree_inner(
                    ui,
                    child,
                    colors,
                    text_edit_buffers,
                    focus_ctx,
                    raw_caches,
                    canvas_w,
                    canvas_h,
                    hit_regions,
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
            let chrome = AppChrome::new(colors);
            chrome.text_label(
                ui,
                text,
                if *size > 0.0 { *size } else { style::TEXT_BODY },
                chrome.text_color(color, ""),
                *bold,
                *monospace,
                false,
            );
        }

        UiNode::Markdown {
            text,
            base_size,
            color,
            padding,
        } => {
            let size = egui::vec2(ui.available_width(), ui.available_height());
            let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
            let content = rect.shrink((*padding).max(0.0));
            if content.is_positive() {
                let text_color = if color.is_empty() {
                    colors.text_primary
                } else {
                    parse_color(color).unwrap_or(colors.text_primary)
                };
                let mut child = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(content)
                        .layout(egui::Layout::top_down(egui::Align::LEFT)),
                );
                child.set_clip_rect(content);
                crate::ui::markdown::show(
                    &mut child,
                    raw_caches.commonmark_cache,
                    colors,
                    text,
                    text_color,
                    if *base_size > 0.0 {
                        *base_size
                    } else {
                        style::TEXT_BODY
                    },
                );
            }
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
                let child_evts = render_component_tree_inner(
                    ui,
                    child,
                    colors,
                    text_edit_buffers,
                    focus_ctx,
                    raw_caches,
                    canvas_w,
                    canvas_h,
                    hit_regions,
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
            events.extend(render_component_tree_inner(
                ui,
                child,
                colors,
                text_edit_buffers,
                focus_ctx,
                raw_caches,
                canvas_w,
                canvas_h,
                hit_regions,
            ));
        }

        UiNode::Raw { command } => {
            // Delegate to the existing flat renderer for a single draw command,
            // threading the pane's persistent caches so markdown/image/list
            // state survives across frames instead of being rebuilt per node.
            let raw_h = match command.as_ref() {
                crate::app_protocol::RenderCommand::ListView { h, .. } if *h > 0.0 => *h,
                crate::app_protocol::RenderCommand::ListView { .. } => ui.available_height(),
                _ => ui.available_height(),
            };
            let (pane_rect, _) = ui.allocate_exact_size(
                egui::vec2(ui.available_width(), raw_h.max(0.0)),
                egui::Sense::hover(),
            );
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
                &mut 0.0f32,
                &mut 0.0f32,
                &mut Vec::new(),
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

        UiNode::Canvas {
            commands,
            width,
            height,
            grow,
        } => {
            let node_canvas_w = if *grow {
                ui.available_width()
            } else {
                (*width).min(ui.available_width())
            }
            .max(0.0);
            let node_canvas_h = if *grow {
                ui.available_height().max(*height)
            } else {
                *height
            }
            .max(0.0);
            // Track the largest canvas — the primary content canvas wins over
            // small utility canvases (e.g. numpads) that appear later in the tree.
            if node_canvas_w > *canvas_w {
                *canvas_w = node_canvas_w;
            }
            if node_canvas_h > *canvas_h {
                *canvas_h = node_canvas_h;
            }
            let (rect, _) = ui.allocate_exact_size(
                egui::vec2(node_canvas_w, node_canvas_h),
                egui::Sense::click(),
            );
            let mut raw_events: Vec<crate::app_protocol::PlexiEvent> = Vec::new();
            let mut raw_focus_ctx = TextEditFocusCtx::new();
            crate::process_app::render::render_draw_commands(
                ui,
                rect,
                commands,
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
                &mut 0.0f32,
                &mut 0.0f32,
                hit_regions,
            );
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
            style: button_style,
            ..
        } => {
            let btn_w = button::chrome_button_intrinsic_width(ui, label);
            let (rect, _) =
                ui.allocate_exact_size(egui::vec2(btn_w, button_height()), egui::Sense::hover());
            if let Some(evt) =
                render_button_at(ui, rect, node_id, label, *disabled, button_style, colors)
            {
                events.push(evt);
            }
        }

        UiNode::ActionBar { actions } => {
            let (rect, _) = ui.allocate_exact_size(
                egui::vec2(ui.available_width(), action_bar_height()),
                egui::Sense::hover(),
            );
            AppChrome::new(colors).paint_action_bar_background(ui, rect);
            let mut x = rect.min.x;
            let y = rect.min.y + (action_bar_height() - button_height()) / 2.0;

            for action in actions {
                let UiNode::Button {
                    node_id,
                    label,
                    disabled,
                    style: button_style,
                    ..
                } = action
                else {
                    log::warn!(
                        "render_components: ActionBar child is not a Button; skipping child"
                    );
                    continue;
                };

                let w = button::chrome_button_intrinsic_width(ui, label);
                if x + w > rect.max.x {
                    log::debug!(
                        "render_components: ActionBar clipped remaining actions at width={}",
                        rect.width()
                    );
                    break;
                }
                let button_rect =
                    egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(w, button_height()));
                if let Some(evt) = render_button_at(
                    ui,
                    button_rect,
                    node_id,
                    label,
                    *disabled,
                    button_style,
                    colors,
                ) {
                    events.push(evt);
                }
                x += w + style::SPACE_SM;
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

            let chrome_response = AppChrome::new(colors).text_edit(
                ui,
                widget_id,
                placeholder,
                buffer,
                *multiline,
                *max_length,
            );
            let response = chrome_response.response;

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
            if response.clicked() || chrome_response.frame_clicked {
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
            AppChrome::new(colors).paint_badge(ui, label, fill, fg);
        }

        UiNode::Dot { color, size, .. } => {
            AppChrome::new(colors).paint_dot(ui, color, *size);
        }

        // ── L1 layout components ────────────────────────────────────────
        UiNode::Column {
            children,
            gap,
            padding_top,
            padding,
        } => {
            log::trace!(
                "render_components: Column {} children gap={gap} padding_top={padding_top} padding={padding}",
                children.len()
            );
            let content_padding = semantic_shell_padding(children, *padding);
            if is_semantic_app_shell(children) {
                if !APP_CHROME_INFO_LOGGED.swap(true, Ordering::Relaxed) {
                    log::info!(
                        "render_components: SDK semantic app chrome routed through host AppChrome"
                    );
                }
                if vertical_stack_needs_full_height(children) {
                    let h = ui.available_height();
                    ui.set_min_height(h);
                }
                events.extend(render_stack(
                    ui,
                    &StackDirection::Vertical,
                    children,
                    *gap,
                    content_padding,
                    colors,
                    text_edit_buffers,
                    focus_ctx,
                    raw_caches,
                    canvas_w,
                    canvas_h,
                    hit_regions,
                ));
                return events;
            }
            // Skip top padding when the first child is AppBar (it's full-bleed chrome)
            let effective_top = match children.first() {
                Some(UiNode::AppBar { .. }) => 0,
                _ => *padding_top as i8,
            };
            egui::Frame::new()
                .inner_margin(egui::Margin {
                    left: content_padding as i8,
                    right: content_padding as i8,
                    top: effective_top,
                    bottom: 0,
                })
                .show(ui, |ui| {
                    // Only fill the remaining height when the stack contains a child
                    // that needs partitioning. Static nested Columns should stay
                    // content-sized; otherwise they push later siblings into footers.
                    if vertical_stack_needs_full_height(children) {
                        let h = ui.available_height();
                        ui.set_min_height(h);
                    }
                    events.extend(render_stack(
                        ui,
                        &StackDirection::Vertical,
                        children,
                        *gap,
                        0.0,
                        colors,
                        text_edit_buffers,
                        focus_ctx,
                        raw_caches,
                        canvas_w,
                        canvas_h,
                        hit_regions,
                    ));
                });
        }

        UiNode::AppBar {
            title, subtitle, ..
        } => {
            AppChrome::new(colors).paint_app_bar(ui, title, subtitle);
        }

        UiNode::FooterKeys {
            entries, divider, ..
        } => {
            AppChrome::new(colors).paint_footer_keys(ui, entries, *divider);
        }

        UiNode::Footer { text, color, .. } => {
            AppChrome::new(colors).paint_footer(ui, text, color);
        }

        UiNode::Section { title, .. } => {
            AppChrome::new(colors).paint_section(ui, title);
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
            let chrome = AppChrome::new(colors);
            let font_size = if *size > 0.0 { *size } else { style::TEXT_BODY };
            if *max_lines > 0 {
                let line_h = font_size + 4.0;
                let max_h = *max_lines as f32 * line_h;
                ui.scope(|ui| {
                    ui.set_max_height(max_h);
                    chrome.text_label(
                        ui,
                        text,
                        font_size,
                        chrome.text_color(color, tone),
                        *bold,
                        *monospace,
                        true,
                    );
                });
            } else {
                chrome.text_label(
                    ui,
                    text,
                    font_size,
                    chrome.text_color(color, tone),
                    *bold,
                    *monospace,
                    true,
                );
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
            AppChrome::new(colors).paint_divider(ui, div_color);
        }

        UiNode::Card {
            children, padding, ..
        } => {
            AppChrome::new(colors).card(ui, *padding, |ui| {
                for (idx, child) in children.iter().enumerate() {
                    if idx > 0 {
                        ui.add_space(app_chrome::CARD_CHILD_GAP);
                    }
                    events.extend(render_component_tree_inner(
                        ui,
                        child,
                        colors,
                        text_edit_buffers,
                        focus_ctx,
                        raw_caches,
                        canvas_w,
                        canvas_h,
                        hit_regions,
                    ));
                }
            });
        }

        UiNode::SelectList {
            items,
            selected_idx,
            ..
        } => {
            AppChrome::new(colors).select_list(ui, items, *selected_idx);
        }
    }

    events
}

// ── Internal helpers ──────────────────────────────────────────────────────────

fn button_height() -> f32 {
    app_chrome::button_height()
}

fn action_bar_height() -> f32 {
    app_chrome::action_bar_height()
}

fn render_button_at(
    ui: &mut Ui,
    rect: egui::Rect,
    node_id: &str,
    label: &str,
    disabled: bool,
    button_style: &str,
    colors: &Colors,
) -> Option<ComponentEventPayload> {
    // Use raw PointerState because button_down/button_pressed read pointer
    // events directly and are not affected by the pane-wide click-and-drag
    // widget registered later around the whole pane.
    let pointer_pos = ui.input(|i| i.pointer.interact_pos().or_else(|| i.pointer.hover_pos()));
    let is_hovered = !disabled && pointer_pos.map_or(false, |p| rect.contains(p));
    let is_down = is_hovered && ui.input(|i| i.pointer.button_down(egui::PointerButton::Primary));
    let is_just_pressed =
        is_hovered && ui.input(|i| i.pointer.button_pressed(egui::PointerButton::Primary));
    button::paint_chrome_button_at(
        ui.painter(),
        rect,
        label,
        app_button_kind(button_style),
        button::ChromeButtonState {
            disabled,
            hovered: is_hovered,
            down: is_down,
        },
        colors,
    );
    if is_hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    if is_just_pressed {
        log::info!("render_components: Button press node_id={node_id}");
        return Some(ComponentEventPayload {
            node_id: node_id.to_owned(),
            event_type: "click".into(),
            payload: None,
        });
    }
    None
}

fn app_button_kind(button_style: &str) -> button::ButtonKind {
    app_chrome::button_kind(button_style)
}

/// Returns the known fixed height of a node that can be bottom-pinned, or `None`
/// if the height cannot be determined without rendering.
fn bottom_pin_height(ui: &egui::Ui, node: &UiNode) -> Option<f32> {
    match node {
        UiNode::FooterKeys { divider, .. } => Some(app_chrome::footer_keys_height(ui, *divider)),
        UiNode::Footer { .. } => Some(app_chrome::footer_height()),
        _ => None,
    }
}

fn vertical_grow_node(node: &UiNode) -> bool {
    match node {
        UiNode::Spacer { grow, .. } => *grow,
        UiNode::Scroll { .. } => true,
        UiNode::SelectList { .. } => true,
        UiNode::Canvas { grow, .. } => *grow,
        UiNode::Raw { command } => matches!(
            command.as_ref(),
            crate::app_protocol::RenderCommand::ListView { .. }
        ),
        _ => false,
    }
}

fn semantic_shell_padding(children: &[UiNode], requested_padding: f32) -> f32 {
    if is_semantic_app_shell(children) {
        requested_padding.max(style::SPACE_MD)
    } else {
        requested_padding
    }
}

fn is_semantic_app_shell(children: &[UiNode]) -> bool {
    matches!(children.first(), Some(UiNode::AppBar { .. }))
        && children
            .iter()
            .any(|child| matches!(child, UiNode::ActionBar { .. }))
        && children.iter().any(|child| {
            matches!(
                child,
                UiNode::Pinned {
                    edge: PinnedEdge::Bottom,
                    child
                } if matches!(child.as_ref(), UiNode::FooterKeys { .. })
            ) || matches!(child, UiNode::FooterKeys { .. })
        })
}

fn vertical_stack_needs_full_height(children: &[UiNode]) -> bool {
    children.iter().any(|child| {
        vertical_grow_node(child)
            || matches!(
                child,
                UiNode::Pinned {
                    edge: PinnedEdge::Bottom,
                    ..
                } | UiNode::FooterKeys { .. }
                    | UiNode::Footer { .. }
            )
    })
}

fn vertical_fixed_height(ui: &egui::Ui, node: &UiNode) -> Option<f32> {
    match node {
        UiNode::Stack {
            direction,
            children,
            padding,
            ..
        } if *direction == StackDirection::Horizontal => {
            let mut max_h: f32 = 0.0;
            for child in children {
                let h = vertical_fixed_height(ui, child)?;
                max_h = max_h.max(h);
            }
            Some(max_h + padding.top + padding.bottom)
        }
        UiNode::AppBar { subtitle, .. } => Some(app_chrome::app_bar_height(subtitle)),
        UiNode::Button { .. } => Some(button_height()),
        UiNode::ActionBar { .. } => Some(action_bar_height()),
        UiNode::TextEdit { multiline, .. } => Some(app_chrome::text_edit_height(*multiline)),
        UiNode::Spacer { size, grow } => {
            if *grow {
                None
            } else {
                Some(if *size > 0.0 { *size } else { style::SPACE_MD })
            }
        }
        UiNode::FooterKeys { .. } | UiNode::Footer { .. } => bottom_pin_height(ui, node),
        UiNode::Divider { .. } => Some(1.0),
        UiNode::Section { .. } => {
            Some(style::SPACE_SM + style::TEXT_HINT + style::SPACE_XS + 1.0 + style::SPACE_XS)
        }
        UiNode::Text { size, .. } => Some(if *size > 0.0 {
            *size + 4.0
        } else {
            style::TEXT_BODY + 4.0
        }),
        UiNode::Badge { .. } => Some(style::TEXT_CAPTION + style::BADGE_PAD_V * 2.0 + 2.0),
        UiNode::Card { children, padding } => {
            let child_gap = app_chrome::CARD_CHILD_GAP * children.len().saturating_sub(1) as f32;
            let mut total = app_chrome::card_padding(*padding) * 2.0 + child_gap;
            for child in children {
                total += vertical_fixed_height(ui, child)?;
            }
            Some(total)
        }
        UiNode::Dot { size, .. } => Some(if *size > 0.0 { *size } else { 8.0 }),
        UiNode::Canvas { height, grow, .. } => {
            if *grow {
                None
            } else {
                Some(*height)
            }
        }
        UiNode::Sized { height, .. } => height.map(|h| h.max(0.0)),
        _ => None,
    }
}

/// True for nodes that expand to fill remaining *width* in a horizontal Stack.
fn horizontal_grow_node(node: &UiNode) -> bool {
    match node {
        UiNode::Canvas { grow, .. } => *grow,
        UiNode::Spacer { grow, .. } => *grow,
        _ => false,
    }
}

/// Fixed width (in px) for a node inside a horizontal Stack, or `None` when the
/// node has no intrinsic width and should render inline. Grow nodes return
/// `None` here — they are partitioned by [`horizontal_grow_node`].
fn horizontal_fixed_width(node: &UiNode) -> Option<f32> {
    match node {
        UiNode::Sized { width, .. } => width.map(|w| w.max(0.0)),
        UiNode::Spacer { size, grow } => {
            if *grow {
                None
            } else {
                Some(if *size > 0.0 { *size } else { style::SPACE_MD })
            }
        }
        UiNode::Canvas { width, grow, .. } => {
            if *grow {
                None
            } else {
                Some((*width).max(0.0))
            }
        }
        _ => None,
    }
}

fn render_horizontal_children(
    ui: &mut Ui,
    children: &[UiNode],
    gap: f32,
    panel_h: f32,
    colors: &Colors,
    text_edit_buffers: &mut std::collections::HashMap<String, String>,
    focus_ctx: &mut TextEditFocusCtx,
    raw_caches: &mut RawNodeCaches<'_>,
    canvas_w: &mut f32,
    canvas_h: &mut f32,
    hit_regions: &mut Vec<(egui::Rect, String)>,
) -> Vec<ComponentEventPayload> {
    let mut events = Vec::new();
    if children.is_empty() {
        return events;
    }

    let available_w = ui.available_width().max(0.0);
    let gap_total = gap * children.len().saturating_sub(1) as f32;
    let mut fixed_total = 0.0f32;
    let mut grow_count = 0usize;

    for child in children {
        if horizontal_grow_node(child) {
            grow_count += 1;
        } else if let Some(w) = horizontal_fixed_width(child) {
            fixed_total += w;
        }
    }

    let grow_w = if grow_count > 0 {
        ((available_w - fixed_total - gap_total).max(0.0)) / grow_count as f32
    } else {
        0.0
    };

    for (i, child) in children.iter().enumerate() {
        if i > 0 && gap > 0.0 {
            ui.add_space(gap);
        }

        let allocated_w = if horizontal_grow_node(child) {
            Some(grow_w)
        } else {
            horizontal_fixed_width(child)
        };

        if let Some(w) = allocated_w {
            ui.allocate_ui(egui::vec2(w.max(0.0), panel_h), |ui| {
                ui.set_min_width(w.max(0.0));
                ui.set_max_width(w.max(0.0));
                events.extend(render_component_tree_inner(
                    ui,
                    child,
                    colors,
                    text_edit_buffers,
                    focus_ctx,
                    raw_caches,
                    canvas_w,
                    canvas_h,
                    hit_regions,
                ));
            });
        } else {
            events.extend(render_component_tree_inner(
                ui,
                child,
                colors,
                text_edit_buffers,
                focus_ctx,
                raw_caches,
                canvas_w,
                canvas_h,
                hit_regions,
            ));
        }
    }

    events
}

fn render_vertical_children(
    ui: &mut Ui,
    children: &[&UiNode],
    gap: f32,
    content_inset: f32,
    available_h_override: Option<f32>,
    colors: &Colors,
    text_edit_buffers: &mut std::collections::HashMap<String, String>,
    focus_ctx: &mut TextEditFocusCtx,
    raw_caches: &mut RawNodeCaches<'_>,
    canvas_w: &mut f32,
    canvas_h: &mut f32,
    hit_regions: &mut Vec<(egui::Rect, String)>,
) -> Vec<ComponentEventPayload> {
    let mut events = Vec::new();
    if children.is_empty() {
        return events;
    }

    ui.spacing_mut().item_spacing.y = 0.0;
    let available_h = available_h_override
        .unwrap_or_else(|| ui.available_height().min(ui.max_rect().height()))
        .max(0.0);
    let gap_total = gap * children.len().saturating_sub(1) as f32;
    let mut fixed_total = gap_total;
    let mut grow_count = 0usize;

    for child in children {
        if vertical_grow_node(child) {
            grow_count += 1;
        } else if let Some(h) = vertical_fixed_height(ui, child) {
            fixed_total += h;
        }
    }

    let grow_h = if grow_count > 0 {
        ((available_h - fixed_total).max(0.0)) / grow_count as f32
    } else {
        0.0
    };

    for (i, child) in children.iter().enumerate() {
        if i > 0 && gap > 0.0 {
            ui.add_space(gap);
        }

        let allocated_h = if vertical_grow_node(child) {
            Some(grow_h)
        } else {
            vertical_fixed_height(ui, child)
        };

        if let Some(h) = allocated_h {
            let (slot_rect, _) = ui.allocate_exact_size(
                egui::vec2(ui.available_width(), h.max(0.0)),
                egui::Sense::hover(),
            );
            let render_rect = shell_child_rect(slot_rect, child, content_inset);
            ui.allocate_new_ui(egui::UiBuilder::new().max_rect(render_rect), |ui| {
                ui.set_clip_rect(render_rect);
                ui.set_min_height(render_rect.height());
                ui.set_max_height(render_rect.height());
                events.extend(render_component_tree_inner(
                    ui,
                    child,
                    colors,
                    text_edit_buffers,
                    focus_ctx,
                    raw_caches,
                    canvas_w,
                    canvas_h,
                    hit_regions,
                ));
            });
        } else if content_inset > 0.0 && node_uses_shell_content_inset(child) {
            egui::Frame::new()
                .inner_margin(egui::Margin {
                    left: content_inset as i8,
                    right: content_inset as i8,
                    top: 0,
                    bottom: 0,
                })
                .show(ui, |ui| {
                    events.extend(render_component_tree_inner(
                        ui,
                        child,
                        colors,
                        text_edit_buffers,
                        focus_ctx,
                        raw_caches,
                        canvas_w,
                        canvas_h,
                        hit_regions,
                    ));
                });
        } else {
            events.extend(render_component_tree_inner(
                ui,
                child,
                colors,
                text_edit_buffers,
                focus_ctx,
                raw_caches,
                canvas_w,
                canvas_h,
                hit_regions,
            ));
        }
    }

    events
}

fn shell_child_rect(slot_rect: egui::Rect, child: &UiNode, content_inset: f32) -> egui::Rect {
    if content_inset <= 0.0 || !node_uses_shell_content_inset(child) {
        return slot_rect;
    }
    let inset = content_inset.min(slot_rect.width() / 2.0);
    egui::Rect::from_min_max(
        egui::pos2(slot_rect.min.x + inset, slot_rect.min.y),
        egui::pos2(slot_rect.max.x - inset, slot_rect.max.y),
    )
}

fn node_uses_shell_content_inset(node: &UiNode) -> bool {
    !matches!(
        node,
        UiNode::AppBar { .. }
            | UiNode::FooterKeys { .. }
            | UiNode::Footer { .. }
            | UiNode::Pinned {
                edge: PinnedEdge::Bottom,
                ..
            }
    )
}

fn render_stack(
    ui: &mut Ui,
    direction: &StackDirection,
    children: &[UiNode],
    gap: f32,
    content_inset: f32,
    colors: &Colors,
    text_edit_buffers: &mut std::collections::HashMap<String, String>,
    focus_ctx: &mut TextEditFocusCtx,
    raw_caches: &mut RawNodeCaches<'_>,
    canvas_w: &mut f32,
    canvas_h: &mut f32,
    hit_regions: &mut Vec<(egui::Rect, String)>,
) -> Vec<ComponentEventPayload> {
    let mut events = Vec::new();
    match direction {
        StackDirection::Horizontal => {
            let panel_h = ui.available_height();
            log::trace!("render_stack: horizontal panel_h={panel_h}");
            ui.horizontal(|ui| {
                events.extend(render_horizontal_children(
                    ui,
                    children,
                    gap,
                    panel_h,
                    colors,
                    text_edit_buffers,
                    focus_ctx,
                    raw_caches,
                    canvas_w,
                    canvas_h,
                    hit_regions,
                ));
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
            //      (kept for hand-authored protocol trees; SDK footers use Pinned)
            let mut pinned_bottom: Vec<(f32, &UiNode)> = Vec::new();
            let mut body_children: Vec<&UiNode> = Vec::new();

            for child in children {
                if let UiNode::Pinned {
                    edge: PinnedEdge::Bottom,
                    child: inner,
                } = child
                {
                    if let Some(h) = bottom_pin_height(ui, inner) {
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
                if let Some(h) = bottom_pin_height(ui, last) {
                    pinned_bottom.push((h, body_children.pop().unwrap()));
                } else {
                    break;
                }
            }
            // Reverse so they render in original order (we popped from the end)
            pinned_bottom.reverse();

            if !pinned_bottom.is_empty() {
                let total_pinned_h: f32 = pinned_bottom.iter().map(|(h, _)| h).sum();
                let stack_size = egui::vec2(ui.available_width(), ui.available_height());
                let (stack_rect, _) = ui.allocate_exact_size(stack_size, egui::Sense::hover());
                let body_h = (stack_rect.height() - total_pinned_h).max(0.0);
                let body_rect = egui::Rect::from_min_size(
                    stack_rect.min,
                    egui::vec2(stack_rect.width(), body_h),
                );

                ui.allocate_new_ui(egui::UiBuilder::new().max_rect(body_rect), |ui| {
                    ui.set_clip_rect(body_rect);
                    ui.set_min_height(body_h);
                    ui.set_max_height(body_h);
                    events.extend(render_vertical_children(
                        ui,
                        &body_children,
                        gap,
                        content_inset,
                        Some(body_h),
                        colors,
                        text_edit_buffers,
                        focus_ctx,
                        raw_caches,
                        canvas_w,
                        canvas_h,
                        hit_regions,
                    ));
                });

                let mut footer_y = stack_rect.max.y - total_pinned_h;
                for (footer_h, inner) in &pinned_bottom {
                    let footer_rect = egui::Rect::from_min_size(
                        egui::pos2(stack_rect.min.x, footer_y),
                        egui::vec2(stack_rect.width(), *footer_h),
                    );
                    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(footer_rect), |ui| {
                        ui.set_clip_rect(footer_rect);
                        ui.set_min_height(*footer_h);
                        ui.set_max_height(*footer_h);
                        events.extend(render_component_tree_inner(
                            ui,
                            inner,
                            colors,
                            text_edit_buffers,
                            focus_ctx,
                            raw_caches,
                            canvas_w,
                            canvas_h,
                            hit_regions,
                        ));
                    });
                    footer_y += *footer_h;
                }
                log::trace!(
                    "render_components: render_stack vertical pinned body_h={body_h:.0} footer_h={total_pinned_h:.0}"
                );
            } else {
                log::trace!(
                    "render_stack: vertical no-pin {} children avail_h={}",
                    children.len(),
                    ui.available_height()
                );
                let child_refs: Vec<&UiNode> = children.iter().collect();
                ui.vertical(|ui| {
                    events.extend(render_vertical_children(
                        ui,
                        &child_refs,
                        gap,
                        content_inset,
                        None,
                        colors,
                        text_edit_buffers,
                        focus_ctx,
                        raw_caches,
                        canvas_w,
                        canvas_h,
                        hit_regions,
                    ));
                });
            }
        }
    }
    events
}

use crate::process_app::render::parse_color;

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod render_component_tree_tests {
    use super::*;
    use crate::app_protocol::{FooterKeyEntry, StackDirection, UiNode, UiPadding};

    fn scaffold_shell_tree() -> UiNode {
        UiNode::Column {
            children: vec![
                UiNode::AppBar {
                    title: "Counter".into(),
                    subtitle: String::new(),
                },
                UiNode::Text {
                    text: "3".into(),
                    size: 0.0,
                    color: String::new(),
                    bold: false,
                    monospace: false,
                },
                UiNode::Spacer {
                    size: 0.0,
                    grow: true,
                },
                UiNode::ActionBar {
                    actions: vec![UiNode::Button {
                        node_id: "counter-increment".into(),
                        label: "Increment".into(),
                        disabled: false,
                        style: "primary".into(),
                    }],
                },
                UiNode::Pinned {
                    edge: PinnedEdge::Bottom,
                    child: Box::new(UiNode::FooterKeys {
                        entries: vec![
                            FooterKeyEntry {
                                keys: vec!["i".into()],
                                description: "increment".into(),
                            },
                            FooterKeyEntry {
                                keys: vec!["r".into()],
                                description: "reset".into(),
                            },
                        ],
                        divider: true,
                    }),
                },
            ],
            gap: 8.0,
            padding_top: 0.0,
            padding: style::SPACE_MD,
        }
    }

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
            style: "primary".into(),
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

    #[test]
    fn action_bar_node_constructable() {
        let node = UiNode::ActionBar {
            actions: vec![UiNode::Button {
                node_id: "save".into(),
                label: "Save".into(),
                disabled: false,
                style: "primary".into(),
            }],
        };
        if let UiNode::ActionBar { actions } = &node {
            assert_eq!(actions.len(), 1);
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
        use crate::render::app_chrome;
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

        let expected_footer = style::SPACE_MD + 1.0 + style::SPACE_MD + style::TEXT_CAPTION + 5.0;

        let ctx = egui::Context::default();
        ctx.begin_pass(egui::RawInput::default());
        egui::CentralPanel::default().show(&ctx, |ui| {
            let crh = app_chrome::chip_row_height(ui);
            let row_h = crh + 4.0;
            let expected_with_div = 1.0 + style::SPACE_SM + row_h + style::SPACE_SM;
            let expected_no_div = style::SPACE_SM + row_h + style::SPACE_SM;

            assert_eq!(bottom_pin_height(ui, &fk_with_div), Some(expected_with_div));
            assert_eq!(bottom_pin_height(ui, &fk_no_div), Some(expected_no_div));
            assert_eq!(bottom_pin_height(ui, &footer), Some(expected_footer));
            assert_eq!(bottom_pin_height(ui, &label), None);
        });
        let _ = ctx.end_pass();
    }

    #[test]
    fn footer_keys_content_rect_centers_horizontally_and_vertically() {
        let full_rect = egui::Rect::from_min_size(egui::pos2(0.0, 100.0), egui::vec2(320.0, 40.0));
        let content =
            crate::render::app_chrome::footer_keys_content_rect(full_rect, 150.0, 18.0, true);

        assert_eq!(content.center().x, full_rect.center().x);
        let top_pad = content.min.y - (full_rect.min.y + 1.0);
        let bottom_pad = full_rect.max.y - content.max.y;
        assert_eq!(top_pad, bottom_pad);
    }

    /// A horizontal stack of fixed-height buttons reserves one button row in vertical layout.
    #[test]
    fn horizontal_button_stack_has_fixed_height() {
        use super::vertical_fixed_height;

        let stack = UiNode::Stack {
            direction: StackDirection::Horizontal,
            children: vec![
                UiNode::Button {
                    node_id: "save".into(),
                    label: "Save".into(),
                    disabled: false,
                    style: "primary".into(),
                },
                UiNode::Button {
                    node_id: "cancel".into(),
                    label: "Cancel".into(),
                    disabled: false,
                    style: "ghost".into(),
                },
            ],
            gap: 8.0,
            padding: crate::app_protocol::UiPadding {
                top: 3.0,
                bottom: 5.0,
                ..Default::default()
            },
        };

        let ctx = egui::Context::default();
        ctx.begin_pass(egui::RawInput::default());
        egui::CentralPanel::default().show(&ctx, |ui| {
            assert_eq!(vertical_fixed_height(ui, &stack), Some(40.0));
        });
        let _ = ctx.end_pass();
    }

    #[test]
    fn action_bar_has_fixed_height() {
        use super::{action_bar_height, vertical_fixed_height};

        let node = UiNode::ActionBar {
            actions: vec![UiNode::Button {
                node_id: "save".into(),
                label: "Save".into(),
                disabled: false,
                style: "primary".into(),
            }],
        };

        let ctx = egui::Context::default();
        ctx.begin_pass(egui::RawInput::default());
        egui::CentralPanel::default().show(&ctx, |ui| {
            assert_eq!(vertical_fixed_height(ui, &node), Some(action_bar_height()));
        });
        let _ = ctx.end_pass();
    }

    #[test]
    fn card_with_known_semantic_children_has_fixed_height() {
        use super::vertical_fixed_height;
        use crate::render::app_chrome;

        let node = UiNode::Card {
            padding: style::SPACE_MD,
            children: vec![
                UiNode::Text {
                    text: "3".into(),
                    size: 24.0,
                    color: String::new(),
                    bold: true,
                    monospace: false,
                },
                UiNode::TextEdit {
                    node_id: "name".into(),
                    placeholder: "Type".into(),
                    value: String::new(),
                    multiline: false,
                    max_length: 0,
                },
            ],
        };

        let ctx = egui::Context::default();
        ctx.begin_pass(egui::RawInput::default());
        egui::CentralPanel::default().show(&ctx, |ui| {
            assert_eq!(
                vertical_fixed_height(ui, &node),
                Some(
                    style::SPACE_MD * 2.0
                        + 24.0
                        + 4.0
                        + app_chrome::CARD_CHILD_GAP
                        + app_chrome::text_edit_height(false)
                )
            );
        });
        let _ = ctx.end_pass();
    }

    /// Static nested Columns stay content-sized; pinned/grow stacks fill the available height.
    #[test]
    fn vertical_stack_fill_height_only_when_needed() {
        use super::vertical_stack_needs_full_height;

        let static_children = vec![UiNode::Text {
            text: "body".into(),
            size: 0.0,
            color: String::new(),
            bold: false,
            monospace: false,
        }];
        assert!(!vertical_stack_needs_full_height(&static_children));

        let grow_children = vec![UiNode::SelectList {
            items: vec![],
            selected_idx: 0,
        }];
        assert!(vertical_stack_needs_full_height(&grow_children));

        let footer_children = vec![UiNode::Pinned {
            edge: PinnedEdge::Bottom,
            child: Box::new(UiNode::FooterKeys {
                entries: vec![],
                divider: true,
            }),
        }];
        assert!(vertical_stack_needs_full_height(&footer_children));
    }

    #[test]
    fn semantic_app_shell_has_minimum_content_padding() {
        let UiNode::Column { children, .. } = scaffold_shell_tree() else {
            panic!("scaffold_shell_tree should return column");
        };

        assert_eq!(semantic_shell_padding(&children, 0.0), style::SPACE_MD);
        assert_eq!(semantic_shell_padding(&children, 4.0), style::SPACE_MD);
        assert_eq!(semantic_shell_padding(&children, 24.0), 24.0);
    }

    #[test]
    fn plain_column_can_still_be_full_bleed() {
        let children = vec![UiNode::Text {
            text: "body".into(),
            size: 0.0,
            color: String::new(),
            bold: false,
            monospace: false,
        }];

        assert_eq!(semantic_shell_padding(&children, 0.0), 0.0);
    }

    #[test]
    fn shell_layout_accepts_fresh_scaffold_small_and_normal_viewports() {
        let tree = scaffold_shell_tree();

        assert!(
            validate_shell_layout(&tree, 320.0, 240.0).is_empty(),
            "{:?}",
            validate_shell_layout(&tree, 320.0, 240.0)
        );
        assert!(
            validate_shell_layout(&tree, 800.0, 600.0).is_empty(),
            "{:?}",
            validate_shell_layout(&tree, 800.0, 600.0)
        );
    }

    #[test]
    fn shell_layout_rejects_footer_below_viewport() {
        let tree = scaffold_shell_tree();
        let errors = validate_shell_layout(&tree, 320.0, 20.0);

        assert!(
            errors
                .iter()
                .any(|error| error.contains("footer would extend below viewport")),
            "{errors:?}"
        );
    }

    #[test]
    fn shell_layout_rejects_action_footer_overlap() {
        let tree = scaffold_shell_tree();
        let errors = validate_shell_layout(&tree, 320.0, 110.0);

        assert!(
            errors
                .iter()
                .any(|error| error.contains("action_bar overlaps footer")),
            "{errors:?}"
        );
    }

    #[test]
    fn shell_layout_rejects_negative_grow_area() {
        let tree = scaffold_shell_tree();
        let errors = validate_shell_layout(&tree, 320.0, 110.0);

        assert!(
            errors
                .iter()
                .any(|error| error.contains("grow area is negative")),
            "{errors:?}"
        );
    }

    #[test]
    fn shell_layout_rejects_action_bar_after_footer() {
        let mut tree = scaffold_shell_tree();
        if let UiNode::Column { children, .. } = &mut tree {
            children.swap(3, 4);
        }

        let errors = validate_shell_layout(&tree, 320.0, 240.0);

        assert!(
            errors
                .iter()
                .any(|error| error.contains("action_bar appears after footer")),
            "{errors:?}"
        );
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

    /// Serde round-trip for `UiNode::Sized` with a null (inherited) height.
    #[test]
    fn sized_serde_roundtrip() {
        let node = UiNode::Sized {
            width: Some(160.0),
            height: None,
            child: Box::new(UiNode::Text {
                text: "side".into(),
                size: 0.0,
                color: String::new(),
                bold: false,
                monospace: false,
            }),
        };
        let json = serde_json::to_string(&node).unwrap();
        assert!(json.contains("\"type\":\"sized\""), "json={json}");
        let parsed: UiNode = serde_json::from_str(&json).unwrap();
        assert_eq!(node, parsed);
    }

    /// Horizontal width partitioning: a `Sized{width:160}` sidebar next to a
    /// growing `Canvas` gives the Canvas `total_width - 160 - gap`.
    #[test]
    fn horizontal_partition_sized_plus_grow_canvas() {
        use super::{horizontal_fixed_width, horizontal_grow_node};
        let gap = 12.0_f32;
        let total_w = 800.0_f32;
        let sidebar = UiNode::Sized {
            width: Some(160.0),
            height: None,
            child: Box::new(UiNode::Text {
                text: "side".into(),
                size: 0.0,
                color: String::new(),
                bold: false,
                monospace: false,
            }),
        };
        let canvas = UiNode::Canvas {
            commands: vec![],
            width: 0.0,
            height: 0.0,
            grow: true,
        };

        assert_eq!(horizontal_fixed_width(&sidebar), Some(160.0));
        assert_eq!(horizontal_fixed_width(&canvas), None);
        assert!(horizontal_grow_node(&canvas));
        assert!(!horizontal_grow_node(&sidebar));

        // Mirror render_horizontal_children's arithmetic.
        let children = [&sidebar, &canvas];
        let gap_total = gap * (children.len() - 1) as f32;
        let mut fixed_total = 0.0f32;
        let mut grow_count = 0usize;
        for c in &children {
            if horizontal_grow_node(c) {
                grow_count += 1;
            } else if let Some(w) = horizontal_fixed_width(c) {
                fixed_total += w;
            }
        }
        let grow_w = (total_w - fixed_total - gap_total) / grow_count as f32;
        assert_eq!(grow_w, total_w - 160.0 - gap);
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

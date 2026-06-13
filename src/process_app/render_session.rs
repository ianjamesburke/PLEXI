//! RenderSession — per-frame rendering state for a ProcessApp pane.
//!
//! Owns all widget passes (draw commands, TextInput, scroll regions) so that
//! `ProcessApp` itself only holds persistent app lifecycle state.

use crate::app_protocol::{PlexiEvent, RenderCommand};
use std::collections::{HashMap, HashSet};

pub(crate) struct RenderSession {
    /// Per-input text buffers, keyed on the `id` field of `DrawCommand::TextInput`.
    /// Persists between frames and survives submit (cleared on submit via `submit_text_input`).
    pub(crate) text_input_buffers: HashMap<String, String>,
    /// Per-TextEdit node buffers for `UiNode::TextEdit` in the component tree.
    /// Keyed on `node_id`. Seeded from the app's `value` field when a new
    /// node_id appears; persists across frames so the host owns the live text.
    pub(crate) text_edit_buffers: HashMap<String, String>,
    /// IDs of `TextInput` widgets visible in the most recently rendered frame.
    /// Used to detect newly-visible inputs for auto-focus.
    text_input_visible_prev: HashSet<String>,
    /// Scratch set for the current frame's visible `TextInput` ids. Swapped
    /// into `text_input_visible_prev` at the end of each render pass so the
    /// allocation is reused instead of rebuilt every frame.
    text_input_visible_current: HashSet<String>,
    /// True when any `TextInput` or `TextEdit` widget had egui focus during the
    /// last render pass. Read by `handle_key` in mod.rs to suppress key
    /// forwarding while the user types.
    pub(crate) text_input_has_focus: bool,
    /// Per-region scroll offsets for `DrawCommand::BeginScroll` / `EndScroll`.
    /// Key = scroll region id; value = current vertical offset in logical pixels.
    scroll_offsets: HashMap<String, f32>,
    /// Per-ListView scroll offsets. Key = list_view id; value = current vertical offset.
    list_view_scroll_offsets: HashMap<String, f32>,
    /// Tracks the last selected index that triggered a scroll-to-selected alignment.
    /// Scroll-to-selected only fires when the selection index changes, so mouse-wheel
    /// scrolls are not overridden every frame.
    list_view_last_aligned_sel: HashMap<String, usize>,
    /// True when the current frame contains a ListView command.
    /// When true, `handle_key` in mod.rs suppresses j/k/up/down/enter forwarding.
    pub(crate) list_view_intercepts_nav: bool,
    /// Set externally by `src/pane_ops/layout.rs` when a pane gains keyboard focus.
    /// Read during render to auto-focus the first TextInput.
    pub(crate) pane_just_focused: bool,
    /// Events accumulated during one render pass. Drained after render into
    /// `ProcessApp::outbound_events` by `drain_events`.
    pub(crate) outbound_events: Vec<PlexiEvent>,
    /// Focus/visibility tracking for `UiNode::TextEdit` nodes in component trees.
    /// Persists across frames to detect newly-visible fields for auto-focus.
    text_edit_focus_ctx: crate::render::components::TextEditFocusCtx,
}

impl RenderSession {
    pub(crate) fn new() -> Self {
        Self {
            text_input_buffers: HashMap::new(),
            text_edit_buffers: HashMap::new(),
            text_input_visible_prev: HashSet::new(),
            text_input_visible_current: HashSet::new(),
            text_input_has_focus: false,
            scroll_offsets: HashMap::new(),
            list_view_scroll_offsets: HashMap::new(),
            list_view_last_aligned_sel: HashMap::new(),
            list_view_intercepts_nav: false,
            pane_just_focused: false,
            outbound_events: Vec::new(),
            text_edit_focus_ctx: crate::render::components::TextEditFocusCtx::new(),
        }
    }

    /// Execute all rendering passes for one frame.
    ///
    /// Pass 1 — draw commands (painter-only, no egui allocation)
    /// Pass 2 — TextInput widgets (real egui widgets with buffer state)
    /// Pass 3 — scroll region scan (pointer interaction, emits ScrollOffset events)
    /// Pass 4 — ListView interaction (j/k/enter, scroll gesture)
    /// Pass 5 — app scroll delta (emits Scroll for SDK Scrollable when wheel
    ///           moves over the pane but no host-managed region consumed it)
    pub(crate) fn render(
        &mut self,
        ui: &mut egui::Ui,
        pane_rect: egui::Rect,
        frame: &[RenderCommand],
        colors: &crate::ui::theme::Colors,
        commonmark_cache: &mut egui_commonmark::CommonMarkCache,
        audio_peaks: &HashMap<String, f32>,
        pane_id: u64,
        image_cache: &mut super::image_cache::ImageCache,
        app_dir: &std::path::Path,
        net_http_granted: bool,
        is_focused: bool,
    ) {
        log::debug!(
            "render_session: render pane_id={} cmds={}",
            pane_id,
            frame.len()
        );

        // Propagate pane_just_focused into the TextEdit focus context so
        // ComponentTree TextEdit nodes auto-focus on pane switch.
        self.text_edit_focus_ctx.pane_just_focused = self.pane_just_focused;

        // ── Pass 1: draw commands ────────────────────────────────────────────
        crate::process_app::render::render_draw_commands(
            ui,
            pane_rect,
            frame,
            colors,
            commonmark_cache,
            audio_peaks,
            image_cache,
            app_dir,
            net_http_granted,
            &mut self.list_view_scroll_offsets,
            &mut self.list_view_last_aligned_sel,
            &mut self.outbound_events,
            &mut self.text_edit_buffers,
            &mut self.text_edit_focus_ctx,
        );

        // ── Pass 2: TextInput widgets ────────────────────────────────────────
        self.render_text_inputs(ui, pane_rect, frame, pane_id, colors);

        // ── Pass 3: scroll region scan ───────────────────────────────────────
        let scroll_consumed = self.render_scroll_regions(ui, pane_rect, frame);

        // ── Pass 4: ListView interaction (j/k/enter, scroll gesture) ─────────
        self.render_list_views(ui, pane_rect, frame, is_focused);

        // ── Pass 5: app scroll delta for SDK Scrollable ───────────────────────
        if !scroll_consumed {
            self.render_app_scroll(ui, pane_rect, frame);
        }

        // Merge TextEdit focus state: if any ComponentTree TextEdit has focus,
        // suppress key forwarding (same as TextInput focus).
        if self.text_edit_focus_ctx.any_has_focus {
            self.text_input_has_focus = true;
        }

        // Rotate TextEdit visibility sets for next-frame auto-focus detection.
        self.text_edit_focus_ctx.end_frame();
    }

    /// Pass 2: render every `RenderCommand::TextInput` as a real egui `TextEdit`.
    fn render_text_inputs(
        &mut self,
        ui: &mut egui::Ui,
        pane_rect: egui::Rect,
        frame: &[RenderCommand],
        pane_id: u64,
        colors: &crate::ui::theme::Colors,
    ) {
        let origin = pane_rect.min;
        let mut submitted: Vec<String> = Vec::new();
        self.text_input_visible_current.clear();
        let mut focus_granted = false;
        let mut any_has_focus = false;

        for cmd in frame {
            let RenderCommand::TextInput {
                id,
                x,
                y,
                w,
                h,
                placeholder,
                multiline,
                value,
            } = cmd
            else {
                continue;
            };

            self.text_input_visible_current.insert(id.clone());
            let newly_visible = !self.text_input_visible_prev.contains(id.as_str());

            let desired_h = h.max(24.0);
            let min_x = origin.x + x;
            let min_y = origin.y + y;
            let max_x = (min_x + *w).min(pane_rect.max.x);
            let max_y = (min_y + desired_h).min(pane_rect.max.y);
            if max_x <= min_x || max_y <= min_y {
                continue;
            }
            let widget_rect =
                egui::Rect::from_min_max(egui::pos2(min_x, min_y), egui::pos2(max_x, max_y));
            let actual_size = widget_rect.size();

            let widget_id = ui.id().with(("text_input", pane_id, id.as_str()));

            let (resp, cursor_char_idx, changed_value) = {
                let buffer = self.text_input_buffers.entry(id.clone()).or_default();
                if let Some(value) = value {
                    if buffer != value {
                        *buffer = value.clone();
                    }
                }
                // Draw single pill background (fill only).
                ui.painter().rect_filled(
                    widget_rect,
                    crate::ui::style::RADIUS_MD,
                    colors.bg_active,
                );

                let mut child = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(widget_rect)
                        .id_salt(widget_id),
                );
                // egui's caret is hidden (transparent, non-blinking);
                // draw_text_caret paints a glyph-height replacement on top.
                child.visuals_mut().text_cursor.blink = false;
                child.visuals_mut().text_cursor.stroke.color = egui::Color32::TRANSPARENT;

                // Placeholder at half strength — the theme's override_text_color
                // makes egui's default hint color near-white otherwise.
                let hint = egui::RichText::new(placeholder.as_str())
                    .color(colors.text_primary.gamma_multiply(0.5));
                // TextEdit defaults to the Monospace text style — app input
                // fields must read as UI chrome (Inter), not terminal text.
                let output = if *multiline {
                    let edit = egui::TextEdit::multiline(buffer)
                        .id(widget_id)
                        .desired_width(actual_size.x)
                        .hint_text(hint)
                        .font(egui::TextStyle::Body)
                        .frame(false);
                    egui::ScrollArea::vertical()
                        .max_height(actual_size.y)
                        .show(&mut child, |ui| {
                            let font_id = egui::TextStyle::Body.resolve(ui.style());
                            let row_height = ui.fonts(|f| f.row_height(&font_id));
                            let output = edit.show(ui);
                            crate::ui::text_field::draw_text_caret(
                                ui,
                                &output,
                                font_id.size,
                                row_height,
                                egui::Stroke::new(1.5, colors.accent),
                            );
                            output
                        })
                        .inner
                } else {
                    let font_size = child.text_style_height(&egui::TextStyle::Body);
                    let text_h = font_size * 1.4;
                    let v_inset = ((actual_size.y - text_h) * 0.5).max(4.0);
                    let inner_rect = widget_rect.shrink2(egui::vec2(8.0, v_inset));
                    let mut inner_child = child.new_child(
                        egui::UiBuilder::new()
                            .max_rect(inner_rect)
                            .id_salt(widget_id.with("c")),
                    );
                    // egui's caret is hidden (transparent, non-blinking);
                    // draw_text_caret paints a glyph-height replacement on top.
                    inner_child.visuals_mut().text_cursor.blink = false;
                    inner_child.visuals_mut().text_cursor.stroke.color =
                        egui::Color32::TRANSPARENT;
                    let edit = egui::TextEdit::singleline(buffer)
                        .id(widget_id)
                        .desired_width(inner_rect.width())
                        .hint_text(hint)
                        .font(egui::TextStyle::Body)
                        .frame(false);
                    let font_id = egui::TextStyle::Body.resolve(inner_child.style());
                    let row_height = inner_child.fonts(|f| f.row_height(&font_id));
                    let output = edit.show(&mut inner_child);
                    crate::ui::text_field::draw_text_caret(
                        &inner_child,
                        &output,
                        font_id.size,
                        row_height,
                        egui::Stroke::new(1.5, colors.accent),
                    );
                    output
                };

                // Draw focus-aware border over the pill — one stroke, no glow ring.
                let border_color = if output.response.has_focus() {
                    colors.accent
                } else {
                    colors.border
                };
                ui.painter().rect_stroke(
                    widget_rect,
                    crate::ui::style::RADIUS_MD,
                    egui::Stroke::new(1.0, border_color),
                    egui::StrokeKind::Outside,
                );

                if (newly_visible || self.pane_just_focused) && !focus_granted {
                    output.response.request_focus();
                    focus_granted = true;
                }
                let cursor_idx = output.cursor_range.map(|cr| cr.primary.ccursor.index);
                // response.changed() avoids the per-frame full-buffer
                // clone+compare the old prev_value snapshot required.
                let changed_value = output.response.changed().then(|| buffer.clone());
                (output.response, cursor_idx, changed_value)
            };

            if let Some(value) = changed_value {
                log::info!(
                    "render_session: TextInput changed id={} len={}",
                    id,
                    value.len()
                );
                self.outbound_events.push(PlexiEvent::TextChanged {
                    id: id.clone(),
                    value,
                });
            }

            let pointer_pressed_inside = ui
                .input(|i| i.pointer.button_pressed(egui::PointerButton::Primary))
                && ui.input(|i| {
                    i.pointer
                        .interact_pos()
                        .map_or(false, |pos| widget_rect.contains(pos))
                });
            if pointer_pressed_inside {
                resp.request_focus();
            }

            if resp.has_focus() {
                any_has_focus = true;
                let control_key = ui.input(|i| {
                    if i.key_pressed(egui::Key::Tab) {
                        Some("tab")
                    } else if i.key_pressed(egui::Key::ArrowDown) {
                        Some("down")
                    } else if i.key_pressed(egui::Key::ArrowUp) {
                        Some("up")
                    } else if i.key_pressed(egui::Key::Escape) {
                        Some("escape")
                    } else {
                        None
                    }
                });
                if let Some(key) = control_key {
                    let modifiers = ui.input(|i| crate::app_protocol::Modifiers {
                        shift: i.modifiers.shift,
                        ctrl: i.modifiers.ctrl,
                        alt: i.modifiers.alt,
                        cmd: i.modifiers.command,
                    });
                    self.outbound_events.push(PlexiEvent::TextInputKey {
                        id: id.clone(),
                        key: key.to_string(),
                        modifiers,
                    });
                }
            }

            if *multiline {
                if resp.has_focus() {
                    let enter_no_shift =
                        ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift);
                    let shift_enter =
                        ui.input(|i| i.key_pressed(egui::Key::Enter) && i.modifiers.shift);

                    if enter_no_shift {
                        if let Some(buf) = self.text_input_buffers.get_mut(id.as_str()) {
                            if buf.ends_with('\n') {
                                buf.pop();
                            }
                        }
                        submitted.push(id.clone());
                    } else if shift_enter {
                        if let (Some(buf), Some(char_idx)) = (
                            self.text_input_buffers.get_mut(id.as_str()),
                            cursor_char_idx,
                        ) {
                            let byte_idx = buf
                                .char_indices()
                                .nth(char_idx)
                                .map(|(i, _)| i)
                                .unwrap_or(buf.len());
                            buf.insert(byte_idx, '\n');
                        }
                    }
                }
            } else {
                if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    submitted.push(id.clone());
                    resp.request_focus();
                }
            }
        }

        // Rotate visibility sets, reusing both allocations across frames.
        std::mem::swap(
            &mut self.text_input_visible_prev,
            &mut self.text_input_visible_current,
        );
        self.text_input_visible_current.clear();
        self.pane_just_focused = false;
        self.text_input_has_focus = any_has_focus;

        for id in submitted {
            self.push_submit_event(&id);
        }
    }

    /// Pass 3: scan for scroll regions and emit `ScrollOffset` events.
    /// Returns true if a `BeginScroll` region consumed the wheel input so that
    /// Pass 5 knows not to emit a fallback `PlexiEvent::Scroll`.
    fn render_scroll_regions(
        &mut self,
        ui: &mut egui::Ui,
        pane_rect: egui::Rect,
        frame: &[RenderCommand],
    ) -> bool {
        let scroll_delta = ui.input(|i| i.smooth_scroll_delta);
        if scroll_delta.y == 0.0 {
            return false;
        }
        let Some(pointer_pos) = ui.input(|i| i.pointer.hover_pos()) else {
            return false;
        };

        let mut scroll_regions: Vec<(&String, egui::Rect, f32)> = Vec::new();
        let origin = pane_rect.min;
        for cmd in frame {
            if let RenderCommand::BeginScroll {
                id,
                x,
                y,
                w,
                h,
                content_height,
            } = cmd
            {
                let viewport = egui::Rect::from_min_size(
                    egui::pos2(origin.x + x, origin.y + y),
                    egui::vec2(*w, *h),
                );
                scroll_regions.push((id, viewport, *content_height));
            }
        }

        for (id, viewport, content_height) in scroll_regions.iter().rev() {
            if viewport.contains(pointer_pos) {
                let viewport_h = viewport.height();
                let max_offset = (content_height - viewport_h).max(0.0);
                let prev = self.scroll_offsets.get(*id).copied().unwrap_or(0.0);
                let next = (prev - scroll_delta.y).clamp(0.0, max_offset);
                if (next - prev).abs() > 0.01 {
                    self.scroll_offsets.insert((*id).clone(), next);
                    self.outbound_events.push(PlexiEvent::ScrollOffset {
                        id: (*id).clone(),
                        offset_y: next,
                    });
                }
                return true;
            }
        }
        false
    }

    /// Pass 5: forward raw wheel delta to the app as `PlexiEvent::Scroll` so
    /// SDK `Scrollable` components can update their offset. Only fires when no
    /// `BeginScroll` region consumed the event (Pass 3 returns false) and the
    /// cursor is over the pane but not over a `ListView` rect (Pass 4 handles
    /// those).
    fn render_app_scroll(
        &mut self,
        ui: &mut egui::Ui,
        pane_rect: egui::Rect,
        frame: &[RenderCommand],
    ) {
        let scroll_delta = ui.input(|i| i.smooth_scroll_delta);
        if scroll_delta.y == 0.0 {
            return;
        }
        let Some(pointer_pos) = ui.input(|i| i.pointer.hover_pos()) else {
            return;
        };
        if !pane_rect.contains(pointer_pos) {
            return;
        }

        // Skip if the cursor is inside a ListView (Pass 4 already handled it).
        for cmd in frame {
            if let RenderCommand::ListView { x, y, w, h, .. } = cmd {
                let list_w = if *w > 0.0 { *w } else { pane_rect.width() };
                let list_h = if *h > 0.0 { *h } else { pane_rect.height() - y };
                let list_rect = egui::Rect::from_min_size(
                    egui::pos2(pane_rect.min.x + x, pane_rect.min.y + y),
                    egui::vec2(list_w, list_h),
                );
                if list_rect.contains(pointer_pos) {
                    return;
                }
            }
        }

        log::info!(
            "render_session: forwarding wheel delta_y={} to app",
            scroll_delta.y
        );
        self.outbound_events.push(PlexiEvent::Scroll {
            delta_y: scroll_delta.y,
        });
    }

    /// Private helper: drain the buffer for `id` and push a `TextSubmitted` event
    /// to `self.outbound_events`.
    fn push_submit_event(&mut self, id: &str) {
        let value = self.text_input_buffers.remove(id).unwrap_or_default();
        self.outbound_events.push(PlexiEvent::TextSubmitted {
            id: id.to_string(),
            value,
        });
    }

    /// Drain accumulated events. Returns an iterator the caller extends into
    /// `ProcessApp::outbound_events`.
    pub(crate) fn drain_events(&mut self) -> std::vec::Drain<'_, PlexiEvent> {
        self.outbound_events.drain(..)
    }

    /// Remove the buffer for `id` (or use empty string) and return a
    /// `PlexiEvent::TextSubmitted` event. Called by `ProcessApp::submit_text_input`
    /// in tests — production submit goes through `push_submit_event` inside `render`.
    #[cfg(test)]
    pub(crate) fn submit_text_input(&mut self, id: &str) -> PlexiEvent {
        let value = self.text_input_buffers.remove(id).unwrap_or_default();
        PlexiEvent::TextSubmitted {
            id: id.to_string(),
            value,
        }
    }

    /// Pass 4: handle ListView pointer scroll, j/k/enter key events.
    fn render_list_views(
        &mut self,
        ui: &mut egui::Ui,
        pane_rect: egui::Rect,
        frame: &[RenderCommand],
        is_focused: bool,
    ) {
        use crate::app_protocol::PlexiEvent;

        // Reset intercept flag — recalculate from current frame
        self.list_view_intercepts_nav = false;

        // Prune scroll state for lists no longer in the frame. A frame holds
        // at most a handful of ListView commands, so a linear scan per stale
        // key beats rebuilding a HashSet of cloned ids every frame.
        let is_live = |id: &String| {
            frame
                .iter()
                .any(|cmd| matches!(cmd, RenderCommand::ListView { id: live, .. } if live == id))
        };
        self.list_view_scroll_offsets.retain(|id, _| is_live(id));
        self.list_view_last_aligned_sel.retain(|id, _| is_live(id));

        let mut handled_nav = false;
        for cmd in frame {
            let RenderCommand::ListView {
                id,
                x,
                y,
                w,
                h,
                items,
                selected,
                loading,
                ..
            } = cmd
            else {
                continue;
            };

            // This frame has a list_view — host intercepts j/k/enter
            self.list_view_intercepts_nav = true;

            let list_w = if *w > 0.0 { *w } else { pane_rect.width() };
            let list_h = if *h > 0.0 { *h } else { pane_rect.height() - y };
            let list_rect = egui::Rect::from_min_size(
                egui::pos2(pane_rect.min.x + x, pane_rect.min.y + y),
                egui::vec2(list_w, list_h),
            );

            // Click detection — use raw input rather than ui.interact() to avoid
            // the pane-level Sense::click_and_drag() widget (mod.rs) claiming
            // the event first (last-registered widget wins in egui).
            if !items.is_empty() && !*loading {
                let is_double = ui.input(|i| {
                    i.pointer
                        .button_double_clicked(egui::PointerButton::Primary)
                });
                let is_click = ui.input(|i| {
                    i.pointer.button_released(egui::PointerButton::Primary)
                        && !i.pointer.is_decidedly_dragging()
                });
                if is_double || is_click {
                    if let Some(pos) = ui.input(|i| i.pointer.interact_pos()) {
                        if list_rect.contains(pos) {
                            let scroll_y = self
                                .list_view_scroll_offsets
                                .get(id.as_str())
                                .copied()
                                .unwrap_or(0.0);
                            let mut item_top = 0.0f32;
                            for (i, item) in items.iter().enumerate() {
                                let h = item.height();
                                let row_abs_y = list_rect.min.y + item_top - scroll_y;
                                let row_rect = egui::Rect::from_min_size(
                                    egui::pos2(list_rect.min.x, row_abs_y),
                                    egui::vec2(list_w, h),
                                );
                                if list_rect.intersect(row_rect).contains(pos) {
                                    if is_double {
                                        log::info!("ListView[{id}]: double-click → activate {i}");
                                        self.outbound_events.push(PlexiEvent::ListSelect {
                                            id: id.clone(),
                                            index: i,
                                        });
                                        self.outbound_events.push(PlexiEvent::ListActivate {
                                            id: id.clone(),
                                            index: i,
                                        });
                                    } else {
                                        log::info!("ListView[{id}]: click → select {i}");
                                        self.outbound_events.push(PlexiEvent::ListSelect {
                                            id: id.clone(),
                                            index: i,
                                        });
                                    }
                                    ui.ctx().request_repaint();
                                    break;
                                }
                                item_top += h;
                            }
                        }
                    }
                }
            }

            if handled_nav || *loading || items.is_empty() {
                continue;
            }

            let n = items.len();
            let sel = (*selected).min(n.saturating_sub(1));

            // Scroll gesture (pointer inside list_rect)
            let scroll_delta = ui.input(|i| i.smooth_scroll_delta);
            if scroll_delta.y != 0.0 {
                if let Some(pos) = ui.input(|i| i.pointer.hover_pos()) {
                    if list_rect.contains(pos) {
                        let total_h: f32 = items.iter().map(|item| item.height()).sum::<f32>();
                        let max_scroll = (total_h - list_h).max(0.0);
                        let prev = self
                            .list_view_scroll_offsets
                            .get(id.as_str())
                            .copied()
                            .unwrap_or(0.0);
                        let next = (prev - scroll_delta.y).clamp(0.0, max_scroll);
                        if (next - prev).abs() > 0.01 {
                            self.list_view_scroll_offsets.insert(id.clone(), next);
                            ui.ctx().request_repaint();
                        }
                    }
                }
            }

            // j / down — only when this pane has focus; prevents routing to
            // background app while a terminal pane is focused (#1795)
            let j_pressed = is_focused
                && ui.input(|i| i.key_pressed(egui::Key::J) || i.key_pressed(egui::Key::ArrowDown));
            if j_pressed && sel + 1 < n {
                let new_sel = sel + 1;
                log::info!("ListView[{id}]: j → select {new_sel}");
                self.outbound_events.push(PlexiEvent::ListSelect {
                    id: id.clone(),
                    index: new_sel,
                });
                ui.ctx().request_repaint();
                handled_nav = true;
            }

            // k / up
            let k_pressed = is_focused
                && ui.input(|i| i.key_pressed(egui::Key::K) || i.key_pressed(egui::Key::ArrowUp));
            if k_pressed && sel > 0 {
                let new_sel = sel - 1;
                log::info!("ListView[{id}]: k → select {new_sel}");
                self.outbound_events.push(PlexiEvent::ListSelect {
                    id: id.clone(),
                    index: new_sel,
                });
                ui.ctx().request_repaint();
                handled_nav = true;
            }

            // Enter
            let enter_pressed = is_focused && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if enter_pressed {
                log::info!("ListView[{id}]: enter → activate {sel}");
                self.outbound_events.push(PlexiEvent::ListActivate {
                    id: id.clone(),
                    index: sel,
                });
                ui.ctx().request_repaint();
                handled_nav = true;
            }
        }
    }
}

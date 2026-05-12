//! RenderSession — per-frame rendering state for a ProcessApp pane.
//!
//! Owns all widget passes (draw commands, TextInput, scroll regions) so that
//! `ProcessApp` itself only holds persistent app lifecycle state.

use std::collections::{HashMap, HashSet};
use crate::app_protocol::{PlexiEvent, RenderCommand};

pub(crate) struct RenderSession {
    /// Per-input text buffers, keyed on the `id` field of `DrawCommand::TextInput`.
    /// Persists between frames and survives submit (cleared on submit via `submit_text_input`).
    pub(crate) text_input_buffers: HashMap<String, String>,
    /// IDs of `TextInput` widgets visible in the most recently rendered frame.
    /// Used to detect newly-visible inputs for auto-focus.
    text_input_visible_prev: HashSet<String>,
    /// True when any `TextInput` widget had egui focus during the last render pass.
    /// Read by `handle_key` in mod.rs to suppress key forwarding while the user types.
    pub(crate) text_input_has_focus: bool,
    /// Per-region scroll offsets for `DrawCommand::BeginScroll` / `EndScroll`.
    /// Key = scroll region id; value = current vertical offset in logical pixels.
    scroll_offsets: HashMap<String, f32>,
    /// Set externally by `src/pane_ops/layout.rs` when a pane gains keyboard focus.
    /// Read during render to auto-focus the first TextInput.
    pub(crate) pane_just_focused: bool,
    /// Events accumulated during one render pass. Drained after render into
    /// `ProcessApp::outbound_events` by `drain_events`.
    outbound_events: Vec<PlexiEvent>,
}

impl RenderSession {
    pub(crate) fn new() -> Self {
        Self {
            text_input_buffers: HashMap::new(),
            text_input_visible_prev: HashSet::new(),
            text_input_has_focus: false,
            scroll_offsets: HashMap::new(),
            pane_just_focused: false,
            outbound_events: Vec::new(),
        }
    }

    /// Execute all three rendering passes for one frame.
    ///
    /// Pass 1 — draw commands (painter-only, no egui allocation)
    /// Pass 2 — TextInput widgets (real egui widgets with buffer state)
    /// Pass 3 — scroll region scan (pointer interaction, emits ScrollOffset events)
    pub(crate) fn render(
        &mut self,
        ui: &mut egui::Ui,
        pane_rect: egui::Rect,
        frame: &[RenderCommand],
        colors: &crate::theme::Colors,
        commonmark_cache: &mut egui_commonmark::CommonMarkCache,
        audio_peaks: &HashMap<String, f32>,
        pane_id: u64,
        image_cache: &mut super::image_cache::ImageCache,
        app_dir: &std::path::Path,
    ) {
        log::debug!("render_session: render pane_id={} cmds={}", pane_id, frame.len());

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
        );

        // ── Pass 2: TextInput widgets ────────────────────────────────────────
        self.render_text_inputs(ui, pane_rect, frame, pane_id, colors);

        // ── Pass 3: scroll region scan ───────────────────────────────────────
        self.render_scroll_regions(ui, pane_rect, frame);
    }

    /// Pass 2: render every `RenderCommand::TextInput` as a real egui `TextEdit`.
    fn render_text_inputs(
        &mut self,
        ui: &mut egui::Ui,
        pane_rect: egui::Rect,
        frame: &[RenderCommand],
        pane_id: u64,
        colors: &crate::theme::Colors,
    ) {
        let origin = pane_rect.min;
        let mut submitted: Vec<String> = Vec::new();
        let mut visible_this_frame: HashSet<String> = HashSet::new();
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
            } = cmd
            else {
                continue;
            };

            visible_this_frame.insert(id.clone());
            let newly_visible = !self.text_input_visible_prev.contains(id.as_str());

            let desired_h = h.max(24.0);
            let min_x = origin.x + x;
            let min_y = origin.y + y;
            let max_x = (min_x + *w).min(pane_rect.max.x);
            let max_y = (min_y + desired_h).min(pane_rect.max.y);
            if max_x <= min_x || max_y <= min_y {
                continue;
            }
            let widget_rect = egui::Rect::from_min_max(
                egui::pos2(min_x, min_y),
                egui::pos2(max_x, max_y),
            );
            let actual_size = widget_rect.size();

            let widget_id = ui.id().with(("text_input", pane_id, id.as_str()));

            let (resp, cursor_char_idx) = {
                let buffer = self.text_input_buffers.entry(id.clone()).or_default();
                let mut child = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(widget_rect)
                        .id_salt(widget_id),
                );
                child.visuals_mut().text_cursor.stroke.width = 1.5;
                child.visuals_mut().text_cursor.stroke.color = colors.accent;
                child.visuals_mut().extreme_bg_color = colors.bg_active;
                child.visuals_mut().widgets.active.bg_stroke = egui::Stroke::new(1.0, colors.accent);
                child.visuals_mut().widgets.inactive.bg_stroke = egui::Stroke::new(1.0, colors.border);
                let output = if *multiline {
                    let edit = egui::TextEdit::multiline(buffer)
                        .id(widget_id)
                        .desired_width(actual_size.x)
                        .hint_text(placeholder.as_str())
                        .frame(true);
                    egui::ScrollArea::vertical()
                        .max_height(actual_size.y)
                        .show(&mut child, |ui| edit.show(ui))
                        .inner
                } else {
                    let edit = egui::TextEdit::singleline(buffer)
                        .id(widget_id)
                        .desired_width(actual_size.x)
                        .hint_text(placeholder.as_str())
                        .frame(true);
                    edit.show(&mut child)
                };
                if (newly_visible || self.pane_just_focused) && !focus_granted {
                    output.response.request_focus();
                    focus_granted = true;
                }
                let cursor_idx = output.cursor_range.map(|cr| cr.primary.ccursor.index);
                (output.response, cursor_idx)
            };

            let pointer_pressed_inside = ui.input(|i| {
                i.pointer.button_pressed(egui::PointerButton::Primary)
            }) && ui.input(|i| {
                i.pointer
                    .interact_pos()
                    .map_or(false, |pos| widget_rect.contains(pos))
            });
            if pointer_pressed_inside {
                resp.request_focus();
            }

            if resp.has_focus() {
                any_has_focus = true;
            }

            if *multiline {
                if resp.has_focus() {
                    let enter_no_shift = ui.input(|i| {
                        i.key_pressed(egui::Key::Enter) && !i.modifiers.shift
                    });
                    let shift_enter = ui.input(|i| {
                        i.key_pressed(egui::Key::Enter) && i.modifiers.shift
                    });

                    if enter_no_shift {
                        if let Some(buf) = self.text_input_buffers.get_mut(id.as_str()) {
                            if buf.ends_with('\n') {
                                buf.pop();
                            }
                        }
                        submitted.push(id.clone());
                    } else if shift_enter {
                        if let (Some(buf), Some(char_idx)) =
                            (self.text_input_buffers.get_mut(id.as_str()), cursor_char_idx)
                        {
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

        self.text_input_visible_prev = visible_this_frame;
        self.pane_just_focused = false;
        self.text_input_has_focus = any_has_focus;

        for id in submitted {
            self.push_submit_event(&id);
        }
    }

    /// Pass 3: scan for scroll regions and emit `ScrollOffset` events.
    fn render_scroll_regions(
        &mut self,
        ui: &mut egui::Ui,
        pane_rect: egui::Rect,
        frame: &[RenderCommand],
    ) {
        let scroll_delta = ui.input(|i| i.smooth_scroll_delta);
        let pointer_pos = ui.input(|i| i.pointer.hover_pos());

        let mut scroll_regions: Vec<(&String, egui::Rect, f32)> = Vec::new();
        let origin = pane_rect.min;
        for cmd in frame {
            if let RenderCommand::BeginScroll { id, x, y, w, h, content_height } = cmd {
                let viewport = egui::Rect::from_min_size(
                    egui::pos2(origin.x + x, origin.y + y),
                    egui::vec2(*w, *h),
                );
                scroll_regions.push((id, viewport, *content_height));
            }
        }

        if scroll_delta.y != 0.0 {
            if let Some(pos) = pointer_pos {
                for (id, viewport, content_height) in scroll_regions.iter().rev() {
                    if viewport.contains(pos) {
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
                        break;
                    }
                }
            }
        }
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
}

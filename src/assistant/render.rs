//! egui rendering for the host Assistant pane: header bar, transcript,
//! streaming row, slash-command picker, and multiline composer. Pure view
//! over `AssistantModel` — all state transitions go back through the model.
//!
//! Layout: the composer (plus picker and permission sheet) lives in a
//! content-sized bottom panel so it stays glued to the pane's bottom edge —
//! when the picker's match list shrinks, the panel shrinks downward instead
//! of the composer floating up and leaving dead space below it. The
//! transcript fills whatever remains above.

use egui::RichText;

use crate::ui::button::{chrome_button, ButtonKind};
use crate::ui::hints::{HintBar, HintGroup};
use crate::ui::list::ListRow;
use crate::ui::style;
use crate::ui::theme::Colors;

use super::commands;
use super::model::{AssistantModel, PermissionChoice, ToolStatus, TurnRole};

/// What the composer asked the pane shell to do this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComposerEvent {
    Submit,
    /// The user decided the pending permission sheet.
    Permission(PermissionChoice),
}

/// Stateless renderer for the Assistant pane.
pub struct AssistantRenderer;

impl AssistantRenderer {
    /// Header bar height — mirrors the terminal pane name bar and the
    /// text-editor note header so the assistant reads as the same chrome.
    const HEADER_BAR_H: f32 = 20.0;
    const HEADER_FONT_SIZE: f32 = 11.0;
    /// Composer stops growing past this many text rows and scrolls instead.
    const COMPOSER_MAX_ROWS: f32 = 8.0;
    /// User bubbles cap at this fraction of the transcript width.
    const BUBBLE_MAX_FRACTION: f32 = 0.72;

    pub fn draw(
        ui: &mut egui::Ui,
        model: &mut AssistantModel,
        colors: &Colors,
        is_focused: bool,
    ) -> Option<ComposerEvent> {
        // Match the terminal/editor surface, not the darker app-pane base
        // fill — the assistant is host chrome, same as the scratchpad.
        ui.painter()
            .rect_filled(ui.available_rect_before_wrap(), 0.0, colors.terminal_bg);
        ui.visuals_mut().extreme_bg_color = colors.terminal_bg;

        Self::draw_header(ui, model, colors);

        let total_h = ui.available_rect_before_wrap().height();
        let picker_max_h = (total_h * 0.4).max(0.0);
        let mut event = None;

        // The bottom panel is content-sized with one frame of lag; keep
        // frames flowing while the picker is open so its height converges
        // immediately as the match list grows and shrinks.
        if model.picker_active() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(33));
        }

        // Bottom-anchored, content-sized: composer pinned to the pane bottom,
        // picker and permission sheet stacking directly above it.
        egui::TopBottomPanel::bottom(egui::Id::new("assistant_bottom").with(&model.conversation_id))
            .show_separator_line(false)
            .frame(egui::Frame::new().inner_margin(egui::Margin {
                left: style::SPACE_MD as i8,
                right: style::SPACE_MD as i8,
                top: style::SPACE_XS as i8,
                bottom: style::SPACE_SM as i8,
            }))
            .show_inside(ui, |ui| {
                if let Some(choice) = Self::draw_permission_sheet(ui, model, colors) {
                    event = Some(ComposerEvent::Permission(choice));
                }
                if model.picker_active() {
                    Self::draw_picker(ui, model, colors, picker_max_h);
                }
                if let Some(composer_event) = Self::draw_composer(ui, model, colors, is_focused) {
                    event = Some(composer_event);
                }
                let hints = [
                    HintGroup::new(&["\u{21b5}"], "send"),
                    HintGroup::new(&["\u{21e7}", "\u{21b5}"], "newline"),
                    HintGroup::new(&["/"], "commands"),
                ];
                HintBar::new(&hints).show(ui, colors);
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(
                style::SPACE_MD as i8,
                0,
            )))
            .show_inside(ui, |ui| {
                Self::draw_transcript(ui, model, colors);
            });

        event
    }

    /// Title bar styled like the terminal pane name bar: session name (or
    /// "Assistant"), with a status pip — accent while a turn streams, danger
    /// when the last turn errored, dim when idle.
    fn draw_header(ui: &mut egui::Ui, model: &AssistantModel, colors: &Colors) {
        let bar_rect = egui::Rect::from_min_size(
            ui.cursor().min,
            egui::vec2(ui.available_width(), Self::HEADER_BAR_H),
        );
        ui.advance_cursor_after_rect(bar_rect);
        ui.painter()
            .rect_filled(bar_rect, 0.0, colors.pane_header_bg());

        let pip_color = if model.streaming.in_flight {
            colors.accent
        } else if matches!(
            model.turns.last().map(|t| t.role),
            Some(TurnRole::Error)
        ) {
            colors.danger
        } else {
            colors.text_dim
        };
        ui.painter().circle_filled(
            egui::pos2(bar_rect.left() + style::SPACE_SM + 3.0, bar_rect.center().y),
            3.0,
            pip_color,
        );

        let title = model
            .session_name
            .clone()
            .unwrap_or_else(|| "Assistant".to_string());
        // A real label (not painter text) so the title stays in the
        // accessibility tree for UI-harness queries.
        let mut bar_ui = ui.new_child(egui::UiBuilder::new().max_rect(bar_rect));
        bar_ui.centered_and_justified(|ui| {
            ui.label(
                RichText::new(title)
                    .size(Self::HEADER_FONT_SIZE)
                    .color(colors.text_dim),
            );
        });
    }

    fn draw_transcript(ui: &mut egui::Ui, model: &AssistantModel, colors: &Colors) {
        egui::ScrollArea::vertical()
            .id_salt("assistant_transcript")
            .auto_shrink([false, false])
            // Pin to bottom only while a turn is streaming in.
            .stick_to_bottom(model.streaming.in_flight)
            .show(ui, |ui| {
                ui.add_space(style::SPACE_SM);
                for turn in &model.turns {
                    Self::draw_turn_row(ui, colors, turn.role, &turn.text, turn.status);
                }
                for active in &model.active_tools {
                    Self::draw_active_tool_row(ui, colors, &active.tool);
                }
                if model.streaming.in_flight {
                    Self::draw_streaming_row(ui, model, colors);
                }
                ui.add_space(style::SPACE_SM);
            });
    }

    fn draw_turn_row(
        ui: &mut egui::Ui,
        colors: &Colors,
        role: TurnRole,
        text: &str,
        status: Option<ToolStatus>,
    ) {
        match role {
            // Delivered app events are compact single-line rows.
            TurnRole::Event => {
                ui.label(
                    RichText::new(text)
                        .size(style::TEXT_CAPTION)
                        .monospace()
                        .color(colors.accent),
                );
                ui.add_space(style::SPACE_SM);
            }
            // Completed tool calls are compact single-line rows.
            TurnRole::Tool => {
                let (icon, color) = match status {
                    Some(ToolStatus::Succeeded) | None => ("✓", colors.text_dim),
                    Some(ToolStatus::Failed) => ("✗", colors.danger),
                };
                ui.label(
                    RichText::new(format!("{icon} {text}"))
                        .size(style::TEXT_CAPTION)
                        .monospace()
                        .color(color),
                );
                ui.add_space(style::SPACE_SM);
            }
            // User turns sit right-aligned in an outlined bubble, like every
            // mainstream chat client.
            TurnRole::User => {
                let bubble_max = ui.available_width() * Self::BUBBLE_MAX_FRACTION;
                ui.with_layout(egui::Layout::top_down(egui::Align::Max), |ui| {
                    egui::Frame::new()
                        .fill(colors.bg_active)
                        .stroke(egui::Stroke::new(1.0, colors.border))
                        .corner_radius(style::RADIUS_LG)
                        .inner_margin(egui::Margin::symmetric(
                            style::SPACE_SM as i8,
                            style::SPACE_XS as i8,
                        ))
                        .show(ui, |ui| {
                            ui.set_max_width(bubble_max);
                            ui.label(
                                RichText::new(text)
                                    .size(style::TEXT_BODY)
                                    .color(colors.text_primary),
                            );
                        });
                });
                ui.add_space(style::SPACE_MD);
            }
            // Assistant replies flow plain, left-aligned, full-width.
            TurnRole::Assistant => {
                ui.label(
                    RichText::new(text)
                        .size(style::TEXT_BODY)
                        .color(colors.text_primary),
                );
                ui.add_space(style::SPACE_MD);
            }
            TurnRole::Error => {
                ui.label(
                    RichText::new("error")
                        .size(style::TEXT_HINT)
                        .color(colors.danger),
                );
                ui.label(
                    RichText::new(text)
                        .size(style::TEXT_BODY)
                        .color(colors.danger),
                );
                ui.add_space(style::SPACE_MD);
            }
        }
    }

    /// A tool call currently running inside the in-flight turn.
    fn draw_active_tool_row(ui: &mut egui::Ui, colors: &Colors, tool: &str) {
        ui.label(
            RichText::new(format!("⟳ {tool} — running…"))
                .size(style::TEXT_CAPTION)
                .monospace()
                .color(colors.accent),
        );
        ui.add_space(style::SPACE_SM);
    }

    /// Permission sheet for the pending ask-gated tool call, rendered above
    /// the composer. Returns the user's decision, if any, this frame.
    fn draw_permission_sheet(
        ui: &mut egui::Ui,
        model: &AssistantModel,
        colors: &Colors,
    ) -> Option<PermissionChoice> {
        let pending = model.pending_permission.as_ref()?;
        let mut choice = None;
        egui::Frame::new()
            .fill(colors.bg_active)
            .stroke(egui::Stroke::new(1.0, colors.accent))
            .corner_radius(style::RADIUS_MD)
            .inner_margin(egui::Margin::same(style::SPACE_SM as i8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(
                    RichText::new("Permission required")
                        .size(style::TEXT_CAPTION)
                        .color(colors.accent),
                );
                ui.label(
                    RichText::new(format!(
                        "assistant (medium) wants to run the app tool '{}'",
                        pending.tool
                    ))
                    .size(style::TEXT_BODY)
                    .color(colors.text_primary),
                );
                if !pending.input_summary.is_empty() {
                    ui.scope(|ui| {
                        ui.set_max_width(ui.available_width());
                        crate::ui::labels::description_label(ui, &pending.input_summary, colors);
                    });
                }
                ui.add_space(style::SPACE_XS);
                ui.horizontal(|ui| {
                    if chrome_button(ui, "Allow once", ButtonKind::Accent, colors, 0.0).clicked() {
                        choice = Some(PermissionChoice::AllowOnce);
                    }
                    if chrome_button(ui, "Allow this session", ButtonKind::Primary, colors, 0.0)
                        .clicked()
                    {
                        choice = Some(PermissionChoice::AllowSession);
                    }
                    if chrome_button(ui, "Always allow", ButtonKind::Primary, colors, 0.0).clicked()
                    {
                        choice = Some(PermissionChoice::AllowAlways);
                    }
                    if chrome_button(ui, "Deny", ButtonKind::Danger, colors, 0.0).clicked() {
                        choice = Some(PermissionChoice::Deny);
                    }
                });
            });
        ui.add_space(style::SPACE_XS);
        choice
    }

    fn draw_streaming_row(ui: &mut egui::Ui, model: &AssistantModel, colors: &Colors) {
        if !model.streaming.partial_reasoning.is_empty() {
            egui::CollapsingHeader::new(
                RichText::new("thinking…")
                    .size(style::TEXT_CAPTION)
                    .color(colors.text_dim),
            )
            .id_salt("assistant_thinking")
            .default_open(false)
            .show(ui, |ui| {
                ui.label(
                    RichText::new(&model.streaming.partial_reasoning)
                        .size(style::TEXT_CAPTION)
                        .color(colors.text_dim),
                );
            });
        }
        if model.streaming.partial_answer.is_empty() {
            ui.label(
                RichText::new("…")
                    .size(style::TEXT_BODY)
                    .color(colors.text_dim),
            );
        } else {
            ui.label(
                RichText::new(&model.streaming.partial_answer)
                    .size(style::TEXT_BODY)
                    .color(colors.text_primary),
            );
        }
        ui.add_space(style::SPACE_MD);
    }

    /// Stable egui id for the composer TextEdit, salted by conversation so
    /// two assistant panes never share cursor state.
    fn composer_id(model: &AssistantModel) -> egui::Id {
        egui::Id::new("assistant_composer").with(&model.conversation_id)
    }

    /// Move the composer caret to the end of `text` — used after picker
    /// completion replaces the buffer, so typing continues after the
    /// inserted trailing space instead of mid-command.
    fn set_caret_to_end(ctx: &egui::Context, te_id: egui::Id, text: &str) {
        let mut state = egui::TextEdit::load_state(ctx, te_id).unwrap_or_default();
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::one(
                egui::text::CCursor::new(text.chars().count()),
            )));
        state.store(ctx, te_id);
    }

    fn draw_picker(ui: &mut egui::Ui, model: &mut AssistantModel, colors: &Colors, max_h: f32) {
        let matches = commands::filter_commands(&model.picker_query());
        if matches.is_empty() {
            return;
        }
        if model.picker_selected >= matches.len() {
            model.picker_selected = matches.len() - 1;
        }

        // Picker keyboard nav — consumed here so the composer never sees it.
        let mut complete = false;
        ui.input_mut(|input| {
            if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown)
                && model.picker_selected + 1 < matches.len()
            {
                model.picker_selected += 1;
            }
            if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp)
                && model.picker_selected > 0
            {
                model.picker_selected -= 1;
            }
            if input.consume_key(egui::Modifiers::NONE, egui::Key::Tab) {
                complete = true;
            }
        });
        if complete {
            let (name, _) = matches[model.picker_selected];
            model.composer = format!("/{name} ");
            Self::set_caret_to_end(ui.ctx(), Self::composer_id(model), &model.composer);
            log::info!("assistant: picker completed '/{name}' via Tab");
            return;
        }

        let selected_idx = model.picker_selected;
        let mut clicked: Option<&'static str> = None;
        egui::Frame::new()
            .fill(colors.bg_active)
            .stroke(egui::Stroke::new(1.0, colors.border))
            .corner_radius(style::RADIUS_MD)
            .inner_margin(egui::Margin::same(style::SPACE_XS as i8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                egui::ScrollArea::vertical()
                    .id_salt("assistant_picker_scroll")
                    .max_height(max_h.max(0.0))
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        for (i, (name, purpose)) in matches.iter().enumerate() {
                            let row = ListRow::new(&format!("/{name}"))
                                .secondary(purpose)
                                .dense()
                                .selected(i == selected_idx)
                                .show(ui, colors);
                            if i == selected_idx {
                                row.scroll_to_me(None);
                            }
                            if row.row_clicked() {
                                clicked = Some(*name);
                            }
                        }
                    });
            });
        ui.add_space(style::SPACE_XS);
        if let Some(name) = clicked {
            model.composer = format!("/{name} ");
            Self::set_caret_to_end(ui.ctx(), Self::composer_id(model), &model.composer);
            log::info!("assistant: picker completed '/{name}' via click");
        }
    }

    fn draw_composer(
        ui: &mut egui::Ui,
        model: &mut AssistantModel,
        colors: &Colors,
        is_focused: bool,
    ) -> Option<ComposerEvent> {
        let picker_open = model.picker_active();
        let te_id = Self::composer_id(model);
        let font_id = egui::FontId::proportional(style::TEXT_BODY);
        let row_height = ui.fonts(|f| f.row_height(&font_id));
        let max_text_h = row_height * Self::COMPOSER_MAX_ROWS;

        // Accent outline while the composer holds keyboard focus — same
        // affordance as the host text fields.
        let has_kb_focus = ui.memory(|m| m.has_focus(te_id));
        let stroke_color = if has_kb_focus { colors.accent } else { colors.border };

        let mut response = None;
        egui::Frame::new()
            .fill(colors.bg_active)
            .stroke(egui::Stroke::new(1.0, stroke_color))
            .corner_radius(style::RADIUS_LG)
            .inner_margin(egui::Margin::symmetric(
                style::SPACE_SM as i8,
                style::SPACE_XS as i8,
            ))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                // Grows with content up to COMPOSER_MAX_ROWS, then scrolls —
                // same shape as the quick-note entry.
                egui::ScrollArea::vertical()
                    .id_salt("assistant_composer_scroll")
                    .max_height(max_text_h)
                    .show(ui, |ui| {
                        response = Some(ui.add(
                            egui::TextEdit::multiline(&mut model.composer)
                                .id(te_id)
                                .desired_rows(1)
                                .desired_width(f32::INFINITY)
                                .frame(false)
                                .hint_text(
                                    RichText::new("Message the assistant — / for commands")
                                        .size(style::TEXT_CAPTION)
                                        .color(colors.text_dim),
                                )
                                .font(font_id.clone()),
                        ));
                    });
            });
        let response = response?;
        if is_focused && !response.has_focus() && model.composer.is_empty() {
            response.request_focus();
        }

        // Enter submits; Shift+Enter inserts a newline. TextEdit has already
        // inserted the newline by the time we see the key, so strip it.
        let enter = response.has_focus()
            && ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift);
        if !enter {
            return None;
        }
        if let Some(idx) = model.composer.rfind('\n') {
            model.composer.remove(idx);
        }
        if picker_open {
            // Enter while picking completes the selected command.
            let matches = commands::filter_commands(&model.picker_query());
            if let Some((name, _)) = matches.get(model.picker_selected) {
                model.composer = format!("/{name} ");
                Self::set_caret_to_end(ui.ctx(), te_id, &model.composer);
                return None;
            }
        }
        Some(ComposerEvent::Submit)
    }
}

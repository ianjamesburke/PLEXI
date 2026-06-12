//! egui rendering for the host Assistant pane: transcript, streaming row,
//! slash-command picker, and multiline composer. Pure view over
//! `AssistantModel` — all state transitions go back through the model.

use egui::RichText;

use crate::ui::button::{chrome_button, ButtonKind};
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
    /// Height reserved for the permission sheet when one is pending.
    const SHEET_H: f32 = 110.0;

    pub fn draw(
        ui: &mut egui::Ui,
        model: &mut AssistantModel,
        colors: &Colors,
        is_focused: bool,
    ) -> Option<ComposerEvent> {
        let total = ui.available_rect_before_wrap();
        let mut event = None;

        ui.vertical(|ui| {
            // Compute composer height dynamically from line count so the
            // transcript region shrinks/grows with the composer content.
            let line_count = model.composer.lines().count().max(2);
            let font_id = egui::FontId::proportional(style::TEXT_BODY);
            let row_height = ui.fonts(|f| f.row_height(&font_id));
            // Add top/bottom padding (SPACE_XS each side) + margin from add_space call.
            let composer_h = (line_count as f32 * row_height
                + style::SPACE_SM * 2.0
                + style::SPACE_XS * 2.0)
                .clamp(60.0, 200.0);
            let sheet_h = if model.pending_permission.is_some() {
                Self::SHEET_H
            } else {
                0.0
            };
            // Reserve space for picker (up to 40% of remaining height, min 0).
            let picker_max_h = if model.picker_active() {
                ((total.height() - composer_h - sheet_h) * 0.4).max(0.0)
            } else {
                0.0
            };
            let transcript_h = (total.height() - composer_h - sheet_h - picker_max_h).max(0.0);
            ui.allocate_ui(egui::vec2(total.width(), transcript_h), |ui| {
                Self::draw_transcript(ui, model, colors);
            });
            if let Some(choice) = Self::draw_permission_sheet(ui, model, colors) {
                event = Some(ComposerEvent::Permission(choice));
            }
            if model.picker_active() {
                Self::draw_picker(ui, model, colors, picker_max_h);
            }
            if let Some(composer_event) = Self::draw_composer(ui, model, colors, is_focused) {
                event = Some(composer_event);
            }
        });
        event
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

    fn role_label(role: TurnRole) -> &'static str {
        match role {
            TurnRole::User => "you",
            TurnRole::Assistant => "assistant",
            TurnRole::Tool => "tool",
            TurnRole::Error => "error",
            TurnRole::Event => "event",
        }
    }

    fn role_color(role: TurnRole, colors: &Colors) -> egui::Color32 {
        match role {
            TurnRole::User => colors.accent,
            TurnRole::Assistant => colors.text_dim,
            TurnRole::Tool => colors.text_dim,
            TurnRole::Error => colors.danger,
            TurnRole::Event => colors.accent,
        }
    }

    fn draw_turn_row(
        ui: &mut egui::Ui,
        colors: &Colors,
        role: TurnRole,
        text: &str,
        status: Option<ToolStatus>,
    ) {
        // Delivered app events are compact single-line rows.
        if role == TurnRole::Event {
            ui.horizontal(|ui| {
                ui.add_space(style::SPACE_SM);
                ui.label(
                    RichText::new(text)
                        .size(style::TEXT_CAPTION)
                        .monospace()
                        .color(colors.accent),
                );
            });
            ui.add_space(style::SPACE_SM);
            return;
        }
        // Completed tool calls are compact single-line rows.
        if role == TurnRole::Tool {
            let (icon, color) = match status {
                Some(ToolStatus::Succeeded) | None => ("✓", colors.text_dim),
                Some(ToolStatus::Failed) => ("✗", colors.danger),
            };
            ui.horizontal(|ui| {
                ui.add_space(style::SPACE_SM);
                ui.label(
                    RichText::new(format!("{icon} {text}"))
                        .size(style::TEXT_CAPTION)
                        .monospace()
                        .color(color),
                );
            });
            ui.add_space(style::SPACE_SM);
            return;
        }
        ui.horizontal(|ui| {
            ui.add_space(style::SPACE_SM);
            ui.vertical(|ui| {
                ui.label(
                    RichText::new(Self::role_label(role))
                        .size(style::TEXT_HINT)
                        .color(Self::role_color(role, colors)),
                );
                let body_color = if role == TurnRole::Error {
                    colors.danger
                } else {
                    colors.text_primary
                };
                ui.label(
                    RichText::new(text)
                        .size(style::TEXT_BODY)
                        .color(body_color),
                );
            });
        });
        ui.add_space(style::SPACE_MD);
    }

    /// A tool call currently running inside the in-flight turn.
    fn draw_active_tool_row(ui: &mut egui::Ui, colors: &Colors, tool: &str) {
        ui.horizontal(|ui| {
            ui.add_space(style::SPACE_SM);
            ui.label(
                RichText::new(format!("⟳ {tool} — running…"))
                    .size(style::TEXT_CAPTION)
                    .monospace()
                    .color(colors.accent),
            );
        });
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
        ui.horizontal(|ui| {
            ui.add_space(style::SPACE_SM);
            ui.vertical(|ui| {
                ui.label(
                    RichText::new("assistant")
                        .size(style::TEXT_HINT)
                        .color(colors.text_dim),
                );
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
            });
        });
        ui.add_space(style::SPACE_MD);
    }

    fn draw_picker(
        ui: &mut egui::Ui,
        model: &mut AssistantModel,
        colors: &Colors,
        max_h: f32,
    ) {
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
        }

        let selected_idx = model.picker_selected;
        egui::Frame::new()
            .fill(colors.bg_active)
            .stroke(egui::Stroke::new(1.0, colors.border))
            .corner_radius(style::RADIUS_MD)
            .inner_margin(egui::Margin::same(style::SPACE_SM as i8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                egui::ScrollArea::vertical()
                    .id_salt("assistant_picker_scroll")
                    .max_height(max_h.max(0.0))
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        for (i, (name, purpose)) in matches.iter().enumerate() {
                            let selected = i == selected_idx;
                            let name_color =
                                if selected { colors.accent } else { colors.text_primary };
                            let row_resp = ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(format!("/{name}"))
                                        .size(style::TEXT_CAPTION)
                                        .monospace()
                                        .color(name_color),
                                );
                                ui.add_space(style::SPACE_SM);
                                ui.scope(|ui| {
                                    ui.set_max_width(ui.available_width());
                                    crate::ui::labels::description_label(ui, purpose, colors);
                                });
                            });
                            if selected {
                                row_resp.response.scroll_to_me(None);
                            }
                        }
                    });
            });
    }

    fn draw_composer(
        ui: &mut egui::Ui,
        model: &mut AssistantModel,
        colors: &Colors,
        is_focused: bool,
    ) -> Option<ComposerEvent> {
        let picker_open = model.picker_active();
        ui.add_space(style::SPACE_XS);
        let response = ui.add(
            egui::TextEdit::multiline(&mut model.composer)
                .id_salt("assistant_composer")
                .desired_rows(2)
                .desired_width(f32::INFINITY)
                .hint_text(
                    RichText::new("Message the assistant — / for commands")
                        .size(style::TEXT_CAPTION)
                        .color(colors.text_dim),
                )
                .font(egui::FontId::proportional(style::TEXT_BODY)),
        );
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
                return None;
            }
        }
        Some(ComposerEvent::Submit)
    }
}

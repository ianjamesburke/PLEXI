//! egui rendering for the host Assistant pane: transcript, streaming row,
//! slash-command picker, and multiline composer. Pure view over
//! `AssistantModel` — all state transitions go back through the model.

use egui::RichText;

use crate::ui::style;
use crate::ui::theme::Colors;

use super::commands;
use super::model::{AssistantModel, TurnRole};

/// What the composer asked the pane shell to do this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComposerEvent {
    Submit,
}

/// Stateless renderer for the Assistant pane.
pub struct AssistantRenderer;

impl AssistantRenderer {
    /// Height reserved for the composer + picker at the bottom of the pane.
    const COMPOSER_H: f32 = 72.0;

    pub fn draw(
        ui: &mut egui::Ui,
        model: &mut AssistantModel,
        colors: &Colors,
        is_focused: bool,
    ) -> Option<ComposerEvent> {
        let total = ui.available_rect_before_wrap();
        let mut event = None;

        ui.vertical(|ui| {
            let transcript_h = (total.height() - Self::COMPOSER_H).max(0.0);
            ui.allocate_ui(egui::vec2(total.width(), transcript_h), |ui| {
                Self::draw_transcript(ui, model, colors);
            });
            if model.picker_active() {
                Self::draw_picker(ui, model, colors);
            }
            event = Self::draw_composer(ui, model, colors, is_focused);
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
                    Self::draw_turn_row(ui, colors, turn.role, &turn.text);
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
        }
    }

    fn role_color(role: TurnRole, colors: &Colors) -> egui::Color32 {
        match role {
            TurnRole::User => colors.accent,
            TurnRole::Assistant => colors.text_dim,
            TurnRole::Tool => colors.text_dim,
            TurnRole::Error => colors.danger,
        }
    }

    fn draw_turn_row(ui: &mut egui::Ui, colors: &Colors, role: TurnRole, text: &str) {
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

    fn draw_picker(ui: &mut egui::Ui, model: &mut AssistantModel, colors: &Colors) {
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

        egui::Frame::new()
            .fill(colors.bg_active)
            .stroke(egui::Stroke::new(1.0, colors.border))
            .corner_radius(style::RADIUS_MD)
            .inner_margin(egui::Margin::same(style::SPACE_SM as i8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                let visible = matches.len().min(8);
                for (i, (name, purpose)) in matches.iter().take(visible).enumerate() {
                    let selected = i == model.picker_selected;
                    let name_color = if selected { colors.accent } else { colors.text_primary };
                    ui.horizontal(|ui| {
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
                }
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

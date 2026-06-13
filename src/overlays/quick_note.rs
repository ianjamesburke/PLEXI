use super::*;

impl PlexiApp {
    /// Quick note compose phase: full-screen scrim + centered text input.
    pub(crate) fn draw_quick_note_modal(&mut self, ctx: &egui::Context) {
        use crate::ui::style;
        use egui::RichText;

        // Consume Esc to close.
        let esc = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        if esc {
            self.pop_focus_layer(&crate::app::FocusLayer::QuickNote);
            self.quick_note_text.clear();
            log::info!("QuickNote: modal dismissed");
            return;
        }

        // Explicitly consume paste events before the TextEdit renders.
        // egui::TextEdit processes Paste via filtered_events(), which requires
        // settled egui focus. When the zoom overlay is active it calls
        // set_focus(true) unconditionally, stealing focus from the TextEdit and
        // leaving paste unhandled — the event then falls through to the terminal
        // backend. Consuming here is consistent with how Esc and Enter are
        // handled above and guarantees paste lands in QuickNote every frame.
        let pasted: String = ctx.input_mut(|i| {
            let mut text = String::new();
            i.events.retain(|e| {
                if let egui::Event::Paste(t) = e {
                    text.push_str(t);
                    false
                } else {
                    true
                }
            });
            text
        });
        if !pasted.is_empty() {
            let te_id = egui::Id::new("quick_note_text");
            let inserted = if let Some(mut state) = egui::TextEdit::load_state(ctx, te_id) {
                if let Some(range) = state.cursor.char_range() {
                    let lo = range.primary.index.min(range.secondary.index);
                    let hi = range.primary.index.max(range.secondary.index);
                    let chars: Vec<char> = self.quick_note_text.chars().collect();
                    let mut new_chars: Vec<char> = chars[..lo].to_vec();
                    new_chars.extend(pasted.chars());
                    new_chars.extend(chars[hi..].iter().copied());
                    self.quick_note_text = new_chars.into_iter().collect();
                    let new_pos = lo + pasted.chars().count();
                    state
                        .cursor
                        .set_char_range(Some(egui::text::CCursorRange::one(
                            egui::text::CCursor::new(new_pos),
                        )));
                    state.store(ctx, te_id);
                    true
                } else {
                    false
                }
            } else {
                false
            };
            if !inserted {
                self.quick_note_text.push_str(&pasted);
            }
            log::info!("QuickNote: pasted {} chars", pasted.len());
        }

        // Plain Enter (no shift) → commit to inbox.
        let commit = ctx.input_mut(|i| {
            if !i.modifiers.shift && !i.modifiers.command {
                i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
            } else {
                false
            }
        });
        if commit && !self.quick_note_text.trim().is_empty() {
            let text = self.quick_note_text.clone();
            if self.commit_quick_note_to_inbox(&text) {
                self.pop_focus_layer(&crate::app::FocusLayer::QuickNote);
                self.quick_note_text.clear();
                log::info!("QuickNote: committed to inbox");
            }
            return;
        }

        let screen_rect = ctx.screen_rect();

        // Modal — grows from ~25% to ~80% of screen height as the user types.
        let modal_w = (screen_rect.width() * 0.72).min(864.0).max(480.0);
        let max_text_h = (screen_rect.height() * 0.8).max(80.0);
        let line_h = style::TEXT_BODY + 4.0;
        let initial_rows = ((screen_rect.height() * 0.25) / line_h).round() as usize;
        let initial_rows = initial_rows.max(3);
        let colors = self.colors;
        let response = crate::ui::overlay::ModalShell::centered("quick_note_modal")
            .width(modal_w)
            .show(ctx, &colors, |ui| {
                {
                    {
                        // Hint
                        ui.label(
                            RichText::new("Enter to save  ·  Shift+Enter for new line  ·  Esc to discard")
                                .color(self.colors.text_dim.linear_multiply(0.5))
                                .size(style::TEXT_HINT)
                                .family(egui::FontFamily::Monospace),
                        );
                        ui.add_space(style::SPACE_SM);

                        // Text input — starts at ~25% screen height, grows with content, caps at ~80%.
                        egui::ScrollArea::vertical()
                            .max_height(max_text_h)
                            .show(ui, |ui| {
                                ui.scope(|ui| {
                                    // egui's caret is hidden (transparent,
                                    // non-blinking); draw_text_caret paints a
                                    // glyph-height replacement on top.
                                    ui.visuals_mut().text_cursor.blink = false;
                                    ui.visuals_mut().text_cursor.stroke.color =
                                        egui::Color32::TRANSPARENT;

                                    let te_id = egui::Id::new("quick_note_text");
                                    let qn_font_id = egui::FontId::monospace(style::TEXT_BODY);
                                    let qn_row_height =
                                        ui.fonts(|f| f.row_height(&qn_font_id));
                                    let output = egui::TextEdit::multiline(
                                        &mut self.quick_note_text,
                                    )
                                    .id(te_id)
                                    .font(qn_font_id)
                                    .text_color(self.colors.text_primary)
                                    .desired_width(f32::INFINITY)
                                    .desired_rows(initial_rows)
                                    .frame(false)
                                    .hint_text(
                                        RichText::new("What's on your mind?")
                                            .color(
                                                self.colors.text_dim.linear_multiply(0.3),
                                            )
                                            .size(style::TEXT_BODY),
                                    )
                                    .show(ui);
                                    crate::ui::text_field::draw_text_caret(
                                        ui,
                                        &output,
                                        style::TEXT_BODY,
                                        qn_row_height,
                                        egui::Stroke::new(1.5, self.colors.accent),
                                    );
                                });
                            });
                    }
                }
            });
        if response.dismissed {
            self.pop_focus_layer(&crate::app::FocusLayer::QuickNote);
            self.quick_note_text.clear();
            log::info!("QuickNote: modal dismissed via click-away");
        }
    }

    pub(crate) fn quick_note_handle_key(
        &mut self,
        ctx: &egui::Context,
    ) -> crate::app::app_trait::KeyDisposition {
        // Consume Cmd+0 so poll_actions doesn't fire OpenQuickNote while the modal
        // is already open — that would reset mid-session note state.
        ctx.input_mut(|i| {
            i.consume_key(egui::Modifiers::COMMAND, egui::Key::Num0);
        });
        crate::app::app_trait::KeyDisposition::Consumed
    }
}

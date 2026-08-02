use super::*;

impl PlexiApp {
    /// Quick note compose phase: full-screen scrim + centered text input.
    pub(crate) fn draw_quick_note_modal(&mut self, ctx: &egui::Context) {
        use crate::ui::style;
        use egui::RichText;

        // Consume Esc to close.
        let esc = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        if esc {
            self.pop_focus_layer(&crate::app::FocusKind::QuickNote);
            self.quick_note_text.clear();
            self.quick_note_attachments.clear();
            log::info!("QuickNote: modal dismissed");
            return;
        }

        // The modal owns raw file drops before tiled panes render. Taking the
        // events here routes Cmd+0 drops through the same egui host input that
        // production window drops use, without leaking them into a pane behind
        // the modal.
        let dropped_files = ctx.input_mut(|i| std::mem::take(&mut i.raw.dropped_files));
        for file in dropped_files {
            match file.path {
                Some(path) => match self.stage_quick_note_image(path.clone()) {
                    Ok(()) => log::info!("QuickNote: drop accepted source={}", path.display()),
                    Err(error) => {
                        log::info!(
                            "QuickNote: drop rejected source={} reason={error}",
                            path.display()
                        );
                    }
                },
                None => log::info!("QuickNote: drop rejected reason=missing_local_path"),
            }
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
        if commit
            && (!self.quick_note_text.trim().is_empty() || !self.quick_note_attachments.is_empty())
        {
            let text = self.quick_note_text.clone();
            if self.commit_quick_note_to_inbox(&text) {
                self.pop_focus_layer(&crate::app::FocusKind::QuickNote);
                self.quick_note_text.clear();
                self.quick_note_attachments.clear();
                log::info!("QuickNote: committed to inbox");
            }
            return;
        }

        let screen_rect = ctx.content_rect();

        // Modal — grows from ~25% to ~80% of screen height as the user types.
        let modal_w = (screen_rect.width() * 0.72).clamp(480.0, 864.0);
        let max_text_h = (screen_rect.height() * 0.8).max(80.0);
        let font = self.quick_note_font();
        let line_h = font.size + 4.0;
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
                            RichText::new(
                                "Enter to save  ·  Shift+Enter for new line  ·  Esc to discard",
                            )
                            .color(self.colors.text_dim.linear_multiply(0.5))
                            .font(font.clone()),
                        );
                        ui.add_space(style::SPACE_SM);

                        if !self.quick_note_attachments.is_empty() {
                            ui.label(
                                RichText::new(format!(
                                    "{} image{} attached",
                                    self.quick_note_attachments.len(),
                                    if self.quick_note_attachments.len() == 1 {
                                        ""
                                    } else {
                                        "s"
                                    }
                                ))
                                .color(self.colors.text_dim.linear_multiply(0.7))
                                .font(font.clone()),
                            );
                            ui.add_space(style::SPACE_SM);
                        }

                        crate::ui::text_field::TextArea::multiline(
                            egui::Id::new("quick_note_text"),
                            RichText::new("What's on your mind?").font(font.clone()),
                        )
                        .surface(crate::ui::focus::SurfaceKey::Overlay(
                            crate::app::input_owner::OverlaySurface::Layer(
                                crate::app::FocusKind::QuickNote,
                            ),
                        ))
                        .font(font.clone())
                        .text_color(self.colors.text_primary)
                        .hint_color(self.colors.text_dim.linear_multiply(0.3))
                        .rows(initial_rows)
                        .max_height(max_text_h)
                        .frame(false)
                        .log_name("quick_note")
                        .show(ui, &mut self.quick_note_text, &self.colors);
                    }
                }
            });
        if response.dismissed {
            self.pop_focus_layer(&crate::app::FocusKind::QuickNote);
            self.quick_note_text.clear();
            self.quick_note_attachments.clear();
            log::info!("QuickNote: modal dismissed via click-away");
        }
    }

    /// The configured shared terminal family at the current application font
    /// size. Every QuickNote text surface uses this exact `FontId`.
    pub(crate) fn quick_note_font(&self) -> egui::FontId {
        egui::FontId::monospace(self.default_font_size)
    }

    pub(crate) fn quick_note_handle_key(
        &mut self,
        input: &mut crate::app::input_router::PlexiInput,
    ) -> crate::app::app_trait::KeyDisposition {
        // Consume Cmd+0 so poll_actions doesn't fire OpenQuickNote while the modal
        // is already open — that would reset mid-session note state.
        input.consume_key(egui::Modifiers::COMMAND, egui::Key::Num0);
        crate::app::app_trait::KeyDisposition::Consumed
    }
}

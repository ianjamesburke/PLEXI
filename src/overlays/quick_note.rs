use super::*;

/// Rows the QuickNote editor reserves for an empty or one-line note, so the
/// capture card opens at content height instead of a screen fraction.
const QUICK_NOTE_MIN_ROWS: usize = 3;
/// Row ceiling: the editor grows with the note up to this many rows, then
/// caps and scrolls.
const QUICK_NOTE_MAX_ROWS: usize = 12;
/// Vertical `TextEdit` margin, matching `TextArea`'s default `symmetric(8, 5)`.
const QUICK_NOTE_EDITOR_PAD_V: f32 = 5.0;

/// Editor rows for a note, derived from the note's own line count rather than
/// the screen height.
pub(crate) fn quick_note_editor_rows(text: &str) -> usize {
    text.lines()
        .count()
        .max(1)
        .clamp(QUICK_NOTE_MIN_ROWS, QUICK_NOTE_MAX_ROWS)
}

impl PlexiApp {
    /// Quick note compose phase: full-screen scrim + centered text input.
    pub(crate) fn draw_quick_note_modal(&mut self, ctx: &egui::Context) {
        use crate::ui::style;
        use egui::RichText;

        // Consume Esc to close.
        let esc = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        if esc {
            ctx.input_mut(|i| {
                i.events
                    .retain(|event| !matches!(event, egui::Event::Paste(_)))
            });
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

        // Plain Enter (no shift) → commit to inbox.
        let commit = ctx.input_mut(|i| {
            if !i.modifiers.shift && !i.modifiers.command {
                i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
            } else {
                false
            }
        });
        // Give the editor focus before it reads the event queue. Paste must
        // reach TextEdit in the same order as selection, typing and deletion.
        let editor_id = egui::Id::new("quick_note_text");
        self.focus_overlay_text_for_input(ctx, crate::app::FocusKind::QuickNote, editor_id);

        let screen_rect = ctx.content_rect();

        // Modal — sized to the note's content, not the screen; only a note
        // past the row ceiling gets a capped, scrollable editor.
        let modal_w = (screen_rect.width() * 0.72).clamp(480.0, 864.0);
        let font = self.quick_note_font();
        let colors = self.colors;
        let response = crate::ui::overlay::ModalShell::centered("quick_note_modal")
            .width(modal_w)
            .own_scroll()
            .show(ctx, &colors, |ui| {
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

                    // Inline preview of every staged image, reusing the
                    // editor's shared decoder and strip renderer (0478) so a
                    // preview and the saved note render identically —
                    // including the placeholder for an undecodable file.
                    let max_width = ui.available_width().max(16.0);
                    let base = std::path::PathBuf::from("/");
                    let preview_font = self.quick_note_font();
                    let weak = self.colors.text_dim.linear_multiply(0.5);
                    let attachments = self.quick_note_attachments.clone();
                    for source in &attachments {
                        let dest = source.to_string_lossy().into_owned();
                        let height = crate::editor::widget::inline_image_strip_height(
                            &mut self.quick_note_image_cache,
                            ui.ctx(),
                            &base,
                            &dest,
                            max_width,
                        );
                        let (rect, _) = ui.allocate_exact_size(
                            egui::vec2(max_width, height),
                            egui::Sense::hover(),
                        );
                        crate::editor::widget::paint_inline_image_strip(
                            &mut self.quick_note_image_cache,
                            ui.painter(),
                            &base,
                            &dest,
                            &preview_font,
                            rect,
                            weak,
                        );
                        ui.add_space(style::SPACE_XS);
                    }
                    ui.add_space(style::SPACE_SM);
                }

                let row_height = ui.fonts_mut(|f| f.row_height(&font));
                let editor = crate::ui::text_field::TextArea::multiline(
                    editor_id,
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
                .rows(quick_note_editor_rows(&self.quick_note_text))
                .frame(false)
                .log_name("quick_note");
                let editor = editor.max_height(
                    QUICK_NOTE_MAX_ROWS as f32 * row_height + QUICK_NOTE_EDITOR_PAD_V * 2.0,
                );
                editor.show(ui, &mut self.quick_note_text, &self.colors);

                let hints = [
                    crate::ui::hints::HintGroup::new(&["Enter"], "save"),
                    crate::ui::hints::HintGroup::new(&["Shift", "Enter"], "new line"),
                    crate::ui::hints::HintGroup::new(&["Esc"], "discard"),
                ];
                crate::ui::hints::HintBar::new(&hints).show(ui, &self.colors);
            });
        // TextEdit reads the events without removing them. Keep paste owned
        // by Quick Note even if this pass closes the overlay.
        let paste_count = ctx.input_mut(|i| {
            let before = i.events.len();
            i.events
                .retain(|event| !matches!(event, egui::Event::Paste(_)));
            before - i.events.len()
        });
        if paste_count > 0 {
            log::info!("QuickNote: handled {paste_count} paste event(s)");
        }

        if commit
            && (!self.quick_note_text.trim().is_empty() || !self.quick_note_attachments.is_empty())
        {
            let text = self.quick_note_text.clone();
            if self.commit_quick_note(&text) {
                self.pop_focus_layer(&crate::app::FocusKind::QuickNote);
                self.quick_note_text.clear();
                self.quick_note_attachments.clear();
                log::info!("QuickNote: committed to inbox");
            }
            return;
        }

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
    ) -> crate::app::view_trait::KeyDisposition {
        // Consume the open-quick-note chord so poll_actions doesn't fire
        // OpenQuickNote while the modal is already open — that would reset
        // mid-session note state. Read from the (config-overridable) bindings,
        // never hardcoded, so a rebound chord is still swallowed here.
        let binding = self.key_bindings.open_quick_note;
        input.consume_key(binding.0, binding.1);
        crate::app::view_trait::KeyDisposition::Consumed
    }
}

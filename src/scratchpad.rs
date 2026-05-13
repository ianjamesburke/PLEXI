use chrono::Local;
use std::path::PathBuf;
use crate::app_trait::{App, AppCommand, AppRenderContext};
use crate::style;

#[derive(Debug, PartialEq, Clone, Copy)]
enum Mode {
    Editing,
    SaveModal,
}

pub struct ScratchpadApp {
    workspace_root: PathBuf,
    text: String,
    mode: Mode,
    filename: String,
    focus_requested: bool,
    save_focus_requested: bool,
    wants_close: bool,
    pending_cmds: Vec<AppCommand>,
}

fn default_filename() -> String {
    Local::now().format("%Y-%m-%d_%H%M%S.md").to_string()
}

impl ScratchpadApp {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self {
            workspace_root,
            text: String::new(),
            mode: Mode::Editing,
            filename: default_filename(),
            focus_requested: false,
            save_focus_requested: false,
            wants_close: false,
            pending_cmds: Vec::new(),
        }
    }

    fn notes_dir(&self) -> PathBuf {
        self.workspace_root.join(".plexi").join("notes")
    }

    fn save_file(&mut self) -> bool {
        let notes_dir = self.notes_dir();
        if let Err(e) = std::fs::create_dir_all(&notes_dir) {
            log::error!("scratchpad: failed to create notes dir {:?}: {e}", notes_dir);
            return false;
        }
        let safe_name = std::path::Path::new(&self.filename)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "note.md".to_string());
        let path = notes_dir.join(&safe_name);
        if let Err(e) = std::fs::write(&path, &self.text) {
            log::error!("scratchpad: failed to write {:?}: {e}", path);
            false
        } else {
            log::info!("scratchpad: saved to {:?}", path);
            true
        }
    }
}

impl App for ScratchpadApp {
    fn type_id(&self) -> &'static str {
        "scratchpad"
    }

    fn display_name(&self) -> String {
        "Scratchpad".to_string()
    }

    fn keyboard_capture(&self) -> bool {
        true
    }

    fn wants_close(&self) -> bool {
        self.wants_close
    }

    fn take_pending_commands(&mut self) -> Vec<AppCommand> {
        std::mem::take(&mut self.pending_cmds)
    }

    fn handle_key(&mut self, _input: &egui::InputState) -> bool {
        false
    }

    fn ui(&mut self, ui: &mut egui::Ui, ctx: &AppRenderContext<'_>) {
        let colors = ctx.colors;

        if self.mode == Mode::Editing {
            let save_pressed = ui.input_mut(|i| {
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::S)
            });
            let esc_pressed = ui.input_mut(|i| {
                i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)
            });

            if save_pressed && !self.text.trim().is_empty() {
                self.mode = Mode::SaveModal;
                self.save_focus_requested = false;
                log::info!("scratchpad: Cmd+S — opening save modal");
                return;
            }
            if esc_pressed {
                log::info!("scratchpad: Esc — discarding");
                self.wants_close = true;
                return;
            }

            let status_height = style::TEXT_HINT + style::SPACE_SM * 2.0;
            let available = ui.available_rect_before_wrap();
            let text_rect = egui::Rect::from_min_max(
                available.min,
                egui::pos2(available.max.x, available.max.y - status_height),
            );
            let status_rect = egui::Rect::from_min_max(
                egui::pos2(available.min.x, available.max.y - status_height),
                available.max,
            );

            let te_id = egui::Id::new("scratchpad_text");
            if !self.focus_requested {
                ui.ctx().memory_mut(|m| m.request_focus(te_id));
                self.focus_requested = true;
            }

            ui.allocate_new_ui(egui::UiBuilder::new().max_rect(text_rect), |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.scope(|ui| {
                        ui.visuals_mut().text_cursor.stroke.width = 1.5;
                        ui.visuals_mut().text_cursor.stroke.color = colors.accent;
                        ui.add(
                            egui::TextEdit::multiline(&mut self.text)
                                .id(te_id)
                                .font(egui::FontId::monospace(style::TEXT_BODY))
                                .text_color(colors.text_primary)
                                .desired_width(f32::INFINITY)
                                .frame(false),
                        );
                    });
                });
            });

            // Status bar
            ui.painter().rect_filled(status_rect, 0.0, colors.bg_sidebar);
            ui.allocate_new_ui(egui::UiBuilder::new().max_rect(status_rect), |ui| {
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.add_space(style::SPACE_SM);
                    crate::widgets::key_combo_list(
                        ui,
                        &[&["⌘", "S"], &["Esc"]],
                        Some("save · discard"),
                        colors,
                    );
                });
            });
        } else {
            // Save modal
            let available = ui.available_rect_before_wrap();

            // Dim background
            ui.painter().rect_filled(available, 0.0, egui::Color32::from_black_alpha(200));

            let modal_w = (available.width() * 0.6).min(480.0).max(300.0);
            let esc_pressed = ui.input_mut(|i| {
                i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)
            });
            if esc_pressed {
                self.mode = Mode::Editing;
                return;
            }

            let te_id = egui::Id::new("scratchpad_filename");
            if !self.save_focus_requested {
                ui.ctx().memory_mut(|m| m.request_focus(te_id));
                self.save_focus_requested = true;
            }

            egui::Area::new(egui::Id::new("scratchpad_save_modal"))
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .order(egui::Order::Foreground)
                .show(ui.ctx(), |ui| {
                    egui::Frame::new()
                        .fill(colors.bg_sidebar)
                        .stroke(egui::Stroke::new(1.0, colors.border))
                        .corner_radius(style::RADIUS_MD)
                        .inner_margin(egui::Margin::symmetric(24, 20))
                        .show(ui, |ui| {
                            ui.set_width(modal_w);

                            ui.label(
                                egui::RichText::new("Save as")
                                    .color(colors.text_dim)
                                    .size(style::TEXT_HINT)
                                    .family(egui::FontFamily::Monospace),
                            );
                            ui.add_space(style::SPACE_SM);

                            let enter_pressed = ui.input_mut(|i| {
                                i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
                            });

                            ui.add(
                                egui::TextEdit::singleline(&mut self.filename)
                                    .id(te_id)
                                    .font(egui::FontId::monospace(style::TEXT_BODY))
                                    .text_color(colors.text_primary)
                                    .desired_width(f32::INFINITY),
                            );

                            let dir_display = self.notes_dir().display().to_string();
                            ui.label(
                                egui::RichText::new(dir_display)
                                    .color(colors.text_dim)
                                    .size(style::TEXT_HINT)
                                    .family(egui::FontFamily::Monospace),
                            );

                            if enter_pressed && !self.filename.trim().is_empty() {
                                log::info!("scratchpad: saving as '{}'", self.filename);
                                if self.save_file() {
                                    self.wants_close = true;
                                }
                                return;
                            }

                            ui.add_space(style::SPACE_SM);
                            crate::widgets::key_combo_list(
                                ui,
                                &[&["Enter"], &["Esc"]],
                                Some("save · go back"),
                                colors,
                            );
                        });
                });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scratchpad_initial_state() {
        let app = ScratchpadApp::new(std::path::PathBuf::from("/tmp/test_workspace"));
        assert!(!app.wants_close(), "new scratchpad should not want close");
        assert_eq!(app.type_id(), "scratchpad");
        assert!(app.keyboard_capture(), "scratchpad must capture keyboard");
        assert!(app.text.is_empty(), "text starts empty");
        assert_eq!(app.mode, Mode::Editing);
    }

    #[test]
    fn scratchpad_display_name() {
        let app = ScratchpadApp::new(std::path::PathBuf::from("/tmp"));
        assert_eq!(app.display_name(), "Scratchpad");
    }

    #[test]
    fn scratchpad_pending_commands_empty_initially() {
        let mut app = ScratchpadApp::new(std::path::PathBuf::from("/tmp"));
        let cmds = app.take_pending_commands();
        assert!(cmds.is_empty());
    }
}

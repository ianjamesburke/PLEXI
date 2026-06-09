use crate::app::{FocusLayer, PlexiApp};
use crate::ui::style;

impl PlexiApp {
    pub(crate) fn notes_picker_handle_key(&mut self, ctx: &egui::Context) {
        // Keep the TextEdit from reclaiming egui focus while the picker is open.
        ctx.memory_mut(|m| { if let Some(id) = m.focused() { m.surrender_focus(id); } });

        let count = self.notes_picker_entries.len();
        if count == 0 {
            self.pop_focus_layer(&FocusLayer::NotesPicker);
            return;
        }

        #[derive(Clone, Copy)]
        enum PickerKey { Escape, Down, Up, Enter }

        let action = ctx.input_mut(|i| {
            if i.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
                Some(PickerKey::Escape)
            } else if i.consume_key(egui::Modifiers::NONE, egui::Key::J)
                || i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown)
            {
                Some(PickerKey::Down)
            } else if i.consume_key(egui::Modifiers::NONE, egui::Key::K)
                || i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp)
            {
                Some(PickerKey::Up)
            } else if i.consume_key(egui::Modifiers::NONE, egui::Key::Enter) {
                Some(PickerKey::Enter)
            } else {
                None
            }
        });
        match action {
            Some(PickerKey::Escape) => self.pop_focus_layer(&FocusLayer::NotesPicker),
            Some(PickerKey::Down) => {
                self.notes_picker_selected = (self.notes_picker_selected + 1).min(count - 1);
            }
            Some(PickerKey::Up) => {
                self.notes_picker_selected = self.notes_picker_selected.saturating_sub(1);
            }
            Some(PickerKey::Enter) => self.notes_picker_open_selected(),
            None => {}
        }
    }

    fn notes_picker_open_selected(&mut self) {
        let Some((path, _)) = self.notes_picker_entries.get(self.notes_picker_selected).cloned() else {
            return;
        };
        let active = self.active_window;
        let Some(tile_id) = self.windows[active].focused_pane else {
            self.pop_focus_layer(&FocusLayer::NotesPicker);
            return;
        };
        let pane_id = match self.windows[active].tree.tiles.get(tile_id) {
            Some(egui_tiles::Tile::Pane(id)) => *id,
            _ => { self.pop_focus_layer(&FocusLayer::NotesPicker); return; }
        };
        let state = serde_json::json!({ "path": path.to_string_lossy() });
        if let Some(crate::host::pane::Pane::App(app_pane)) = self.windows[active].panes.get_mut(&pane_id) {
            if let crate::host::pane::AppRuntime::Builtin(app) = &mut app_pane.runtime {
                log::info!("notes_picker: opening {:?} in focused pane", path);
                app.restore_state(&state);
            }
        }
        self.pop_focus_layer(&FocusLayer::NotesPicker);
    }

    pub(crate) fn draw_notes_picker(&mut self, ctx: &egui::Context) {
        let colors = self.colors;
        let entries = self.notes_picker_entries.clone();
        let selected = self.notes_picker_selected;

        let screen = ctx.screen_rect();

        // Scrim — clicking outside the modal dismisses it.
        let scrim_clicked = egui::Area::new(egui::Id::new("notes_picker_scrim"))
            .fixed_pos(screen.min)
            .order(egui::Order::Middle)
            .show(ctx, |ui| {
                ui.painter().rect_filled(screen, 0.0, egui::Color32::from_black_alpha(style::SCRIM_ALPHA));
                ui.allocate_rect(screen, egui::Sense::click()).clicked()
            }).inner;

        // Track which row the user clicked × on (Cell for shared borrow across nested closures).
        let delete_cell = std::cell::Cell::new(None::<usize>);

        egui::Area::new(egui::Id::new("notes_picker"))
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(colors.bg_sidebar)
                    .stroke(egui::Stroke::new(1.0, colors.border))
                    .corner_radius(style::RADIUS_LG)
                    .inner_margin(egui::Margin::symmetric(
                        style::MODAL_PADDING_H,
                        style::MODAL_PADDING_V,
                    ))
                    .show(ui, |ui| {
                        ui.set_width(480.0);

                        ui.label(
                            egui::RichText::new("Notes")
                                .size(style::TEXT_BODY)
                                .color(colors.text_primary),
                        );
                        ui.add_space(style::SPACE_SM);

                        if entries.is_empty() {
                            ui.label(
                                egui::RichText::new("No notes yet. Press \u{2318}+Shift+Space to create one.")
                                    .size(style::TEXT_HINT)
                                    .color(colors.text_dim),
                            );
                            return;
                        }

                        egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                            for (i, (path, preview)) in entries.iter().enumerate() {
                                let is_selected = i == selected;
                                let filename = path
                                    .file_name()
                                    .map(|n| n.to_string_lossy().into_owned())
                                    .unwrap_or_default();

                                let row_fill = if is_selected { colors.bg_active } else { egui::Color32::TRANSPARENT };
                                let (rect, _) = ui.allocate_exact_size(
                                    egui::Vec2::new(ui.available_width(), 28.0),
                                    egui::Sense::hover(),
                                );
                                ui.painter().rect_filled(rect, style::RADIUS_MD, row_fill);

                                let mut row_ui = ui.new_child(egui::UiBuilder::new().max_rect(rect));
                                row_ui.horizontal(|ui| {
                                    ui.add_space(style::SPACE_SM);
                                    ui.label(
                                        egui::RichText::new(&filename)
                                            .size(style::TEXT_HINT)
                                            .color(if is_selected { colors.text_primary } else { colors.text_dim }),
                                    );
                                    if !preview.is_empty() {
                                        let truncated: String = preview.chars().take(50).collect();
                                        ui.add_space(style::SPACE_SM);
                                        ui.scope(|ui| {
                                            ui.set_max_width(160.0);
                                            ui.label(
                                                egui::RichText::new(truncated)
                                                    .size(style::TEXT_HINT)
                                                    .color(colors.text_dim),
                                            );
                                        });
                                    }
                                    // Delete button — right-aligned via RTL sub-layout.
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        ui.add_space(style::SPACE_SM);
                                        let del = ui.add(
                                            egui::Button::new(
                                                egui::RichText::new("×")
                                                    .size(style::TEXT_HINT)
                                                    .color(colors.text_dim),
                                            )
                                            .frame(false),
                                        );
                                        if del.clicked() {
                                            delete_cell.set(Some(i));
                                        }
                                    });
                                });
                            }
                        });

                        ui.add_space(style::SPACE_SM);
                        ui.horizontal(|ui| {
                            crate::ui::widgets::key_combo_list(ui, &[&["j", "k"]], Some("navigate"), &colors);
                            ui.add_space(style::SPACE_SM);
                            crate::ui::widgets::key_combo_list(ui, &[&["\u{21b5}"]], Some("open"), &colors);
                            ui.add_space(style::SPACE_SM);
                            crate::ui::widgets::key_combo_list(ui, &[&["esc"]], Some("dismiss"), &colors);
                        });
                    });
            });

        // Handle delete: remove file and entry from the list.
        if let Some(idx) = delete_cell.get() {
            if let Some((path, _)) = self.notes_picker_entries.get(idx) {
                if let Err(e) = std::fs::remove_file(path) {
                    log::warn!("notes_picker: failed to delete {:?}: {e}", path);
                } else {
                    log::info!("notes_picker: deleted {:?}", path);
                }
            }
            self.notes_picker_entries.remove(idx);
            if self.notes_picker_selected >= self.notes_picker_entries.len() {
                self.notes_picker_selected = self.notes_picker_entries.len().saturating_sub(1);
            }
        }

        // Dismiss on click outside the modal (processed after picker Area so it doesn't
        // fire when clicking inside the modal itself).
        if scrim_clicked {
            self.pop_focus_layer(&FocusLayer::NotesPicker);
        }
    }
}

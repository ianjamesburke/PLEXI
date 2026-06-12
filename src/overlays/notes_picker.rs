use crate::app::text_editor_app::note_path_identity;
use crate::app::{FocusLayer, PlexiApp};
use crate::ui::style;
use crate::ui::{
    hints::{HintBar, HintGroup},
    list::ListRow,
    overlay::ModalShell,
};

impl PlexiApp {
    fn text_editor_path_for_pane(
        window: &crate::host::context::Window,
        pane_id: crate::spatial::tiling::PaneId,
    ) -> Option<std::path::PathBuf> {
        let pane = window.panes.get(&pane_id)?;
        let app_pane = pane.as_app()?;
        if app_pane.runtime.type_id() != "text-editor" {
            return None;
        }
        app_pane.runtime.serialize_state().and_then(|state| {
            state
                .get("path")
                .and_then(|v| v.as_str())
                .map(std::path::PathBuf::from)
                .map(|path| note_path_identity(&path))
        })
    }

    fn find_open_text_editor_tile(
        &self,
        window_idx: usize,
        path: &std::path::Path,
    ) -> Option<(egui_tiles::TileId, crate::spatial::tiling::PaneId)> {
        let window = self.windows.get(window_idx)?;
        let identity = note_path_identity(path);
        window.tree.tiles.iter().find_map(|(tile_id, tile)| {
            let egui_tiles::Tile::Pane(pane_id) = tile else {
                return None;
            };
            (Self::text_editor_path_for_pane(window, *pane_id).as_deref()
                == Some(identity.as_path()))
            .then_some((*tile_id, *pane_id))
        })
    }

    fn notes_picker_delete_entry(&mut self, idx: usize) {
        let Some((path, _)) = self.notes_picker_entries.get(idx).cloned() else {
            return;
        };
        let active = self.active_window;
        if let Some((existing_tile_id, existing_pane_id)) =
            self.find_open_text_editor_tile(active, &path)
        {
            log::info!("notes_picker: refusing to delete open note in pane {existing_pane_id}");
            self.set_window_focused_pane(active, existing_tile_id);
            self.pop_focus_layer(&FocusLayer::NotesPicker);
            return;
        }

        if let Err(e) = std::fs::remove_file(&path) {
            log::warn!("notes_picker: failed to delete {:?}: {e}", path);
        } else {
            log::info!("notes_picker: deleted {:?}", path);
        }
        self.notes_picker_entries.remove(idx);
        if self.notes_picker_selected >= self.notes_picker_entries.len() {
            self.notes_picker_selected = self.notes_picker_entries.len().saturating_sub(1);
        }
    }

    pub(crate) fn notes_picker_handle_key(&mut self, ctx: &egui::Context) {
        // Keep the TextEdit from reclaiming egui focus while the picker is open.
        ctx.memory_mut(|m| {
            if let Some(id) = m.focused() {
                m.surrender_focus(id);
            }
        });

        let count = self.notes_picker_entries.len();
        if count == 0 {
            self.pop_focus_layer(&FocusLayer::NotesPicker);
            return;
        }

        #[derive(Clone, Copy)]
        enum PickerKey {
            Escape,
            Down,
            Up,
            Enter,
            Delete,
            OpenNew,
            Triage,
        }

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
            } else if i.consume_key(egui::Modifiers::NONE, egui::Key::X) {
                Some(PickerKey::Delete)
            } else if i.consume_key(egui::Modifiers::NONE, egui::Key::S) {
                Some(PickerKey::OpenNew)
            } else if i.consume_key(egui::Modifiers::NONE, egui::Key::T) {
                Some(PickerKey::Triage)
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
            Some(PickerKey::Delete) => {
                log::info!(
                    "notes_picker: x key — deleting entry at index {}",
                    self.notes_picker_selected
                );
                self.notes_picker_delete_entry(self.notes_picker_selected);
            }
            Some(PickerKey::OpenNew) => self.notes_picker_open_in_new(),
            Some(PickerKey::Triage) => {
                self.pop_focus_layer(&FocusLayer::NotesPicker);
                if !self.focus_stack.contains(&FocusLayer::NotesTriage) {
                    log::info!("notes_picker: t key — switching to triage");
                    self.open_notes_triage();
                }
            }
            None => {}
        }
    }

    fn notes_picker_open_selected(&mut self) {
        let Some((path, _)) = self
            .notes_picker_entries
            .get(self.notes_picker_selected)
            .cloned()
        else {
            return;
        };
        let active = self.active_window;
        if let Some((existing_tile_id, existing_pane_id)) =
            self.find_open_text_editor_tile(active, &path)
        {
            log::info!("notes_picker: already open in pane {existing_pane_id}, focusing");
            self.set_window_focused_pane(active, existing_tile_id);
            self.pop_focus_layer(&FocusLayer::NotesPicker);
            return;
        }
        let Some(tile_id) = self.windows[active].focused_pane else {
            self.pop_focus_layer(&FocusLayer::NotesPicker);
            return;
        };
        let pane_id = match self.windows[active].tree.tiles.get(tile_id) {
            Some(egui_tiles::Tile::Pane(id)) => *id,
            _ => {
                self.pop_focus_layer(&FocusLayer::NotesPicker);
                return;
            }
        };
        let state = serde_json::json!({ "path": path.to_string_lossy() });
        if let Some(crate::host::pane::Pane::App(app_pane)) =
            self.windows[active].panes.get_mut(&pane_id)
        {
            if let crate::host::pane::AppRuntime::Builtin(app) = &mut app_pane.runtime {
                log::info!("notes_picker: opening {:?} in focused pane", path);
                app.restore_state(&state);
            }
        }
        self.pop_focus_layer(&FocusLayer::NotesPicker);
    }

    fn notes_picker_open_in_new(&mut self) {
        let Some((path, _)) = self
            .notes_picker_entries
            .get(self.notes_picker_selected)
            .cloned()
        else {
            return;
        };
        let path_str = path.display().to_string();
        log::info!("notes_picker: s key — opening {:?} in new text-editor pane", path);
        let _ = self.launch_app_by_id_with_layout("text-editor", None, &[path_str], None);
        self.pop_focus_layer(&FocusLayer::NotesPicker);
    }

    pub(crate) fn draw_notes_picker(&mut self, ctx: &egui::Context) {
        let colors = self.colors;
        let entries = self.notes_picker_entries.clone();
        let selected = self.notes_picker_selected;

        // Track which row the user clicked × on (Cell for shared borrow across nested closures).
        let delete_cell = std::cell::Cell::new(None::<usize>);

        // Snapshot inbox count once — avoid repeated filesystem scans per frame.
        let inbox_notes = crate::notes::scan_inbox();
        let inbox_count = inbox_notes.len();

        let modal_response = ModalShell::centered("notes_picker")
            .title("Notes")
            .width(480.0)
            .escape(true)
            .show(ctx, &colors, |ui| {
                // ── Inbox section ───────────────────────────────────────────────
                if inbox_count > 0 {
                    ui.label(
                        egui::RichText::new(format!("Inbox ({inbox_count})"))
                            .size(style::TEXT_CAPTION)
                            .color(colors.text_dim),
                    );
                    let preview_count = inbox_count.min(5);
                    for note in inbox_notes.iter().take(preview_count) {
                        let preview: String = note.body.trim().chars().take(60).collect();
                        let file_name = note
                            .path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        ui.label(
                            egui::RichText::new(format!("  {file_name}  {preview}"))
                                .size(style::TEXT_HINT)
                                .color(colors.text_dim),
                        );
                    }
                    if inbox_count > 5 {
                        ui.label(
                            egui::RichText::new(format!(
                                "  … and {} more — press t to triage",
                                inbox_count - 5
                            ))
                            .size(style::TEXT_HINT)
                            .color(colors.text_dim),
                        );
                    } else {
                        ui.label(
                            egui::RichText::new("  press t to triage inbox")
                                .size(style::TEXT_HINT)
                                .color(colors.text_dim),
                        );
                    }
                    ui.add_space(style::SPACE_SM);
                }

                if entries.is_empty() {
                    ui.label(
                        egui::RichText::new(
                            "No notes yet. Press \u{2318}+Shift+Space to create one.",
                        )
                        .size(style::TEXT_HINT)
                        .color(colors.text_dim),
                    );
                    return;
                }

                egui::ScrollArea::vertical()
                    .max_height(320.0)
                    .show(ui, |ui| {
                        for (i, (path, preview)) in entries.iter().enumerate() {
                            let is_selected = i == selected;
                            let filename = path
                                .file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_default();
                            let truncated: String = preview.chars().take(50).collect();
                            let row_response = ListRow::new(&filename)
                                .chip("note")
                                .secondary(&truncated)
                                .trailing_action("×")
                                .danger_trailing(true)
                                .selected(is_selected)
                                .show(ui, &colors);
                            if row_response.row_clicked() && !row_response.trailing_clicked() {
                                self.notes_picker_selected = i;
                                self.notes_picker_open_selected();
                            }
                            if row_response.trailing_clicked() {
                                delete_cell.set(Some(i));
                            }
                        }
                    });

                ui.add_space(style::SPACE_SM);
                let hints = [
                    HintGroup::new(&["j", "k"], "navigate"),
                    HintGroup::new(&["\u{21b5}"], "open in place"),
                    HintGroup::new(&["s"], "open new pane"),
                    HintGroup::new(&["t"], "triage inbox"),
                    HintGroup::new(&["x"], "delete"),
                    HintGroup::new(&["esc"], "dismiss"),
                ];
                HintBar::new(&hints).show(ui, &colors);
            });

        // Handle delete: remove file and entry from the list.
        if let Some(idx) = delete_cell.get() {
            self.notes_picker_delete_entry(idx);
        }

        // Dismiss on click outside the modal (processed after picker Area so it doesn't
        // fire when clicking inside the modal itself).
        if modal_response.dismissed {
            self.pop_focus_layer(&FocusLayer::NotesPicker);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::permissions::AppPermissions;
    use crate::host::pane::{AppPane, AppRuntime, Pane};

    fn replace_with_text_editor(app: &mut PlexiApp, pane_id: u64, path: std::path::PathBuf) {
        let app_pane = AppPane {
            id: pane_id,
            runtime: AppRuntime::Builtin(Box::new(
                crate::app::text_editor_app::TextEditorApp::new(path),
            )),
            workspace_root: std::env::temp_dir(),
            permissions: AppPermissions::builtin(),
            manifest_id: "text-editor".to_string(),
            name: "Text Editor".to_string(),
            pane_group: None,
            linked_pane_id: None,
            overlay_replaced: None,
            hidden: false,
            agent: None,
            slots: std::collections::HashMap::new(),
        };

        app.windows[0]
            .panes
            .insert(pane_id, Pane::App(Box::new(app_pane)));
    }

    fn state_path(state: &serde_json::Value) -> String {
        state
            .get("path")
            .and_then(|v| v.as_str())
            .expect("text editor path")
            .to_string()
    }

    #[test]
    fn notes_picker_focuses_existing_text_editor_instead_of_reopening_same_path() {
        let ctx = egui::Context::default();
        let frame_tick = crate::platform::logging::FrameTick::default();
        let (mut app, _tx) = PlexiApp::new_for_test(ctx, frame_tick);

        let note_path = std::env::temp_dir().join(format!(
            "plexi-notes-picker-duplicate-{}.md",
            std::process::id()
        ));
        let other_path = std::env::temp_dir().join(format!(
            "plexi-notes-picker-other-{}.md",
            std::process::id()
        ));

        let (existing_tile, existing_pane) = app.add_test_pane();
        let (focused_tile, focused_pane) = app.add_test_pane();
        replace_with_text_editor(&mut app, existing_pane, note_path.clone());
        replace_with_text_editor(&mut app, focused_pane, other_path.clone());

        app.windows[0].focused_pane = Some(focused_tile);
        app.notes_picker_entries = vec![(note_path.clone(), "note".to_string())];
        app.notes_picker_selected = 0;
        app.push_focus_layer(FocusLayer::NotesPicker);

        app.notes_picker_open_selected();

        assert_eq!(app.windows[0].focused_pane, Some(existing_tile));
        assert_eq!(
            app.find_open_text_editor_tile(0, &note_path),
            Some((existing_tile, existing_pane))
        );

        let focused_state = app.windows[0]
            .panes
            .get(&focused_pane)
            .and_then(|pane| pane.as_app())
            .and_then(|pane| pane.runtime.serialize_state())
            .expect("focused text editor state");
        assert_eq!(
            state_path(&focused_state),
            other_path.to_string_lossy().to_string()
        );
        assert!(!app.focus_stack.contains(&FocusLayer::NotesPicker));
    }

    #[test]
    fn notes_picker_focuses_existing_text_editor_for_alias_path() {
        let ctx = egui::Context::default();
        let frame_tick = crate::platform::logging::FrameTick::default();
        let (mut app, _tx) = PlexiApp::new_for_test(ctx, frame_tick);
        let dir =
            std::env::temp_dir().join(format!("plexi-notes-picker-alias-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let note_path = dir.join("note.md");
        let alias_path = dir.join(".").join("note.md");

        let (existing_tile, existing_pane) = app.add_test_pane();
        let (focused_tile, focused_pane) = app.add_test_pane();
        replace_with_text_editor(&mut app, existing_pane, note_path.clone());
        replace_with_text_editor(&mut app, focused_pane, dir.join("other.md"));

        app.windows[0].focused_pane = Some(focused_tile);
        app.notes_picker_entries = vec![(alias_path, "note".to_string())];
        app.notes_picker_selected = 0;
        app.push_focus_layer(FocusLayer::NotesPicker);

        app.notes_picker_open_selected();

        assert_eq!(app.windows[0].focused_pane, Some(existing_tile));
        assert_eq!(
            app.find_open_text_editor_tile(0, &note_path),
            Some((existing_tile, existing_pane))
        );
        assert!(!app.focus_stack.contains(&FocusLayer::NotesPicker));

        let focused_state = app.windows[0]
            .panes
            .get(&focused_pane)
            .and_then(|pane| pane.as_app())
            .and_then(|pane| pane.runtime.serialize_state())
            .expect("focused text editor state");
        assert!(state_path(&focused_state).ends_with("other.md"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn notes_picker_refuses_to_delete_open_text_editor_note() {
        let ctx = egui::Context::default();
        let frame_tick = crate::platform::logging::FrameTick::default();
        let (mut app, _tx) = PlexiApp::new_for_test(ctx, frame_tick);
        let note_path = std::env::temp_dir().join(format!(
            "plexi-notes-picker-delete-open-{}.md",
            std::process::id()
        ));
        std::fs::write(&note_path, "keep").expect("seed note");

        let (existing_tile, existing_pane) = app.add_test_pane();
        let (focused_tile, _focused_pane) = app.add_test_pane();
        replace_with_text_editor(&mut app, existing_pane, note_path.clone());

        app.windows[0].focused_pane = Some(focused_tile);
        app.notes_picker_entries = vec![(note_path.clone(), "keep".to_string())];
        app.notes_picker_selected = 0;
        app.push_focus_layer(FocusLayer::NotesPicker);

        app.notes_picker_delete_entry(0);

        assert_eq!(app.windows[0].focused_pane, Some(existing_tile));
        assert_eq!(app.notes_picker_entries.len(), 1);
        assert!(note_path.exists());
        assert!(!app.focus_stack.contains(&FocusLayer::NotesPicker));

        let _ = std::fs::remove_file(&note_path);
    }

    #[test]
    fn notes_picker_deletes_closed_note_and_bounds_selection() {
        let ctx = egui::Context::default();
        let frame_tick = crate::platform::logging::FrameTick::default();
        let (mut app, _tx) = PlexiApp::new_for_test(ctx, frame_tick);
        let note_path = std::env::temp_dir().join(format!(
            "plexi-notes-picker-delete-closed-{}.md",
            std::process::id()
        ));
        std::fs::write(&note_path, "delete").expect("seed note");

        app.notes_picker_entries = vec![(note_path.clone(), "delete".to_string())];
        app.notes_picker_selected = 0;

        app.notes_picker_delete_entry(0);

        assert!(app.notes_picker_entries.is_empty());
        assert_eq!(app.notes_picker_selected, 0);
        assert!(!note_path.exists());
    }
}

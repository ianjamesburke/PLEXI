//! Notes picker overlay (Cmd+O).
//!
//! One navigable list: inbox notes (scratch + quick-note captures) on top,
//! kept workspace notes below. Keys:
//!   j/k or arrows — navigate          ↵ — open in focused pane
//!   s — open in new pane
//!   / — fuzzy-find (title, file name, body)
//!   r — rename (frontmatter title)    x — delete
//!   Esc — close (or exit search/rename mode first)

use crate::app::text_editor_app::note_path_identity;
use crate::app::{FocusLayer, PlexiApp};
use crate::ui::style;
use crate::ui::{
    hints::{HintBar, HintGroup},
    list::ListRow,
    overlay::ModalShell,
    text_field::TextField,
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

    pub(crate) fn find_open_text_editor_tile(
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

    /// Indices into `notes_picker_entries` matching the current query, ranked:
    /// title substring, then title fuzzy, then full-text fuzzy. Empty query
    /// returns every entry in stored order (inbox first, then by mtime).
    pub(crate) fn notes_picker_filtered(&self) -> Vec<usize> {
        let query = self.notes_picker_query.trim().to_lowercase();
        if query.is_empty() {
            return (0..self.notes_picker_entries.len()).collect();
        }
        let mut ranked: Vec<(u8, usize)> = self
            .notes_picker_entries
            .iter()
            .enumerate()
            .filter_map(|(i, e)| {
                let title = e.title.to_lowercase();
                if title.contains(&query) {
                    Some((0u8, i))
                } else if crate::notes::fuzzy_match(&query, &title) {
                    Some((1, i))
                } else if crate::notes::fuzzy_match(&query, &e.search_text) {
                    Some((2, i))
                } else {
                    None
                }
            })
            .collect();
        ranked.sort_by_key(|(rank, i)| (*rank, *i));
        ranked.into_iter().map(|(_, i)| i).collect()
    }

    /// Resolve the selected row in the filtered view to an entry index.
    fn notes_picker_selected_entry(&self) -> Option<usize> {
        self.notes_picker_filtered()
            .get(self.notes_picker_selected)
            .copied()
    }

    fn notes_picker_delete_entry(&mut self, entry_idx: usize) {
        let Some(entry) = self.notes_picker_entries.get(entry_idx).cloned() else {
            return;
        };
        let active = self.active_window;
        if let Some((_tile_id, existing_pane_id)) =
            self.find_open_text_editor_tile(active, &entry.path)
        {
            log::info!(
                "notes_picker: refusing to delete open note in pane {existing_pane_id} — close it first"
            );
            return;
        }

        if let Err(e) = std::fs::remove_file(&entry.path) {
            log::warn!("notes_picker: failed to delete {:?}: {e}", entry.path);
        } else {
            log::info!("notes_picker: deleted {:?}", entry.path);
        }
        self.notes_picker_entries.remove(entry_idx);
        let visible = self.notes_picker_filtered().len();
        if self.notes_picker_selected >= visible {
            self.notes_picker_selected = visible.saturating_sub(1);
        }
    }

    pub(crate) fn notes_picker_handle_key(&mut self, ctx: &egui::Context) {
        let visible = self.notes_picker_filtered().len();

        #[derive(Clone, Copy)]
        enum PickerKey {
            Escape,
            Down,
            Up,
            Enter,
            Delete,
            OpenNew,
        }

        let action = ctx.input_mut(|i| {
            if i.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
                Some(PickerKey::Escape)
            } else if i.consume_key(egui::Modifiers::NONE, egui::Key::Enter) {
                Some(PickerKey::Enter)
            } else if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown)
                || i.consume_key(egui::Modifiers::COMMAND, egui::Key::J)
            {
                Some(PickerKey::Down)
            } else if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp)
                || i.consume_key(egui::Modifiers::COMMAND, egui::Key::K)
            {
                Some(PickerKey::Up)
            } else if i.consume_key(egui::Modifiers::COMMAND, egui::Key::S) {
                Some(PickerKey::OpenNew)
            } else if i.consume_key(egui::Modifiers::COMMAND, egui::Key::Backspace) {
                Some(PickerKey::Delete)
            } else {
                None
            }
        });
        match action {
            Some(PickerKey::Escape) => {
                if self.notes_picker_query.is_empty() {
                    self.pop_focus_layer(&FocusLayer::NotesPicker);
                } else {
                    self.notes_picker_query.clear();
                    self.notes_picker_selected = 0;
                }
            }
            Some(PickerKey::Down) => {
                if visible > 0 {
                    self.notes_picker_selected = (self.notes_picker_selected + 1).min(visible - 1);
                }
            }
            Some(PickerKey::Up) => {
                self.notes_picker_selected = self.notes_picker_selected.saturating_sub(1);
            }
            Some(PickerKey::Enter) => self.notes_picker_open_selected(),
            Some(PickerKey::Delete) => {
                if let Some(entry_idx) = self.notes_picker_selected_entry() {
                    log::info!("notes_picker: cmd+backspace — deleting entry {entry_idx}");
                    self.notes_picker_delete_entry(entry_idx);
                }
            }
            Some(PickerKey::OpenNew) => self.notes_picker_open_in_new(),
            None => {}
        }
    }

    fn notes_picker_open_selected(&mut self) {
        let Some(entry_idx) = self.notes_picker_selected_entry() else {
            return;
        };
        let path = self.notes_picker_entries[entry_idx].path.clone();
        let active = self.active_window;
        if let Some((existing_tile_id, existing_pane_id)) =
            self.find_open_text_editor_tile(active, &path)
        {
            log::info!("notes_picker: already open in pane {existing_pane_id}, focusing");
            self.set_window_focused_pane(active, existing_tile_id);
            emit_note_opened(&path);
            self.pop_focus_layer(&FocusLayer::NotesPicker);
            return;
        }
        // Reuse the focused pane only when it's already a text-editor;
        // terminals and other apps get a new pane instead.
        let focused_editor = self.windows[active].focused_pane.and_then(|tile_id| {
            match self.windows[active].tree.tiles.get(tile_id) {
                Some(egui_tiles::Tile::Pane(id)) => {
                    Self::text_editor_path_for_pane(&self.windows[active], *id).map(|_| *id)
                }
                _ => None,
            }
        });
        match focused_editor {
            Some(pane_id) => {
                let state = serde_json::json!({ "path": path.to_string_lossy() });
                if let Some(crate::host::pane::Pane::App(app_pane)) =
                    self.windows[active].panes.get_mut(&pane_id)
                {
                    if let crate::host::pane::AppRuntime::Builtin(app) = &mut app_pane.runtime {
                        log::info!("notes_picker: opening {:?} in focused pane", path);
                        app.restore_state(&state);
                        emit_note_opened(&path);
                    }
                }
                self.pop_focus_layer(&FocusLayer::NotesPicker);
            }
            None => {
                log::info!(
                    "notes_picker: focused pane is not a text-editor — opening {:?} in new pane",
                    path
                );
                self.notes_picker_open_in_new();
            }
        }
    }

    fn notes_picker_open_in_new(&mut self) {
        let Some(entry_idx) = self.notes_picker_selected_entry() else {
            return;
        };
        let path = self.notes_picker_entries[entry_idx].path.clone();
        let path_str = path.display().to_string();
        log::info!("notes_picker: opening {:?} in new text-editor pane", path);
        if self
            .launch_app_by_id_with_layout("text-editor", None, &[path_str], None)
            .is_ok()
        {
            emit_note_opened(&path);
        }
        self.pop_focus_layer(&FocusLayer::NotesPicker);
    }

    pub(crate) fn draw_notes_picker(&mut self, ctx: &egui::Context) {
        let colors = self.colors;
        let filtered = self.notes_picker_filtered();
        let selected = self.notes_picker_selected;
        let total = self.notes_picker_entries.len();
        let inbox_total = self.notes_picker_entries.iter().filter(|e| e.inbox).count();

        let open_cell = std::cell::Cell::new(None::<usize>);
        let prev_selected_cell = std::cell::Cell::new(selected);

        let mut query = self.notes_picker_query.clone();

        let list_max_h = ((ctx.screen_rect().height() - 80.0 - 120.0) * 0.8).max(200.0);

        let modal_response = ModalShell::centered("notes_picker")
            .title("Notes")
            .width(style::MODAL_WIDTH_PALETTE)
            .escape(true)
            .show(ctx, &colors, |ui| {
                let te_id = egui::Id::new("notes_picker_search");
                let te = TextField::singleline(te_id, "Search notes…")
                    .focused(true)
                    .log_name("notes_picker_search")
                    .show(ui, &mut query, &colors);
                if te.changed() {
                    prev_selected_cell.set(usize::MAX); // force scroll reset on query change
                }
                ui.add_space(style::SPACE_SM);

                if total == 0 {
                    ui.label(
                        egui::RichText::new(
                            "No notes yet. Press \u{2318}+Shift+Space to create one.",
                        )
                        .size(style::TEXT_HINT)
                        .color(colors.text_dim),
                    );
                    return;
                }
                if filtered.is_empty() {
                    ui.label(
                        egui::RichText::new("No matching notes.")
                            .size(style::TEXT_HINT)
                            .color(colors.text_dim),
                    );
                    return;
                }

                let mut shown_inbox_header = false;
                let mut shown_kept_header = false;
                egui::ScrollArea::vertical()
                    // animated(false): required by scroll_row_into_view — see src/ui/list.rs.
                    .animated(false)
                    .id_salt("notes_picker_list")
                    .max_height(list_max_h)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.spacing_mut().item_spacing.y = style::SPACE_SM;
                        for (row, &entry_idx) in filtered.iter().enumerate() {
                            let entry = &self.notes_picker_entries[entry_idx];
                            if entry.inbox && !shown_inbox_header {
                                shown_inbox_header = true;
                                ui.label(
                                    egui::RichText::new(format!("Inbox ({inbox_total})"))
                                        .size(style::TEXT_CAPTION)
                                        .color(colors.text_dim),
                                );
                                ui.add_space(style::SPACE_XS);
                            }
                            if !entry.inbox && !shown_kept_header {
                                shown_kept_header = true;
                                if shown_inbox_header {
                                    ui.add_space(style::SPACE_XS);
                                }
                                ui.label(
                                    egui::RichText::new("Notes")
                                        .size(style::TEXT_CAPTION)
                                        .color(colors.text_dim),
                                );
                                ui.add_space(style::SPACE_XS);
                            }

                            let is_selected = row == selected;
                            let secondary = if entry.preview == entry.title {
                                String::new()
                            } else {
                                entry.preview.chars().take(50).collect()
                            };
                            let row_response = ListRow::new(&entry.title)
                                .metadata_chips(if entry.inbox { &["inbox"] } else { &["note"] })
                                .secondary(&secondary)
                                .selected(is_selected)
                                .show(ui, &colors);
                            if is_selected {
                                row_response
                                    .scroll_into_view(ui, selected != prev_selected_cell.get());
                            }
                            if row_response.row_clicked() {
                                open_cell.set(Some(row));
                            }
                        }
                    });

                ui.add_space(style::SPACE_SM);
                HintBar::new(&[
                    HintGroup::new(&["\u{2318}J", "\u{2318}K"], "navigate"),
                    HintGroup::new(&["\u{21b5}"], "open"),
                    HintGroup::new(&["\u{2318}\u{232b}"], "delete"),
                    HintGroup::new(&["esc"], "dismiss"),
                ])
                .show(ui, &colors);
            });

        if query != self.notes_picker_query {
            self.notes_picker_query = query;
            self.notes_picker_selected = 0;
        }

        if let Some(row) = open_cell.get() {
            self.notes_picker_selected = row;
            self.notes_picker_open_selected();
        }

        // Dismiss on click outside the modal (processed after picker Area so it
        // doesn't fire when clicking inside the modal itself).
        if modal_response.dismissed {
            self.pop_focus_layer(&FocusLayer::NotesPicker);
        }
    }
}

fn emit_note_opened(path: &std::path::Path) {
    crate::host::event_log::emit(crate::host::event_log::HostEvent::NoteOpened {
        path: path.display().to_string(),
        timestamp: crate::host::event_log::now_timestamp(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::permissions::AppPermissions;
    use crate::host::pane::{AppPane, AppRuntime, Pane};
    use crate::notes::NotePickerEntry;

    fn picker_entry(path: std::path::PathBuf, preview: &str) -> NotePickerEntry {
        NotePickerEntry {
            title: preview.to_string(),
            preview: preview.to_string(),
            inbox: false,
            search_text: preview.to_lowercase(),
            path,
        }
    }

    fn replace_with_text_editor(app: &mut PlexiApp, pane_id: u64, path: std::path::PathBuf) {
        let app_pane = AppPane {
            pip_status: None,
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
        app.notes_picker_entries = vec![picker_entry(note_path.clone(), "note")];
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
        app.notes_picker_entries = vec![picker_entry(alias_path, "note")];
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
        app.notes_picker_entries = vec![picker_entry(note_path.clone(), "keep")];
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

        app.notes_picker_entries = vec![picker_entry(note_path.clone(), "delete")];
        app.notes_picker_selected = 0;

        app.notes_picker_delete_entry(0);

        assert!(app.notes_picker_entries.is_empty());
        assert_eq!(app.notes_picker_selected, 0);
        assert!(!note_path.exists());
    }

    #[test]
    fn notes_picker_filter_ranks_title_matches_first() {
        let ctx = egui::Context::default();
        let frame_tick = crate::platform::logging::FrameTick::default();
        let (mut app, _tx) = PlexiApp::new_for_test(ctx, frame_tick);

        let mk = |title: &str, body: &str| NotePickerEntry {
            path: std::path::PathBuf::from(format!("/tmp/{title}.md")),
            title: title.to_string(),
            preview: body.to_string(),
            inbox: false,
            search_text: format!("{title} {body}").to_lowercase(),
        };
        app.notes_picker_entries = vec![
            mk("groceries", "milk and eggs"),
            mk("project ideas", "groceries app concept"),
            mk("unrelated", "nothing here"),
        ];

        app.notes_picker_query = "groceries".to_string();
        let filtered = app.notes_picker_filtered();
        assert_eq!(filtered, vec![0, 1]);

        app.notes_picker_query.clear();
        assert_eq!(app.notes_picker_filtered(), vec![0, 1, 2]);
    }
}

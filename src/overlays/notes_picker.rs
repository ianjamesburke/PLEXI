//! Notes picker overlay (Cmd+O).
//!
//! Two halves: on the left a search field over one navigable list grouped by
//! notes tier (the active context's own tier first, then each tier nested below
//! its root, then the global tier); on the right a wrapped, scrollable preview
//! of the selected note's full content. Keys:
//!   arrows / ⌘J ⌘K — navigate         ↵ — open in focused pane
//!   ⌘S — open in new pane              type — fuzzy-find (title, file name, body)
//!   ⌘C — copy the note body            ⌘X — cut (copy body now, delete on confirm)
//!   ⌘⌫ / ⌘Del — delete, after an inline confirm (↵/y confirm, esc/n cancel)
//!   Esc — cancel a pending confirm, else clear the query, else close
//!
//! Deleting a note that is open in a text-editor pane closes that pane first so
//! the editor cannot write the file back, then removes the file; the picker
//! stays open.

use crate::app::text_editor_app::note_path_identity;
use crate::app::{FocusKind, PlexiApp};
use crate::ui::style;
use crate::ui::{
    hints::{HintBar, HintGroup},
    list::ListRow,
    overlay::ModalShell,
    text_field::TextField,
};

/// A delete awaiting inline confirmation in the notes picker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PendingDelete {
    pub(crate) path: std::path::PathBuf,
    /// Started from Cmd+X (content already copied) rather than Cmd+Backspace.
    pub(crate) cut: bool,
}

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

    /// The pane holding `path` in any window, not just the active one. The
    /// command palette's note corpus spans every window, so activating an
    /// entry must be able to reach the pane that owns it — searching only the
    /// active window would open a second editor on an already-open note.
    pub(crate) fn find_open_text_editor_pane_any_window(
        &self,
        path: &std::path::Path,
    ) -> Option<crate::spatial::tiling::PaneId> {
        (0..self.windows.len()).find_map(|idx| {
            self.find_open_text_editor_tile(idx, path)
                .map(|(_, pane)| pane)
        })
    }

    /// Indices into `notes_picker_entries` matching the current query, ranked:
    /// title substring, then title fuzzy, then full-text fuzzy, with
    /// [`crate::search::score_candidate`] breaking ties within each tier.
    /// Empty query returns every entry in stored order (by tier, then by mtime).
    pub(crate) fn notes_picker_filtered(&self) -> Vec<usize> {
        let query = self.notes_picker_query.trim().to_lowercase();
        if query.is_empty() {
            return (0..self.notes_picker_entries.len()).collect();
        }
        let now = std::time::SystemTime::now();
        let mut ranked: Vec<(u8, i64, usize)> = self
            .notes_picker_entries
            .iter()
            .enumerate()
            .filter_map(|(i, e)| {
                let title = e.title.to_lowercase();
                let candidate =
                    crate::search::CandidateInput::new(&e.title).with_secondary(&e.search_text);
                let score = crate::search::score_candidate(&query, &candidate, now)
                    .map(|m| m.total)
                    .unwrap_or(0);
                if title.contains(&query) {
                    Some((0u8, -score, i))
                } else if crate::notes::fuzzy_match(&query, &title) {
                    Some((1, -score, i))
                } else if crate::notes::fuzzy_match(&query, &e.search_text) {
                    Some((2, -score, i))
                } else {
                    None
                }
            })
            .collect();
        ranked.sort_by_key(|(rank, neg_score, i)| (*rank, *neg_score, *i));
        ranked.into_iter().map(|(_, _, i)| i).collect()
    }

    /// Resolve the selected row in the filtered view to an entry index.
    fn notes_picker_selected_entry(&self) -> Option<usize> {
        self.notes_picker_filtered()
            .get(self.notes_picker_selected)
            .copied()
    }

    /// Content of the selected note for the preview half, read once per path
    /// and cached until the selection moves to another note or the note goes.
    pub(crate) fn notes_picker_preview_text(&mut self) -> Option<std::sync::Arc<str>> {
        let path = self
            .notes_picker_selected_entry()
            .map(|idx| self.notes_picker_entries[idx].path.clone())?;
        if let Some((cached, text)) = &self.notes_picker_preview {
            if *cached == path {
                return Some(text.clone());
            }
        }
        let text: std::sync::Arc<str> = match std::fs::read_to_string(&path) {
            // Preview and clipboard share one projection: the note body, with a
            // Plexi capture metadata envelope stripped and user text preserved.
            Ok(text) => crate::notes::note_body(&text).into(),
            Err(e) => {
                log::warn!("notes_picker: preview read failed for {path:?}: {e}");
                format!("Could not read note: {e}").into()
            }
        };
        self.notes_picker_preview = Some((path, text.clone()));
        Some(text)
    }

    /// Dismiss the notes picker overlay and restore focus to the originating pane.
    pub(crate) fn close_notes_picker(&mut self) {
        log::info!("notes_picker: closed");
        self.notes_picker_pending_delete = None;
        self.notes_picker_status = None;
        self.notes_picker_last_drawn_selected = None;
        self.pop_focus_layer(&FocusKind::NotesPicker);
    }

    /// Flush any pending copied note contents to egui's clipboard output.
    pub(crate) fn notes_picker_flush_copy(&mut self, ctx: &egui::Context) {
        if std::mem::take(&mut self.notes_picker_copy_unsent) {
            if let Some(text) = &self.notes_picker_last_copied {
                ctx.copy_text(text.clone());
            }
        }
    }

    /// Copy the selected note's body to the clipboard, stripping a Plexi capture
    /// metadata envelope. Returns the path copied so cut can reuse it.
    fn notes_picker_copy_selected(&mut self) -> Option<std::path::PathBuf> {
        let entry_idx = self.notes_picker_selected_entry()?;
        let path = self.notes_picker_entries[entry_idx].path.clone();
        let title = self.notes_picker_entries[entry_idx].title.clone();
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                let body = crate::notes::note_body(&text).to_string();
                log::info!("notes_picker: copied {} byte(s) from {path:?}", body.len());
                self.notes_picker_last_copied = Some(body);
                self.notes_picker_copy_unsent = true;
                Some(path)
            }
            Err(e) => {
                log::warn!("notes_picker: copy failed for {path:?}: {e}");
                self.notes_picker_status = Some(format!(
                    "Can't copy \u{201c}{title}\u{201d}: {e}."
                ));
                None
            }
        }
    }

    /// Whether the open text-editor pane for `path` (if any) has unsaved edits.
    pub(crate) fn open_text_editor_pane_is_dirty(&self, path: &std::path::Path) -> bool {
        let Some(pane_id) = self.find_open_text_editor_pane_any_window(path) else {
            return false;
        };
        self.windows
            .iter()
            .find_map(|w| w.panes.get(&pane_id))
            .and_then(|p| p.as_app())
            .and_then(|a| a.runtime.editor_buffer())
            .is_some_and(|b| b.dirty)
    }

    fn notes_picker_refuse_if_dirty(
        &mut self,
        path: &std::path::Path,
        title: &str,
        cut: bool,
    ) -> bool {
        if !self.open_text_editor_pane_is_dirty(path) {
            return false;
        }
        let verb = if cut { "cut" } else { "delete" };
        let msg = format!(
            "Can't {verb} \u{201c}{title}\u{201d}: its pane has unsaved edits. Save or close it first."
        );
        log::info!(
            "notes_picker: refusing {verb} of {path:?} — open editor pane has unsaved edits"
        );
        self.notes_picker_status = Some(msg);
        true
    }

    /// Cmd+X: copy now (at press time, so a cancelled cut still leaves the text
    /// on the clipboard), then ask for delete confirmation. Refuses before
    /// copying if the note's open editor pane has unsaved edits.
    fn notes_picker_cut_selected(&mut self) {
        let Some(entry_idx) = self.notes_picker_selected_entry() else {
            return;
        };
        let path = self.notes_picker_entries[entry_idx].path.clone();
        let title = self.notes_picker_entries[entry_idx].title.clone();
        if self.notes_picker_refuse_if_dirty(&path, &title, true) {
            return;
        }
        if let Some(path) = self.notes_picker_copy_selected() {
            log::info!("notes_picker: cut — awaiting delete confirm for {path:?}");
            self.notes_picker_pending_delete = Some(PendingDelete { path, cut: true });
        }
    }

    fn notes_picker_request_delete(&mut self) {
        if let Some(entry_idx) = self.notes_picker_selected_entry() {
            let path = self.notes_picker_entries[entry_idx].path.clone();
            let title = self.notes_picker_entries[entry_idx].title.clone();
            if self.notes_picker_refuse_if_dirty(&path, &title, false) {
                return;
            }
            log::info!("notes_picker: delete requested — awaiting confirm for {path:?}");
            self.notes_picker_pending_delete = Some(PendingDelete { path, cut: false });
        }
    }

    fn notes_picker_confirm_delete(&mut self) {
        let Some(pending) = self.notes_picker_pending_delete.take() else {
            return;
        };
        let Some(entry_idx) = self
            .notes_picker_entries
            .iter()
            .position(|e| e.path == pending.path)
        else {
            return;
        };
        let title = self.notes_picker_entries[entry_idx].title.clone();
        if self.notes_picker_refuse_if_dirty(&pending.path, &title, pending.cut) {
            return;
        }
        log::info!(
            "notes_picker: confirmed delete (cut={}) of {:?}",
            pending.cut,
            pending.path
        );
        let deleted = self.notes_picker_delete_entry(entry_idx);
        if deleted && pending.cut {
            log::info!("notes_picker: confirmed cut complete — closing picker");
            self.close_notes_picker();
        }
    }

    /// Delete the entry's file. A text-editor pane holding the note is closed
    /// first (its drop flushes unsaved edits, so the file must be removed after,
    /// not before, or the editor would recreate it). Returns true on success.
    pub(crate) fn notes_picker_delete_entry(&mut self, entry_idx: usize) -> bool {
        let Some(entry) = self.notes_picker_entries.get(entry_idx).cloned() else {
            return false;
        };
        while let Some(pane_id) = self.find_open_text_editor_pane_any_window(&entry.path) {
            log::info!("notes_picker: closing pane {pane_id} editing the deleted note");
            self.close_pane_by_id(pane_id);
            if self.find_open_text_editor_pane_any_window(&entry.path) == Some(pane_id) {
                log::warn!("notes_picker: pane {pane_id} did not close");
                break;
            }
        }

        let deleted = match std::fs::remove_file(&entry.path) {
            Ok(()) => {
                log::info!("notes_picker: deleted {:?}", entry.path);
                true
            }
            Err(e) => {
                log::warn!("notes_picker: failed to delete {:?}: {e}", entry.path);
                self.notes_picker_status = Some(format!(
                    "Can't delete \u{201c}{}\u{201d}: {e}.",
                    entry.title
                ));
                false
            }
        };
        if deleted {
            self.notes_picker_entries.remove(entry_idx);
            self.notes_picker_preview = None;
            let visible = self.notes_picker_filtered().len();
            if self.notes_picker_selected >= visible {
                self.notes_picker_selected = visible.saturating_sub(1);
            }
            self.notes_picker_shown_paths = self
                .notes_picker_entries
                .iter()
                .map(|entry| entry.path.clone())
                .collect();
            self.invalidate_notes_index();
        }
        deleted
    }

    pub(crate) fn notes_picker_handle_key(
        &mut self,
        input: &mut crate::app::input_router::PlexiInput,
    ) {
        let visible = self.notes_picker_filtered().len();

        if input.events().iter().any(|e| {
            matches!(
                e,
                egui::Event::Key { pressed: true, .. }
                    | egui::Event::Text(_)
                    | egui::Event::Paste(_)
                    | egui::Event::Copy
                    | egui::Event::Cut
            )
        }) {
            self.notes_picker_status = None;
        }

        // A pending confirm owns the keyboard: Esc cancels it (never closes the
        // picker), Enter/y confirm, everything else is ignored.
        if self.notes_picker_pending_delete.is_some() {
            if input.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
                || input.consume_key(egui::Modifiers::NONE, egui::Key::Y)
            {
                self.notes_picker_confirm_delete();
            } else if input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)
                || input.consume_key(egui::Modifiers::NONE, egui::Key::N)
            {
                log::info!("notes_picker: delete cancelled");
                self.notes_picker_pending_delete = None;
            }
            return;
        }

        #[derive(Clone, Copy)]
        enum PickerKey {
            Copy,
            Cut,
            Escape,
            Down,
            Up,
            Enter,
            Delete,
            OpenNew,
        }

        let action = if input.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
            Some(PickerKey::Escape)
        } else if input.consume_key(egui::Modifiers::NONE, egui::Key::Enter) {
            Some(PickerKey::Enter)
        } else if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown)
            || input.consume_key(egui::Modifiers::COMMAND, egui::Key::J)
        {
            Some(PickerKey::Down)
        } else if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp)
            || input.consume_key(egui::Modifiers::COMMAND, egui::Key::K)
        {
            Some(PickerKey::Up)
        } else if input.consume_key(egui::Modifiers::COMMAND, egui::Key::S) {
            Some(PickerKey::OpenNew)
        } else if input.consume_key(egui::Modifiers::COMMAND, egui::Key::Backspace)
            || input.consume_key(egui::Modifiers::COMMAND, egui::Key::Delete)
        {
            Some(PickerKey::Delete)
        } else if input.consume_copy() {
            Some(PickerKey::Copy)
        } else if input.consume_cut() {
            Some(PickerKey::Cut)
        } else {
            None
        };
        match action {
            Some(PickerKey::Escape) => {
                if self.notes_picker_query.is_empty() {
                    self.close_notes_picker();
                } else {
                    self.notes_picker_query.clear();
                    self.notes_picker_selected = 0;
                    self.notes_picker_last_drawn_selected = None;
                }
            }
            Some(PickerKey::Down) => {
                if visible > 0 {
                    let prev = self.notes_picker_selected;
                    self.notes_picker_selected = (self.notes_picker_selected + 1).min(visible - 1);
                    if self.notes_picker_selected != prev {
                        log::info!(
                            "notes_picker: selection down {} -> {}",
                            prev,
                            self.notes_picker_selected
                        );
                    }
                }
            }
            Some(PickerKey::Up) => {
                let prev = self.notes_picker_selected;
                self.notes_picker_selected = self.notes_picker_selected.saturating_sub(1);
                if self.notes_picker_selected != prev {
                    log::info!(
                        "notes_picker: selection up {} -> {}",
                        prev,
                        self.notes_picker_selected
                    );
                }
            }
            Some(PickerKey::Enter) => self.notes_picker_open_selected(),
            Some(PickerKey::Delete) => self.notes_picker_request_delete(),
            Some(PickerKey::Copy) => {
                if self.notes_picker_copy_selected().is_some() {
                    log::info!("notes_picker: copy successful — closing picker");
                    self.close_notes_picker();
                }
            }
            Some(PickerKey::Cut) => self.notes_picker_cut_selected(),
            Some(PickerKey::OpenNew) => self.notes_picker_open_in_new(),
            None => {}
        }
    }

    fn notes_picker_open_selected(&mut self) {
        self.notes_picker_status = None;
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
            emit_note_opened(&path, Some(&self.router.active().root));
            self.close_notes_picker();
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
                        emit_note_opened(&path, Some(&self.router.active().root));
                    }
                }
                self.close_notes_picker();
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
        self.notes_picker_status = None;
        let Some(entry_idx) = self.notes_picker_selected_entry() else {
            return;
        };
        let path = self.notes_picker_entries[entry_idx].path.clone();
        let path_str = path.display().to_string();
        log::info!("notes_picker: opening {:?} in new text-editor pane", path);
        let context_root = self.router.active().root.clone();
        if self
            .launch_app_by_id_with_layout("text-editor", None, &[path_str], None)
            .is_ok()
        {
            emit_note_opened(&path, Some(&context_root));
        }
        self.close_notes_picker();
    }

    pub(crate) fn draw_notes_picker(&mut self, ctx: &egui::Context) {
        let colors = self.colors;
        let filtered = self.notes_picker_filtered();
        let selected = self.notes_picker_selected;
        let selection_changed = self.notes_picker_last_drawn_selected != Some(selected);
        if self
            .notes_picker_last_drawn_selected
            .is_some_and(|prev| prev != selected)
        {
            self.notes_picker_status = None;
        }
        let total = self.notes_picker_entries.len();
        let preview = self.notes_picker_preview_text();
        let pending_delete = self.notes_picker_pending_delete.clone();
        let pending_title = pending_delete.as_ref().map(|p| {
            let title = self
                .notes_picker_entries
                .iter()
                .find(|e| e.path == p.path)
                .map(|e| e.title.clone())
                .unwrap_or_default();
            (
                title,
                self.find_open_text_editor_pane_any_window(&p.path)
                    .is_some(),
            )
        });
        let status = self.notes_picker_status.clone();
        self.notes_picker_flush_copy(ctx);

        let open_cell = std::cell::Cell::new(None::<usize>);

        let mut query = self.notes_picker_query.clone();

        // The list and preview take the usable share of the canvas after the
        // modal's search, status, and shortcut chrome. The earlier nested
        // 0.8 cap applied the margin twice, leaving a dead band at high UI
        // scales and clipping the scroll area's final row.
        let list_max_h = (ctx.content_rect().height() * style::NOTES_PICKER_MODAL_HEIGHT_FRACTION
            - style::NOTES_PICKER_BODY_CHROME_HEIGHT)
            .max(style::LIST_ROW_H * 2.0);

        // Never wider than the window; below the compact modal width the
        // preview half is dropped and the list takes the whole modal.
        let modal_width = style::MODAL_WIDTH_NOTES_PICKER
            .min((ctx.content_rect().width() - style::SPACE_XL * 2.0).max(0.0));
        let show_preview = modal_width >= style::MODAL_WIDTH_MD;

        let modal_response = ModalShell::centered("notes_picker")
            .own_scroll()
            .title("Notes")
            .width(modal_width)
            .escape(true)
            .show(ctx, &colors, |ui| {
                let te_id = egui::Id::new("notes_picker_search");
                TextField::singleline(te_id, "Search notes…")
                    .surface(crate::ui::focus::SurfaceKey::Overlay(
                        crate::app::input_owner::OverlaySurface::Layer(
                            crate::app::FocusKind::NotesPicker,
                        ),
                    ))
                    .log_name("notes_picker_search")
                    .show(ui, &mut query, &colors);
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

                let list_width = if show_preview {
                    ((ui.available_width() - style::SPACE_MD) * style::NOTES_PICKER_LIST_FRACTION).floor()
                } else {
                    ui.available_width()
                };
                let preview_width = ui.available_width() - list_width - style::SPACE_MD;
                let halves = egui::UiBuilder::new()
                    .id_salt("notes_picker_halves")
                    .layout(egui::Layout::left_to_right(egui::Align::Min));
                ui.scope_builder(halves, |ui| {
                    // A modal Area offers unbounded height; without this cap the
                    // vertical separator and preview stretch past the window.
                    ui.set_max_height(list_max_h);
                    ui.spacing_mut().item_spacing.x = style::SPACE_MD;
                    let list_half = egui::UiBuilder::new()
                        .id_salt("notes_picker_list_half")
                        .layout(egui::Layout::top_down(egui::Align::Min));
                    ui.scope_builder(list_half, |ui| {
                        ui.set_width(list_width);
                        // Tier headers: entries arrive grouped by tier, so a header is
                        // emitted whenever the tier label changes from the previous row.
                        let mut current_tier: Option<Option<String>> = None;
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
                                    // Header per tier group. The primary tier gets none —
                                    // the modal title already names it, and `None` means
                                    // "needs no disambiguation" by construction. Rolled-up
                                    // tiers and the global tier each get one.
                                    if current_tier.as_ref() != Some(&entry.tier_label) {
                                        current_tier = Some(entry.tier_label.clone());
                                        if let Some(label) = entry.tier_label.as_deref() {
                                            ui.add_space(style::SPACE_XS);
                                            ui.label(
                                                egui::RichText::new(label)
                                                    .size(style::TEXT_CAPTION)
                                                    .color(colors.text_dim),
                                            );
                                            ui.add_space(style::SPACE_XS);
                                        }
                                    }

                                    let is_selected = row == selected;
                                    let secondary = if entry.preview == entry.title {
                                        String::new()
                                    } else {
                                        entry.preview.chars().take(50).collect()
                                    };
                                    // Notes rolled up from a nested tier, or from the
                                    // global tier, carry where they came from so a root
                                    // "master view" stays legible.
                                    let chip: &str = entry.tier_label.as_deref().unwrap_or("note");
                                    let row_response = ListRow::new(&entry.title)
                                        .metadata_chips(std::slice::from_ref(&chip))
                                        .secondary(&secondary)
                                        .selected(is_selected)
                                        .show(ui, &colors);
                                    if is_selected {
                                        row_response.scroll_into_view(ui, selection_changed);
                                    }
                                    if row_response.row_clicked() {
                                        open_cell.set(Some(row));
                                    }
                                }
                            });
                    });
                    if show_preview {
                        let preview_half = egui::UiBuilder::new()
                            .id_salt("notes_picker_preview_half")
                            .layout(egui::Layout::top_down(egui::Align::Min));
                        ui.scope_builder(preview_half, |ui| {
                            ui.set_width(preview_width);
                            let font_id = egui::FontId::monospace(self.default_font_size);
                            egui::Frame::new()
                                .fill(colors.bg_active)
                                .stroke(style::hairline_border(colors.border))
                                .corner_radius(style::RADIUS_MD)
                                .inner_margin(egui::Margin::symmetric(12, 10))
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.set_min_height((list_max_h - 20.0).max(60.0));
                                    egui::ScrollArea::vertical()
                                        .id_salt("notes_picker_preview")
                                        .max_height((list_max_h - 20.0).max(60.0))
                                        .auto_shrink([false, false])
                                        .show(ui, |ui| {
                                            ui.set_width(ui.available_width());
                                            let body_text = preview.as_deref().unwrap_or("");
                                            if body_text.trim().is_empty() {
                                                ui.label(
                                                    egui::RichText::new("Empty note")
                                                        .font(font_id.clone())
                                                        .color(colors.text_dim),
                                                );
                                            } else {
                                                ui.add(
                                                    egui::Label::new(
                                                        egui::RichText::new(body_text)
                                                            .font(font_id.clone())
                                                            .color(colors.text_primary),
                                                    )
                                                    .wrap(),
                                                );
                                            }
                                        });
                                });
                        });
                    }
                });

                ui.add_space(style::SPACE_SM);
                let status_message = if let Some((title, open_in_pane)) = &pending_title {
                    let cut = pending_delete.as_ref().is_some_and(|p| p.cut);
                    let verb = if cut { "Cut" } else { "Delete" };
                    let tail = if *open_in_pane {
                        " and close its pane"
                    } else {
                        ""
                    };
                    Some(format!(
                        "{verb} \u{201c}{title}\u{201d}{tail}? \u{21b5}/y confirm \u{b7} esc/n cancel"
                    ))
                } else {
                    status.clone()
                };
                let (status_rect, _) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), style::NOTES_PICKER_STATUS_HEIGHT),
                    egui::Sense::hover(),
                );
                if let Some(msg) = status_message {
                    let mut status_ui = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(status_rect)
                            .layout(egui::Layout::top_down(egui::Align::Min)),
                    );
                    status_ui.set_clip_rect(status_rect.intersect(ui.clip_rect()));
                    let mut job = egui::text::LayoutJob::single_section(
                        msg,
                        egui::TextFormat {
                            font_id: egui::FontId::proportional(style::TEXT_CAPTION),
                            color: colors.danger,
                            ..Default::default()
                        },
                    );
                    job.wrap = egui::text::TextWrapping {
                        max_width: status_rect.width(),
                        max_rows: 2,
                        break_anywhere: false,
                        overflow_character: Some('\u{2026}'),
                    };
                    status_ui.add(egui::Label::new(job));
                }
                ui.add_space(style::SPACE_XS);
                HintBar::new(&[
                    HintGroup::new(&["\u{2191}", "\u{2193}"], "navigate"),
                    HintGroup::new(&["\u{21b5}"], "open"),
                    HintGroup::new(&["\u{2318}C"], "copy"),
                    HintGroup::new(&["\u{2318}X"], "cut"),
                    HintGroup::new(&["\u{2318}\u{232b}"], "delete"),
                    HintGroup::new(&["esc"], "close"),
                ])
                .show(ui, &colors);
            });

        // The y/n confirm keys also reach the search field as text; drop that.
        let query_changed = self.notes_picker_apply_query(query);

        if let Some(row) = open_cell.get() {
            self.notes_picker_selected = row;
            self.notes_picker_status = None;
            self.notes_picker_open_selected();
        }

        // A query can reset the selection after its old row was rendered. Keep
        // the reset through the next frame so the selected first result is
        // scrolled into view instead of inheriting a deliberate wheel offset
        // from the prior result set.
        if !query_changed {
            self.notes_picker_last_drawn_selected = Some(selected);
        }

        // Dismiss on click outside the modal (processed after picker Area so it
        // doesn't fire when clicking inside the modal itself).
        if modal_response.dismissed {
            self.close_notes_picker();
        }
    }

    /// Applies a search query after its frame has painted. Reset the selection
    /// scroll marker with it so the next frame cannot retain a wheel offset
    /// belonging to the previous result set.
    fn notes_picker_apply_query(&mut self, query: String) -> bool {
        if query == self.notes_picker_query || self.notes_picker_pending_delete.is_some() {
            return false;
        }
        self.notes_picker_query = query;
        self.notes_picker_selected = 0;
        self.notes_picker_last_drawn_selected = None;
        self.notes_picker_status = None;
        true
    }
}

fn emit_note_opened(path: &std::path::Path, context_root: Option<&std::path::Path>) {
    crate::host::event_log::emit_scoped(
        crate::host::event_log::HostEvent::NoteOpened {
            path: path.display().to_string(),
            timestamp: crate::host::event_log::now_timestamp(),
        },
        context_root,
    );
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
            search_text: preview.to_lowercase(),
            path,
            tier_label: None,
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
            semantic_state: Default::default(),
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

    fn picker_frame_input() -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(640.0, 480.0),
            )),
            ..Default::default()
        }
    }

    fn draw_picker_frame(app: &mut PlexiApp, ctx: &egui::Context) {
        let _ = ctx.run_ui(picker_frame_input(), |ui| app.draw_notes_picker(ui.ctx()));
    }

    fn press_picker_key(app: &mut PlexiApp, ctx: &egui::Context, key: egui::Key) {
        let mut input = picker_frame_input();
        input.events.push(egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        });
        let _ = ctx.run_ui(input, |ui| {
            let mut input = crate::app::input_router::PlexiInput::take_from(ui.ctx());
            app.notes_picker_handle_key(&mut input);
            app.notes_picker_flush_copy(ui.ctx());
        });
    }

    fn picker_scroll_offset(ctx: &egui::Context) -> f32 {
        let id = egui::Id::new("notes_picker")
            .with("overlay")
            .with(egui::Id::from("child"))
            .with(egui::Id::new("notes_picker_halves"))
            .with(egui::Id::new("notes_picker_list_half"))
            .with(egui::Id::new("notes_picker_list"));
        egui::scroll_area::State::load(ctx, id)
            .expect("notes picker scroll state")
            .offset
            .y
    }

    fn set_picker_scroll_offset(ctx: &egui::Context, offset: f32) {
        let id = egui::Id::new("notes_picker")
            .with("overlay")
            .with(egui::Id::from("child"))
            .with(egui::Id::new("notes_picker_halves"))
            .with(egui::Id::new("notes_picker_list_half"))
            .with(egui::Id::new("notes_picker_list"));
        let mut state =
            egui::scroll_area::State::load(ctx, id).expect("notes picker scroll state");
        state.offset.y = offset;
        state.store(ctx, id);
    }

    #[test]
    fn notes_picker_scroll_follows_keyboard_selection_in_both_directions() {
        let ctx = egui::Context::default();
        crate::ui::theme::setup_fonts(&ctx);
        let frame_tick = crate::platform::logging::FrameTick::default();
        let (mut app, _tx) = PlexiApp::new_for_test(ctx.clone(), frame_tick);
        app.notes_picker_entries = (0..30)
            .map(|i| picker_entry(format!("/tmp/note-{i}.md").into(), &format!("note {i}")))
            .collect();
        app.push_focus_layer(FocusKind::NotesPicker);

        draw_picker_frame(&mut app, &ctx);
        for _ in 0..18 {
            press_picker_key(&mut app, &ctx, egui::Key::ArrowDown);
        }
        draw_picker_frame(&mut app, &ctx);
        let offset_after_down = picker_scroll_offset(&ctx);
        assert_eq!(app.notes_picker_selected, 18);
        assert!(
            offset_after_down > 0.0,
            "selection past the visible window must scroll down"
        );

        for _ in 0..15 {
            press_picker_key(&mut app, &ctx, egui::Key::ArrowUp);
        }
        draw_picker_frame(&mut app, &ctx);
        let offset_after_up = picker_scroll_offset(&ctx);
        assert_eq!(app.notes_picker_selected, 3);
        assert!(
            offset_after_up < offset_after_down,
            "selection above the visible window must scroll back up"
        );
    }

    #[test]
    fn notes_picker_query_reset_keeps_first_result_visible_after_wheel_scroll() {
        let ctx = egui::Context::default();
        crate::ui::theme::setup_fonts(&ctx);
        let frame_tick = crate::platform::logging::FrameTick::default();
        let (mut app, _tx) = PlexiApp::new_for_test(ctx.clone(), frame_tick);
        app.notes_picker_entries = (0..30)
            .map(|i| picker_entry(format!("/tmp/note-{i}.md").into(), &format!("note {i}")))
            .collect();
        app.push_focus_layer(FocusKind::NotesPicker);

        draw_picker_frame(&mut app, &ctx);
        set_picker_scroll_offset(&ctx, 480.0);
        // Query handling happens after the list has been drawn for the frame.
        // The reset must survive into the next draw, otherwise the selected
        // first result retains a wheel offset from the prior result set.
        assert!(app.notes_picker_apply_query("note".into()));
        draw_picker_frame(&mut app, &ctx);

        assert!(
            picker_scroll_offset(&ctx) < 480.0,
            "a new query must restore its selected first result instead of preserving an old wheel offset"
        );
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
        app.push_focus_layer(FocusKind::NotesPicker);

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
        assert!(!app.focus_stack.contains(&FocusKind::NotesPicker));
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
        app.push_focus_layer(FocusKind::NotesPicker);

        app.notes_picker_open_selected();

        assert_eq!(app.windows[0].focused_pane, Some(existing_tile));
        assert_eq!(
            app.find_open_text_editor_tile(0, &note_path),
            Some((existing_tile, existing_pane))
        );
        assert!(!app.focus_stack.contains(&FocusKind::NotesPicker));

        let focused_state = app.windows[0]
            .panes
            .get(&focused_pane)
            .and_then(|pane| pane.as_app())
            .and_then(|pane| pane.runtime.serialize_state())
            .expect("focused text editor state");
        assert!(state_path(&focused_state).ends_with("other.md"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Rollup end to end through `open_notes_picker`, on real tier directories:
    /// a note in a nested tier shows in the nested project's picker *and* in the
    /// enclosing one, an enclosing project's note never leaks down, a
    /// textual-prefix sibling is never aggregated, and the global tier is visible
    /// from everywhere.
    ///
    /// Rollup is now a filesystem property, so this seeds `<root>/.plexi/notes/`
    /// dirs rather than registering descendant contexts — a nested tier rolls up
    /// whether or not a live context is anchored at it.
    #[test]
    fn notes_picker_rolls_up_nested_tiers_and_the_global_tier() {
        let ctx = egui::Context::default();
        let frame_tick = crate::platform::logging::FrameTick::default();
        let (mut app, _tx) = PlexiApp::new_for_test(ctx, frame_tick);

        let base = std::env::temp_dir().join(format!(
            "plexi-notes-tier-picker-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&base);
        let profile = base.join("profile");
        let shared = base.join("shared");
        std::fs::create_dir_all(&profile).expect("profile");
        std::fs::create_dir_all(&shared).expect("shared");
        let _profile_guard = crate::config::set_test_profile_dir(profile);
        let _shared_guard = crate::config::set_test_shared_dir(shared.clone());

        let parent_root = base.join("proj");
        let child_root = parent_root.join("sub");
        // Textual-prefix sibling that must never be aggregated.
        let sibling_root = base.join("projx");

        let seed = |dir: std::path::PathBuf, title: &str| {
            std::fs::create_dir_all(&dir).expect("tier dir");
            std::fs::write(dir.join(format!("{title}.md")), format!("body of {title}"))
                .expect("seed note");
        };
        seed(crate::notes::context_notes_dir(&parent_root), "parent-note");
        seed(crate::notes::context_notes_dir(&child_root), "child-note");
        seed(
            crate::notes::context_notes_dir(&sibling_root),
            "sibling-note",
        );
        seed(crate::notes::global_notes_dir(), "global-note");

        let mk_ctx = |id: u64, name: &str, root: &std::path::Path| crate::host::context::Context {
            name: name.to_string().into(),
            root: root.to_path_buf(),
            description: None,
            context_id: id,
            parent_id: None,
            depth: 0,
            parked: false,
        };
        // Slot 0 is the constructor's own context; append the two under test.
        app.router.push(mk_ctx(10, "proj", &parent_root));
        app.router.push(mk_ctx(11, "sub", &child_root));
        let parent_idx = app.router.position(|c| c.context_id == 10).expect("parent");
        let child_idx = app.router.position(|c| c.context_id == 11).expect("child");

        let titles = |app: &PlexiApp| -> Vec<String> {
            app.notes_picker_entries
                .iter()
                .map(|e| e.title.clone())
                .collect()
        };

        app.router.set_active(parent_idx);
        app.open_notes_picker();
        app.finish_notes_index_refresh_for_test();
        let from_parent = titles(&app);
        assert!(
            from_parent.contains(&"body of parent-note".to_string()),
            "the enclosing project sees its own tier: {from_parent:?}"
        );
        assert!(
            from_parent.contains(&"body of child-note".to_string()),
            "a nested tier rolls up: {from_parent:?}"
        );
        assert!(
            !from_parent.contains(&"body of sibling-note".to_string()),
            "projx is a textual prefix, not nested under proj: {from_parent:?}"
        );
        assert!(
            from_parent.contains(&"body of global-note".to_string()),
            "the global tier is visible from every context: {from_parent:?}"
        );
        assert_eq!(
            app.notes_picker_entries
                .iter()
                .find(|e| e.title == "body of child-note")
                .and_then(|e| e.tier_label.clone()),
            Some("sub".to_string()),
            "nested rows carry their relative path for the master view"
        );

        app.router.set_active(child_idx);
        app.open_notes_picker();
        app.finish_notes_index_refresh_for_test();
        let from_child = titles(&app);
        assert!(from_child.contains(&"body of child-note".to_string()));
        assert!(
            !from_child.contains(&"body of parent-note".to_string()),
            "no walk upward into the enclosing tier: {from_child:?}"
        );
        assert!(
            from_child.contains(&"body of global-note".to_string()),
            "the global tier is still visible: {from_child:?}"
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    /// Cmd+O with a warm index returns before any directory walk. The refresh
    /// still runs, on another thread; this thread's walk counter stays put.
    #[test]
    fn notes_picker_open_with_warm_cache_does_not_walk() {
        let ctx = egui::Context::default();
        let frame_tick = crate::platform::logging::FrameTick::default();
        let (mut app, _tx) = PlexiApp::new_for_test(ctx, frame_tick);

        let base = std::env::temp_dir().join(format!(
            "plexi-notes-open-cache-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&base);
        let profile = base.join("profile");
        let shared = base.join("shared");
        std::fs::create_dir_all(&profile).expect("profile");
        std::fs::create_dir_all(&shared).expect("shared");
        let _profile_guard = crate::config::set_test_profile_dir(profile);
        let _shared_guard = crate::config::set_test_shared_dir(shared.clone());

        let root = base.join("proj");
        std::fs::create_dir_all(&root).expect("root");
        let tier = crate::notes::context_notes_dir(&root);
        std::fs::create_dir_all(&tier).expect("tier");
        std::fs::write(
            tier.join("cached.md"),
            "---\ntitle: \"Cached\"\n---\nbody\n",
        )
        .expect("seed");
        std::fs::create_dir_all(shared.join("notes")).expect("global tier");

        let active_idx = app.router.active_idx();
        app.router.get_mut(active_idx).root = root.clone();
        let query = crate::notes::NotesDiscovery {
            active_root: Some(root.clone()),
            known_roots: vec![root.clone()],
            global_notes: crate::notes::global_notes_dir(),
            home: Some(base.clone()),
            cached_tiers: Vec::new(),
        };
        let snapshot = crate::notes::build_notes_index(&query);
        let generation = app.notes_index.generation();
        assert!(app.notes_index.store(&root, snapshot, generation));
        app.notes_index_loaded = true;

        crate::notes::reset_notes_walk_dirs_for_test();
        let started = std::time::Instant::now();
        app.open_notes_picker();
        let open_ms = started.elapsed().as_secs_f64() * 1000.0;
        assert_eq!(
            crate::notes::notes_walk_dirs(),
            0,
            "Cmd+O must publish the cache without walking"
        );
        assert!(
            app.notes_picker_entries
                .iter()
                .any(|entry| entry.title == "Cached"),
            "the warm cache is what the picker shows"
        );
        eprintln!("picker_open_ms={open_ms:.3} walk_dirs=0");

        app.finish_notes_index_refresh_for_test();
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn notes_picker_open_selected_lands_caret_at_document_end() {
        let ctx = egui::Context::default();
        let frame_tick = crate::platform::logging::FrameTick::default();
        let (mut app, _tx) = PlexiApp::new_for_test(ctx, frame_tick);

        let profile = std::env::temp_dir().join(format!(
            "plexi-notes-picker-caret-end-{}",
            std::process::id()
        ));
        let _guard = crate::config::set_test_profile_dir(profile.clone());
        let _shared = crate::config::set_test_shared_dir(profile.join("shared"));
        // Must be a real tier: `is_note` gates caret-at-end, and only a note in a
        // tier is a note.
        let notes_dir = crate::notes::global_notes_dir();
        std::fs::create_dir_all(&notes_dir).expect("create notes dir");
        let note_path = notes_dir.join("note.md");
        let note_body = "first line\nsecond line\nthird line";
        std::fs::write(&note_path, note_body).expect("seed note");
        let other_path = notes_dir.join("other.md");
        std::fs::write(&other_path, "unrelated").expect("seed other note");

        let (_existing_tile, _existing_pane) = app.add_test_pane();
        let (focused_tile, focused_pane) = app.add_test_pane();
        replace_with_text_editor(&mut app, focused_pane, other_path);

        app.windows[0].focused_pane = Some(focused_tile);
        app.notes_picker_entries = vec![picker_entry(note_path.clone(), "note")];
        app.notes_picker_selected = 0;
        app.push_focus_layer(FocusKind::NotesPicker);

        app.notes_picker_open_selected();

        let semantic = app.windows[0]
            .panes
            .get(&focused_pane)
            .and_then(|pane| pane.as_app())
            .and_then(|pane| pane.runtime.semantic_details())
            .expect("focused text editor semantic state");
        assert_eq!(
            semantic["caret"].as_u64(),
            Some(note_body.len() as u64),
            "caret should land at doc end, not offset 0"
        );
        // No viewport has rendered yet. The scroll target must stay in source
        // coordinates until wrapping is known; the host-level wrapped-note
        // regression checks visibility after the first rendered frames.
        let runtime = &mut app.windows[0]
            .panes
            .get_mut(&focused_pane)
            .expect("focused pane")
            .as_app_mut()
            .expect("editor app")
            .runtime;
        let AppRuntime::Builtin(editor) = runtime else {
            panic!("expected builtin editor")
        };
        let editor = editor
            .as_any_mut()
            .downcast_mut::<crate::app::text_editor_app::TextEditorApp>()
            .expect("text editor");
        assert_eq!(
            editor.test_view().pending_reveal,
            Some(crate::editor::cursor::Cursor::new(2, "third line".len()))
        );

        let _ = std::fs::remove_dir_all(&profile);
    }

    /// Dispatch one picker chord and return the frame output, so a test can read
    /// the real egui clipboard command rather than only intermediate state.
    fn press_picker_chord(
        app: &mut PlexiApp,
        ctx: &egui::Context,
        modifiers: egui::Modifiers,
        key: egui::Key,
    ) -> egui::FullOutput {
        let mut input = picker_frame_input();
        input.events.push(egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        });
        ctx.run_ui(input, |ui| {
            let mut input = crate::app::input_router::PlexiInput::take_from(ui.ctx());
            app.notes_picker_handle_key(&mut input);
            app.notes_picker_flush_copy(ui.ctx());
        })
    }

    /// The text egui was actually handed for the clipboard during a frame.
    fn clipboard_text(output: &egui::FullOutput) -> Option<String> {
        output
            .platform_output
            .commands
            .iter()
            .find_map(|command| match command {
                egui::OutputCommand::CopyText(text) => Some(text.clone()),
                _ => None,
            })
    }

    fn picker_with_notes(
        tag: &str,
        bodies: &[&str],
    ) -> (PlexiApp, egui::Context, Vec<std::path::PathBuf>) {
        let ctx = egui::Context::default();
        crate::ui::theme::setup_fonts(&ctx);
        let frame_tick = crate::platform::logging::FrameTick::default();
        let (mut app, _tx) = PlexiApp::new_for_test(ctx.clone(), frame_tick);
        let paths: Vec<std::path::PathBuf> = bodies
            .iter()
            .enumerate()
            .map(|(i, body)| {
                let path = std::env::temp_dir().join(format!(
                    "plexi-notes-picker-{tag}-{}-{i}.md",
                    std::process::id()
                ));
                std::fs::write(&path, body).expect("seed note");
                path
            })
            .collect();
        app.notes_picker_entries = paths
            .iter()
            .zip(bodies)
            .map(|(p, b)| picker_entry(p.clone(), b))
            .collect();
        app.notes_picker_selected = 0;
        app.push_focus_layer(FocusKind::NotesPicker);
        (app, ctx, paths)
    }

    #[test]
    fn notes_picker_preview_shows_selected_content_and_follows_selection() {
        let (mut app, ctx, paths) = picker_with_notes("preview", &["alpha body", "beta body"]);
        assert_eq!(
            app.notes_picker_preview_text().as_deref(),
            Some("alpha body")
        );
        draw_picker_frame(&mut app, &ctx);
        press_picker_key(&mut app, &ctx, egui::Key::ArrowDown);
        assert_eq!(
            app.notes_picker_preview_text().as_deref(),
            Some("beta body")
        );
        for p in paths {
            let _ = std::fs::remove_file(p);
        }
    }

    /// Baseline regression: a real saved capture's clipboard payload is exactly
    /// its parsed body. The assertion reads the egui clipboard command, and
    /// reverting the body projection in `notes_picker_copy_selected` makes it
    /// fail because the envelope returns to the payload.
    #[test]
    fn notes_picker_preview_and_copy_share_the_note_body_without_metadata() {
        // Written the way `write_note` writes a capture.
        let body = "\n# Heading\n\nbody line 1\nbody line 2\n";
        let full = format!(
            "---\ntitle: \"\"\ncaptured_at: \"2026-10-26T22:00:00Z\"\nsource: \"scratchpad\"\ncwd: \"/tmp\"\n---\n{body}"
        );
        let (mut app, ctx, paths) = picker_with_notes("frontmatter", &[full.as_str()]);
        let before = std::fs::read(&paths[0]).expect("read note before copy");
        assert_eq!(app.notes_picker_preview_text().as_deref(), Some(body));

        let output = press_picker_chord(&mut app, &ctx, egui::Modifiers::COMMAND, egui::Key::C);
        assert_eq!(
            clipboard_text(&output).as_deref(),
            Some(body),
            "clipboard must carry the body, not storage metadata"
        );
        assert_eq!(app.notes_picker_last_copied.as_deref(), Some(body));
        assert_eq!(
            std::fs::read(&paths[0]).expect("read note after copy"),
            before,
            "copy must not modify the backing file"
        );
        assert!(
            !app.focus_stack.contains(&FocusKind::NotesPicker),
            "successful copy must close the picker"
        );
        for p in paths {
            let _ = std::fs::remove_file(p);
        }
    }

    /// A delimiter block the user typed is not storage metadata; the clipboard
    /// payload keeps it, along with fenced code and line breaks.
    #[test]
    fn notes_picker_copy_preserves_user_delimiter_blocks_and_markdown() {
        let content = "---\ntags: [a, b]\n---\n\ntop line\n\n```sh\necho hi\n```\n";
        let ctx = egui::Context::default();
        crate::ui::theme::setup_fonts(&ctx);
        let frame_tick = crate::platform::logging::FrameTick::default();
        let (mut app, _tx) = PlexiApp::new_for_test(ctx.clone(), frame_tick);
        let path = std::env::temp_dir().join(format!(
            "plexi-notes-picker-user-delimiter-{}.md",
            std::process::id()
        ));
        std::fs::write(&path, content).expect("seed note");
        app.notes_picker_entries = vec![picker_entry(path.clone(), content)];
        app.notes_picker_selected = 0;
        app.push_focus_layer(FocusKind::NotesPicker);

        let output = press_picker_chord(&mut app, &ctx, egui::Modifiers::COMMAND, egui::Key::C);
        assert_eq!(clipboard_text(&output).as_deref(), Some(content));
        assert_eq!(app.notes_picker_last_copied.as_deref(), Some(content));
        let _ = std::fs::remove_file(&path);
    }

    /// Copy closes the picker by popping its layer, so whatever owned focus
    /// before the picker opened owns it again.
    #[test]
    fn notes_picker_copy_restores_the_layer_under_it() {
        let (mut app, ctx, paths) = picker_with_notes("copy-focus", &["copy me"]);
        // Stands in for the surface that owned focus before the picker opened.
        app.focus_stack.insert(0, FocusKind::CommandPalette);
        let _ = press_picker_chord(&mut app, &ctx, egui::Modifiers::COMMAND, egui::Key::C);
        assert_eq!(
            app.focus_stack.last(),
            Some(&FocusKind::CommandPalette),
            "the layer under the picker must be restored"
        );
        let _ = std::fs::remove_file(&paths[0]);
    }

    #[test]
    fn notes_picker_cmd_c_copies_content_and_closes_picker() {
        let (mut app, ctx, paths) = picker_with_notes("copy", &["copy me"]);
        let output = press_picker_chord(&mut app, &ctx, egui::Modifiers::COMMAND, egui::Key::C);
        assert_eq!(clipboard_text(&output).as_deref(), Some("copy me"));
        assert_eq!(app.notes_picker_last_copied.as_deref(), Some("copy me"));
        assert!(app.notes_picker_pending_delete.is_none());
        assert!(paths[0].exists());
        assert!(
            !app.focus_stack.contains(&FocusKind::NotesPicker),
            "Cmd+C must close the notes picker on successful copy"
        );
        let _ = std::fs::remove_file(&paths[0]);
    }

    /// Cut puts the same body payload on the clipboard and keeps the consent
    /// flow: cancel deletes nothing, confirm removes the file and closes.
    #[test]
    fn notes_picker_cut_uses_the_body_payload_and_keeps_consent() {
        let body = "cut body\n";
        let full = format!("---\ntitle: \"C\"\nsource: \"cli\"\n---\n{body}");
        let (mut app, ctx, paths) = picker_with_notes("cut-metadata", &[full.as_str()]);

        let output = press_picker_chord(&mut app, &ctx, egui::Modifiers::COMMAND, egui::Key::X);
        assert_eq!(clipboard_text(&output).as_deref(), Some(body));
        assert!(
            paths[0].exists(),
            "cut must wait for confirmation before deleting"
        );
        assert!(app
            .notes_picker_pending_delete
            .as_ref()
            .is_some_and(|p| p.cut));

        press_picker_key(&mut app, &ctx, egui::Key::Escape);
        assert!(app.notes_picker_pending_delete.is_none());
        assert!(paths[0].exists(), "a cancelled cut must not delete");

        press_picker_chord(&mut app, &ctx, egui::Modifiers::COMMAND, egui::Key::X);
        press_picker_key(&mut app, &ctx, egui::Key::Enter);
        assert!(!paths[0].exists());
        assert!(!app.focus_stack.contains(&FocusKind::NotesPicker));
    }

    #[test]
    fn notes_picker_cmd_c_read_failure_keeps_picker_open_with_error() {
        let (mut app, ctx, paths) = picker_with_notes("copy-fail", &["missing note"]);
        let _ = std::fs::remove_file(&paths[0]);
        press_picker_chord(&mut app, &ctx, egui::Modifiers::COMMAND, egui::Key::C);
        assert!(app.notes_picker_last_copied.is_none());
        assert!(
            app.focus_stack.contains(&FocusKind::NotesPicker),
            "failed copy must keep the notes picker open"
        );
        let status = app.notes_picker_status.as_deref().expect("error status");
        assert!(status.contains("Can't copy"));
    }

    #[test]
    fn notes_picker_cmd_x_copies_then_deletes_and_closes_after_confirm() {
        let (mut app, ctx, paths) = picker_with_notes("cut", &["cut me"]);
        press_picker_chord(&mut app, &ctx, egui::Modifiers::COMMAND, egui::Key::X);
        assert_eq!(app.notes_picker_last_copied.as_deref(), Some("cut me"));
        assert!(paths[0].exists(), "cut waits for confirm");
        assert!(app
            .notes_picker_pending_delete
            .as_ref()
            .is_some_and(|p| p.cut));
        assert!(
            app.focus_stack.contains(&FocusKind::NotesPicker),
            "cut must stay open awaiting confirm"
        );
        press_picker_key(&mut app, &ctx, egui::Key::Enter);
        assert!(!paths[0].exists());
        assert!(app.notes_picker_entries.is_empty());
        assert!(
            !app.focus_stack.contains(&FocusKind::NotesPicker),
            "confirmed cut must close the notes picker after deleting"
        );
    }

    #[test]
    fn notes_picker_cut_cancel_leaves_note_and_picker_open() {
        let (mut app, ctx, paths) = picker_with_notes("cut-cancel", &["keep me"]);
        press_picker_chord(&mut app, &ctx, egui::Modifiers::COMMAND, egui::Key::X);
        assert_eq!(app.notes_picker_last_copied.as_deref(), Some("keep me"));
        assert!(app.notes_picker_pending_delete.is_some());
        press_picker_key(&mut app, &ctx, egui::Key::Escape);
        assert!(app.notes_picker_pending_delete.is_none());
        assert!(paths[0].exists());
        assert!(
            app.focus_stack.contains(&FocusKind::NotesPicker),
            "cancelled cut must leave the notes picker open"
        );
        let _ = std::fs::remove_file(&paths[0]);
    }

    #[test]
    #[cfg(unix)]
    fn notes_picker_cut_failed_delete_keeps_picker_open() {
        use std::os::unix::fs::PermissionsExt;
        let ctx = egui::Context::default();
        crate::ui::theme::setup_fonts(&ctx);
        let frame_tick = crate::platform::logging::FrameTick::default();
        let (mut app, _tx) = PlexiApp::new_for_test(ctx.clone(), frame_tick);

        let dir = std::env::temp_dir().join(format!(
            "plexi-notes-picker-failed-del-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("create test dir");
        let note_path = dir.join("locked-note.md");
        std::fs::write(&note_path, "undeletable note content").expect("seed note");

        app.notes_picker_entries =
            vec![picker_entry(note_path.clone(), "undeletable note content")];
        app.notes_picker_selected = 0;
        app.push_focus_layer(FocusKind::NotesPicker);

        // Stage the cut
        press_picker_chord(&mut app, &ctx, egui::Modifiers::COMMAND, egui::Key::X);
        assert_eq!(
            app.notes_picker_last_copied.as_deref(),
            Some("undeletable note content")
        );
        assert!(app.notes_picker_pending_delete.is_some());

        // Make parent directory read-only so remove_file fails with PermissionDenied
        let orig_perms = std::fs::metadata(&dir).unwrap().permissions();
        let mut ro_perms = orig_perms.clone();
        ro_perms.set_mode(0o500);
        std::fs::set_permissions(&dir, ro_perms).unwrap();

        // Confirm delete
        press_picker_key(&mut app, &ctx, egui::Key::Enter);

        // Restore permissions for cleanup
        let mut rw_perms = orig_perms;
        rw_perms.set_mode(0o700);
        let _ = std::fs::set_permissions(&dir, rw_perms);

        assert!(
            note_path.exists(),
            "note file should still exist when delete fails"
        );
        assert!(
            app.focus_stack.contains(&FocusKind::NotesPicker),
            "picker must stay open when delete fails"
        );
        let status = app.notes_picker_status.as_deref().expect("error status");
        assert!(status.contains("Can't delete"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn notes_picker_cmd_backspace_confirms_before_deleting() {
        let (mut app, ctx, paths) = picker_with_notes("confirm", &["one", "two"]);
        press_picker_chord(
            &mut app,
            &ctx,
            egui::Modifiers::COMMAND,
            egui::Key::Backspace,
        );
        assert!(app.notes_picker_pending_delete.is_some());
        assert!(paths[0].exists());

        // Esc cancels the confirm and leaves the picker open.
        press_picker_key(&mut app, &ctx, egui::Key::Escape);
        assert!(app.notes_picker_pending_delete.is_none());
        assert!(paths[0].exists());
        assert!(app.focus_stack.contains(&FocusKind::NotesPicker));

        press_picker_chord(
            &mut app,
            &ctx,
            egui::Modifiers::COMMAND,
            egui::Key::Backspace,
        );
        press_picker_key(&mut app, &ctx, egui::Key::Y);
        assert!(!paths[0].exists());
        assert!(paths[1].exists());
        assert_eq!(app.notes_picker_entries.len(), 1);
        assert_eq!(app.notes_picker_preview_text().as_deref(), Some("two"));
        let _ = std::fs::remove_file(&paths[1]);
    }

    #[test]
    fn notes_picker_delete_closes_open_editor_pane_and_does_not_recreate_file() {
        let (mut app, ctx, paths) = picker_with_notes("delete-open", &["open note"]);
        let (_tile, pane_id) = app.add_test_pane();
        replace_with_text_editor(&mut app, pane_id, paths[0].clone());
        let (focused_tile, _other) = app.add_test_pane();
        app.windows[0].focused_pane = Some(focused_tile);

        press_picker_chord(
            &mut app,
            &ctx,
            egui::Modifiers::COMMAND,
            egui::Key::Backspace,
        );
        press_picker_key(&mut app, &ctx, egui::Key::Enter);

        assert!(
            !app.windows[0].panes.contains_key(&pane_id),
            "editor pane must close"
        );
        assert!(!paths[0].exists());
        assert!(app.notes_picker_entries.is_empty());
        assert!(app.focus_stack.contains(&FocusKind::NotesPicker));
        draw_picker_frame(&mut app, &ctx);
        assert!(
            !paths[0].exists(),
            "closed editor must not recreate the note"
        );
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
            search_text: format!("{title} {body}").to_lowercase(),
            tier_label: None,
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

    fn set_editor_pane_unsaved(app: &mut PlexiApp, pane_id: u64, text: &str) {
        let runtime = &mut app.windows[0]
            .panes
            .get_mut(&pane_id)
            .expect("editor pane")
            .as_app_mut()
            .expect("app pane")
            .runtime;
        let AppRuntime::Builtin(editor) = runtime else {
            panic!("expected builtin editor");
        };
        editor
            .as_any_mut()
            .downcast_mut::<crate::app::text_editor_app::TextEditorApp>()
            .expect("text editor")
            .test_set_unsaved(text);
    }

    #[test]
    fn notes_picker_modal_rect_identical_across_status_states() {
        let (mut app, ctx, paths) = picker_with_notes("rect-states", &["first note", "second"]);
        let area_id = egui::Id::new("notes_picker").with("overlay");
        let measure = |app: &mut PlexiApp, ctx: &egui::Context| -> egui::Rect {
            draw_picker_frame(app, ctx);
            draw_picker_frame(app, ctx);
            ctx.memory(|m| m.area_rect(area_id))
                .expect("notes_picker overlay area_rect")
        };

        let rect_none = measure(&mut app, &ctx);

        app.notes_picker_pending_delete = Some(PendingDelete {
            path: paths[0].clone(),
            cut: false,
        });
        app.notes_picker_status = None;
        let rect_confirm = measure(&mut app, &ctx);

        app.notes_picker_pending_delete = None;
        app.notes_picker_status = Some(
            "Can't delete \u{201c}first note\u{201d}: its pane has unsaved edits.".to_string(),
        );
        let rect_short_err = measure(&mut app, &ctx);

        let long_msg = format!(
            "Can't delete \u{201c}first note\u{201d}: {}",
            "its open editor pane has unsaved edits that would be lost if closed now; ".repeat(10)
        );
        assert!(long_msg.len() > 400);
        app.notes_picker_status = Some(long_msg);
        let rect_long_err = measure(&mut app, &ctx);

        assert_eq!(rect_confirm.min, rect_none.min);
        assert_eq!(rect_confirm.size(), rect_none.size());
        assert_eq!(rect_confirm, rect_none);
        assert_eq!(rect_short_err.min, rect_none.min);
        assert_eq!(rect_short_err.size(), rect_none.size());
        assert_eq!(rect_short_err, rect_none);
        assert_eq!(rect_long_err.min, rect_none.min);
        assert_eq!(rect_long_err.size(), rect_none.size());
        assert_eq!(rect_long_err, rect_none);

        for p in paths {
            let _ = std::fs::remove_file(p);
        }
    }

    #[test]
    fn notes_picker_refuses_delete_and_cut_when_open_editor_pane_is_dirty() {
        let (mut app, ctx, paths) = picker_with_notes("dirty-pane", &["dirty note", "other note"]);
        let (_tile, pane_id) = app.add_test_pane();
        replace_with_text_editor(&mut app, pane_id, paths[0].clone());
        set_editor_pane_unsaved(&mut app, pane_id, "unsaved draft");

        press_picker_chord(
            &mut app,
            &ctx,
            egui::Modifiers::COMMAND,
            egui::Key::Backspace,
        );
        assert!(paths[0].exists(), "dirty note file must remain on disk");
        assert!(
            app.windows[0].panes.contains_key(&pane_id),
            "dirty editor pane must remain open"
        );
        assert!(
            app.notes_picker_pending_delete.is_none(),
            "dirty delete must not enter pending confirm"
        );
        let delete_err = app
            .notes_picker_status
            .as_deref()
            .expect("status error set on refused delete");
        assert!(delete_err.contains("delete"));
        assert!(delete_err.contains("unsaved edits"));

        press_picker_chord(&mut app, &ctx, egui::Modifiers::COMMAND, egui::Key::X);
        assert!(paths[0].exists(), "dirty note file must remain on disk");
        assert!(app.windows[0].panes.contains_key(&pane_id));
        assert!(app.notes_picker_pending_delete.is_none());
        assert!(
            app.notes_picker_last_copied.is_none(),
            "refused cut must not copy to clipboard"
        );
        let cut_err = app
            .notes_picker_status
            .as_deref()
            .expect("status error set on refused cut");
        assert!(cut_err.contains("cut"));
        assert!(cut_err.contains("unsaved edits"));

        // Moving selection clears the status error.
        press_picker_key(&mut app, &ctx, egui::Key::ArrowDown);
        assert!(app.notes_picker_status.is_none());
        press_picker_key(&mut app, &ctx, egui::Key::ArrowUp);

        // Re-check at confirm time if the pane was dirtied while confirm was pending.
        app.notes_picker_pending_delete = Some(PendingDelete {
            path: paths[0].clone(),
            cut: false,
        });
        press_picker_key(&mut app, &ctx, egui::Key::Enter);
        assert!(paths[0].exists());
        assert!(app.windows[0].panes.contains_key(&pane_id));
        assert!(app.notes_picker_pending_delete.is_none());
        assert!(app
            .notes_picker_status
            .as_deref()
            .is_some_and(|s| s.contains("unsaved edits")));

        for p in paths {
            let _ = std::fs::remove_file(p);
        }
    }

    #[test]
    fn notes_picker_handles_egui_copy_and_cut_events_without_key_events() {
        let body = "full body content";
        let full = format!("---\ntitle: Event Copy\nsource: \"scratchpad\"\n---\n{body}");
        let (mut app, ctx, paths) = picker_with_notes("copy-cut-events", &[full.as_str()]);

        let mut copy_input = picker_frame_input();
        copy_input.events.push(egui::Event::Copy);
        let copy_output = ctx.run_ui(copy_input, |ui| {
            let mut input = crate::app::input_router::PlexiInput::take_from(ui.ctx());
            app.notes_picker_handle_key(&mut input);
            app.notes_picker_flush_copy(ui.ctx());
        });
        assert_eq!(clipboard_text(&copy_output).as_deref(), Some(body));
        assert_eq!(app.notes_picker_last_copied.as_deref(), Some(body));
        assert!(app.notes_picker_pending_delete.is_none());
        assert!(paths[0].exists());
        assert!(
            !app.focus_stack.contains(&FocusKind::NotesPicker),
            "egui Event::Copy must close the notes picker"
        );

        // Reopen for cut test
        app.push_focus_layer(FocusKind::NotesPicker);
        app.notes_picker_last_copied = None;
        let mut cut_input = picker_frame_input();
        cut_input.events.push(egui::Event::Cut);
        let cut_output = ctx.run_ui(cut_input, |ui| {
            let mut input = crate::app::input_router::PlexiInput::take_from(ui.ctx());
            app.notes_picker_handle_key(&mut input);
            app.notes_picker_flush_copy(ui.ctx());
        });
        assert_eq!(clipboard_text(&cut_output).as_deref(), Some(body));
        assert_eq!(app.notes_picker_last_copied.as_deref(), Some(body));
        assert!(app
            .notes_picker_pending_delete
            .as_ref()
            .is_some_and(|p| p.cut && p.path == paths[0]));
        assert!(
            paths[0].exists(),
            "cut must wait for confirm before deleting"
        );
        assert!(
            app.focus_stack.contains(&FocusKind::NotesPicker),
            "egui Event::Cut must stay open awaiting confirm"
        );

        press_picker_key(&mut app, &ctx, egui::Key::Enter);
        assert!(!paths[0].exists());
        assert!(app.notes_picker_entries.is_empty());
        assert!(
            !app.focus_stack.contains(&FocusKind::NotesPicker),
            "confirmed cut must close the notes picker"
        );
    }
}

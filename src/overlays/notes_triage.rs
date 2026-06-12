//! Notes inbox triage overlay — reached via `t` from the notes picker.
//!
//! Shows inbox notes one at a time. Key bindings:
//!   h / ←          — previous note
//!   l / n / Enter / → — next note
//!   s              — keep (move to workspace notes dir)
//!   x / d          — trash (move to notes/trash/)
//!   1–9            — run configured action, then trash and advance
//!   Esc            — back to the notes picker

use crate::app::{FocusLayer, PlexiApp};
use crate::notes::{InboxNote, TriageAction};
use crate::ui::style;
use crate::ui::{
    hints::{HintBar, HintGroup},
    overlay::ModalShell,
};

impl PlexiApp {
    /// Leave triage and return to the notes picker (Esc, inbox emptied, or
    /// advancing past the last note). The picker re-scans, so the inbox
    /// section reflects what triage just moved.
    fn notes_triage_back_to_picker(&mut self) {
        self.pop_focus_layer(&FocusLayer::NotesTriage);
        if !self.focus_stack.contains(&FocusLayer::NotesPicker) {
            self.open_notes_picker();
        }
    }

    /// Open the notes triage overlay. Loads inbox + actions and pushes the focus layer.
    pub(crate) fn open_notes_triage(&mut self) {
        let notes = crate::notes::scan_inbox();
        let actions = crate::notes::load_triage_actions();
        log::info!(
            "notes_triage: opening with {} note(s) and {} action(s)",
            notes.len(),
            actions.len()
        );
        self.notes_triage_notes = notes;
        self.notes_triage_actions = actions;
        self.notes_triage_index = 0;
        self.push_focus_layer(FocusLayer::NotesTriage);
    }

    /// Handle keyboard input for the triage overlay. Must surrender egui focus
    /// so the TextEdit in the modal does not reclaim it.
    pub(crate) fn notes_triage_handle_key(&mut self, ctx: &egui::Context) {
        ctx.memory_mut(|m| {
            if let Some(id) = m.focused() {
                m.surrender_focus(id);
            }
        });

        if self.notes_triage_notes.is_empty() {
            log::info!("notes_triage: inbox empty — returning to picker");
            self.notes_triage_back_to_picker();
            return;
        }

        #[derive(Clone, Copy)]
        enum TriageKey {
            Keep,
            Trash,
            Next,
            Prev,
            Close,
            Action(u8),
        }

        let action = ctx.input_mut(|i| {
            if i.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
                Some(TriageKey::Close)
            } else if i.consume_key(egui::Modifiers::NONE, egui::Key::S) {
                Some(TriageKey::Keep)
            } else if i.consume_key(egui::Modifiers::NONE, egui::Key::D)
                || i.consume_key(egui::Modifiers::NONE, egui::Key::X)
            {
                Some(TriageKey::Trash)
            } else if i.consume_key(egui::Modifiers::NONE, egui::Key::H)
                || i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft)
            {
                Some(TriageKey::Prev)
            } else if i.consume_key(egui::Modifiers::NONE, egui::Key::L)
                || i.consume_key(egui::Modifiers::NONE, egui::Key::N)
                || i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
                || i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight)
            {
                Some(TriageKey::Next)
            } else {
                // Check digit keys 1–9 for actions.
                let digits = [
                    (egui::Key::Num1, 1u8),
                    (egui::Key::Num2, 2),
                    (egui::Key::Num3, 3),
                    (egui::Key::Num4, 4),
                    (egui::Key::Num5, 5),
                    (egui::Key::Num6, 6),
                    (egui::Key::Num7, 7),
                    (egui::Key::Num8, 8),
                    (egui::Key::Num9, 9),
                ];
                for (key, digit) in digits {
                    if i.consume_key(egui::Modifiers::NONE, key) {
                        return Some(TriageKey::Action(digit));
                    }
                }
                None
            }
        });

        match action {
            Some(TriageKey::Close) => {
                log::info!("notes_triage: Esc — back to picker");
                self.notes_triage_back_to_picker();
            }
            Some(TriageKey::Keep) => {
                if let Some(note) = self.notes_triage_notes.get(self.notes_triage_index).cloned() {
                    log::info!("notes_triage: s — keeping {:?}", note.path);
                    self.notes_triage_keep(&note);
                    self.notes_triage_advance();
                }
            }
            Some(TriageKey::Trash) => {
                if let Some(note) = self.notes_triage_notes.get(self.notes_triage_index).cloned() {
                    log::info!("notes_triage: d/x — trashing {:?}", note.path);
                    self.notes_triage_trash(&note);
                    self.notes_triage_advance();
                }
            }
            Some(TriageKey::Next) => {
                log::info!("notes_triage: l/n/Enter/→ — next");
                self.notes_triage_index += 1;
                if self.notes_triage_index >= self.notes_triage_notes.len() {
                    log::info!("notes_triage: end of inbox — back to picker");
                    self.notes_triage_back_to_picker();
                }
            }
            Some(TriageKey::Prev) => {
                log::info!("notes_triage: h/← — previous");
                self.notes_triage_index = self.notes_triage_index.saturating_sub(1);
            }
            Some(TriageKey::Action(digit)) => {
                let action = self
                    .notes_triage_actions
                    .iter()
                    .find(|a| a.key == digit)
                    .cloned();
                if let (Some(action), Some(note)) = (
                    action,
                    self.notes_triage_notes.get(self.notes_triage_index).cloned(),
                ) {
                    log::info!(
                        "notes_triage: {} — running action '{}' on {:?}",
                        digit,
                        action.label,
                        note.path
                    );
                    self.notes_triage_run_action(&note, &action);
                    // action.workspace overrides the keep destination; present means keep there.
                    if let Some(ws) = action.workspace.as_deref() {
                        self.notes_triage_keep_to(&note, ws);
                    } else {
                        self.notes_triage_trash(&note);
                    }
                    self.notes_triage_advance();
                }
            }
            None => {}
        }
    }

    /// Draw the triage modal for the current inbox note.
    pub(crate) fn draw_notes_triage(&mut self, ctx: &egui::Context) {
        let colors = self.colors;

        if self.notes_triage_notes.is_empty() {
            let modal_response = ModalShell::centered("notes_triage")
                .title("Inbox Triage")
                .width(480.0)
                .escape(true)
                .show(ctx, &colors, |ui| {
                    ui.label(
                        egui::RichText::new("Inbox is empty.")
                            .size(style::TEXT_HINT)
                            .color(colors.text_dim),
                    );
                });
            if modal_response.dismissed {
                self.notes_triage_back_to_picker();
            }
            return;
        }

        let idx = self.notes_triage_index;
        let total = self.notes_triage_notes.len();

        if idx >= total {
            self.notes_triage_back_to_picker();
            return;
        }

        let note = self.notes_triage_notes[idx].clone();
        let actions = self.notes_triage_actions.clone();

        let title = format!("Inbox Triage ({}/{})", idx + 1, total);

        // Pre-build owned key/label strings for actions so we can borrow them
        // inside the closure without lifetime issues.
        let action_key_labels: Vec<(String, String)> = actions
            .iter()
            .map(|a| (a.key.to_string(), a.label.clone()))
            .collect();

        let modal_response = ModalShell::centered("notes_triage")
            .title(&title)
            .width(520.0)
            .escape(true)
            .show(ctx, &colors, |ui| {
                // Note title (frontmatter), when set.
                if let Some(ref title) = note.frontmatter.title {
                    ui.label(
                        egui::RichText::new(title.as_str())
                            .size(style::TEXT_BODY)
                            .strong()
                            .color(colors.text_primary),
                    );
                    ui.add_space(style::SPACE_XS);
                }
                // Show frontmatter metadata.
                if let Some(ref ts) = note.frontmatter.captured_at {
                    ui.label(
                        egui::RichText::new(format!("captured: {ts}"))
                            .size(style::TEXT_HINT)
                            .color(colors.text_dim),
                    );
                }
                if let Some(ref cwd) = note.frontmatter.cwd {
                    ui.label(
                        egui::RichText::new(format!("cwd: {cwd}"))
                            .size(style::TEXT_HINT)
                            .color(colors.text_dim),
                    );
                }

                ui.add_space(style::SPACE_SM);

                // Note body (scrollable).
                egui::ScrollArea::vertical()
                    .max_height(240.0)
                    .show(ui, |ui| {
                        ui.label(
                            egui::RichText::new(note.body.trim())
                                .size(style::TEXT_BODY)
                                .color(colors.text_primary),
                        );
                    });

                ui.add_space(style::SPACE_SM);

                // Core navigation/decision hints.
                let core_hints = [
                    HintGroup::new(&["h", "l"], "prev/next"),
                    HintGroup::new(&["s"], "keep"),
                    HintGroup::new(&["x"], "trash"),
                    HintGroup::new(&["esc"], "back to notes"),
                ];
                HintBar::new(&core_hints).show(ui, &colors);

                // Configured digit actions in their own section.
                if !action_key_labels.is_empty() {
                    ui.add_space(style::SPACE_XS);
                    ui.label(
                        egui::RichText::new("actions")
                            .size(style::TEXT_CAPTION)
                            .color(colors.text_dim),
                    );
                    // Bind each key slice to a named local so the &str lives long
                    // enough for HintGroup.
                    let action_hints: Vec<([&str; 1], &str)> = action_key_labels
                        .iter()
                        .map(|(k, l)| ([k.as_str()], l.as_str()))
                        .collect();
                    let groups: Vec<HintGroup> = action_hints
                        .iter()
                        .map(|(keys, label)| HintGroup::new(keys.as_slice(), label))
                        .collect();
                    HintBar::new(&groups).show(ui, &colors);
                }
            });

        if modal_response.dismissed {
            self.notes_triage_back_to_picker();
        }
    }

    /// Remove the current note from the triage list; return to the picker when
    /// the inbox empties.
    pub(crate) fn notes_triage_advance(&mut self) {
        if self.notes_triage_notes.is_empty() {
            self.notes_triage_back_to_picker();
            return;
        }
        // Remove the note at the current index (don't advance index — next note
        // slides into position).
        if self.notes_triage_index < self.notes_triage_notes.len() {
            self.notes_triage_notes.remove(self.notes_triage_index);
        }
        if self.notes_triage_notes.is_empty() {
            log::info!("notes_triage: inbox cleared — back to picker");
            self.notes_triage_back_to_picker();
        } else if self.notes_triage_index >= self.notes_triage_notes.len() {
            self.notes_triage_index = self.notes_triage_notes.len() - 1;
        }
    }

    /// Move `note` to `notes/<workspace>/` using the workspace slug from the note's frontmatter.
    pub(crate) fn notes_triage_keep(&self, note: &InboxNote) {
        let active_root = crate::config::active_workspace_root();
        let active_slug: Option<String> = active_root
            .as_ref()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .map(|s| s.to_string());
        let workspace = note
            .frontmatter
            .workspace
            .as_deref()
            .or_else(|| active_slug.as_deref())
            .unwrap_or("default");
        self.notes_triage_keep_to(note, workspace);
    }

    /// Move `note` to `notes/<workspace>/` using an explicit workspace slug.
    pub(crate) fn notes_triage_keep_to(&self, note: &InboxNote, workspace: &str) {
        let dest_dir = crate::config::config_dir()
            .join("notes")
            .join(workspace);
        if let Err(e) = std::fs::create_dir_all(&dest_dir) {
            log::warn!("notes_triage: keep — failed to create {:?}: {e}", dest_dir);
            return;
        }
        let file_name = note.path.file_name().unwrap_or_default();
        let dest = dest_dir.join(file_name);
        match std::fs::rename(&note.path, &dest) {
            Ok(_) => log::info!("notes_triage: kept {:?} → {:?}", note.path, dest),
            Err(e) => {
                // rename() fails across filesystems; fall back to copy+delete.
                match std::fs::copy(&note.path, &dest) {
                    Ok(_) => {
                        let _ = std::fs::remove_file(&note.path);
                        log::info!(
                            "notes_triage: kept (copy) {:?} → {:?}",
                            note.path,
                            dest
                        );
                    }
                    Err(e2) => {
                        log::warn!(
                            "notes_triage: keep failed rename={e} copy={e2} {:?}",
                            note.path
                        );
                    }
                }
            }
        }
    }

    /// Move `note` to `notes/trash/`.
    pub(crate) fn notes_triage_trash(&self, note: &InboxNote) {
        let trash_dir = crate::config::config_dir().join("notes").join("trash");
        if let Err(e) = std::fs::create_dir_all(&trash_dir) {
            log::warn!("notes_triage: trash — failed to create {:?}: {e}", trash_dir);
            // Fallback: just delete.
            let _ = std::fs::remove_file(&note.path);
            return;
        }
        let file_name = note.path.file_name().unwrap_or_default();
        let dest = trash_dir.join(file_name);
        match std::fs::rename(&note.path, &dest) {
            Ok(_) => log::info!("notes_triage: trashed {:?} → {:?}", note.path, dest),
            Err(_) => {
                match std::fs::copy(&note.path, &dest) {
                    Ok(_) => {
                        let _ = std::fs::remove_file(&note.path);
                        log::info!(
                            "notes_triage: trashed (copy) {:?} → {:?}",
                            note.path,
                            dest
                        );
                    }
                    Err(e2) => {
                        log::warn!(
                            "notes_triage: trash failed {:?}: {e2} — deleting directly",
                            note.path
                        );
                        let _ = std::fs::remove_file(&note.path);
                    }
                }
            }
        }
    }

    /// Execute the action's shell command against the note.
    /// Hidden actions run silently in the background. Visible actions open a terminal pane.
    pub(crate) fn notes_triage_run_action(&mut self, note: &InboxNote, action: &TriageAction) {
        let cmd =
            crate::notes::substitute_action_tokens(&action.command, &note.body, &note.frontmatter);
        log::info!(
            "notes_triage: running action '{}' hidden={}: {}",
            action.label,
            action.hidden,
            cmd
        );

        if action.hidden {
            match std::process::Command::new("sh").args(["-c", &cmd]).spawn() {
                Ok(_) => log::info!("notes_triage: hidden action spawned"),
                Err(e) => log::warn!("notes_triage: hidden action spawn failed: {e}"),
            }
        } else {
            // Open a visible terminal pane that stays open after the command finishes.
            let win_idx = self.active_window;
            if let Some(focused_tile) = self.windows[win_idx].focused_pane {
                self.spawn_terminal_pane_at(
                    win_idx,
                    focused_tile,
                    true,  // split vertically
                    false,
                    Some(&cmd),
                    false, // keep pane open after exit
                    None,
                    false,
                );
            } else {
                log::warn!("notes_triage: no focused tile — falling back to background spawn");
                let _ = std::process::Command::new("sh").args(["-c", &cmd]).spawn();
            }
        }
    }
}

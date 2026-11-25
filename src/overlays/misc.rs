use super::*;
use crate::broker::permit::ContextRootProof;
use crate::host::keys::{Action, Direction, KeyBindings};

/// Shown in the root editor's title when socket input shared a frame with Enter.
const SYNTHETIC_NOTE: &str = ": not applied, press Enter again";

pub(crate) struct WindowChosenRoot(());

/// The proof for a root a person chose in the Plexi window: typed or picked in
/// the root editor, or taken from the focused view by their own keypress or
/// click. The caller must have checked that the frame carried no synthetic
/// input; this proves which UI handled the value, not that a human did
/// (human-input evidence is stint 1283).
pub(crate) fn window_chosen_root() -> ContextRootProof {
    ContextRootProof::HostUi(WindowChosenRoot(()))
}

#[cfg(test)]
impl PlexiApp {
    /// Set up a fixture through the same boundary as a window-chosen root.
    pub(crate) fn set_context_root(&mut self, root: std::path::PathBuf, id: Option<u64>) {
        self.commit_context_root(
            root,
            id.unwrap_or(self.router.active().context_id),
            window_chosen_root(),
        )
        .unwrap();
    }
}

pub(super) fn section_label(ui: &mut egui::Ui, title: &str, colors: &Colors) {
    ui.label(
        RichText::new(title)
            .size(style::TEXT_HINT)
            .color(colors.text_dim)
            .strong(),
    );
    ui.add_space(2.0);
}

fn shortcut_description(ui: &mut egui::Ui, desc: &str, colors: &Colors) {
    crate::ui::typography::caption(ui, desc, colors);
}

fn shortcut_section(
    ui: &mut egui::Ui,
    title: &'static str,
    rows: &[(&[Action], Option<&str>)],
    cell_width: f32,
    bindings: &KeyBindings,
    colors: &Colors,
) {
    section_label(ui, title, colors);
    egui::Grid::new(("workspace_shortcuts", title))
        .num_columns(2)
        .min_col_width(0.0)
        .max_col_width(cell_width)
        .spacing([style::SPACE_SM, 4.0])
        .show(ui, |ui| {
            for (actions, group_label) in rows {
                let chords: Vec<_> = actions
                    .iter()
                    .filter_map(|action| bindings.chord_for_action(action))
                    .map(crate::ui::shortcuts::egui_chord_tokens)
                    .collect();
                if chords.is_empty() {
                    continue;
                }
                let combos: Vec<_> = chords.iter().map(Vec::as_slice).collect();
                let label = group_label.or_else(|| actions.first()?.command_label());
                if let [keys] = combos.as_slice() {
                    crate::ui::shortcuts::key_combo(ui, keys, colors);
                } else {
                    let prefix = &combos[0][..combos[0].len() - 1];
                    if combos.iter().all(|keys| &keys[..keys.len() - 1] == prefix) {
                        let mut keys = prefix.to_vec();
                        keys.extend(combos.iter().filter_map(|keys| keys.last().copied()));
                        crate::ui::shortcuts::key_combo(ui, &keys, colors);
                    } else {
                        crate::ui::shortcuts::key_combo_list(ui, &combos, None, colors);
                    }
                }
                if let Some(label) = label {
                    shortcut_description(ui, label, colors);
                }
                ui.end_row();
            }
        });
}

fn should_show_changelog_line(line: &str) -> bool {
    let Some(entry) = line
        .trim_start()
        .strip_prefix("- ")
        .or_else(|| line.trim_start().strip_prefix("* "))
    else {
        return true;
    };
    let entry = entry
        .trim_start_matches("**")
        .trim_start()
        .to_ascii_lowercase();
    !entry.starts_with("chore:") && !entry.starts_with("chore(")
}

fn should_render_changelog_body_line(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed == "# Changelog" || trimmed == "Newest releases appear first." {
        return false;
    }
    should_show_changelog_line(line)
}

impl PlexiApp {
    pub(crate) fn draw_shortcuts_overlay(&mut self, ctx: &egui::Context) {
        if !self.show_shortcuts {
            return;
        }
        let response = crate::ui::overlay::ModalShell::centered("shortcuts")
            .title("Keyboard Shortcuts")
            .width(style::MODAL_WIDTH_MD)
            .escape(true)
            .show(ctx, &self.colors.clone(), |ui| {
                {
                    let colors = &self.colors;

                    ui.horizontal(|ui| {
                        // Keep both columns inside the shell's content budget. Grid
                        // minimums and unwrapped chip rows otherwise expand the Area.
                        ui.spacing_mut().item_spacing.x = 0.0;
                        let column_width = (ui.available_width() - 2.0 * style::SPACE_MD) / 2.0;
                        // Long chords may use up to 60%; descriptions wrap in
                        // the remaining space (or use more for shorter chords).
                        let cell_width = (column_width - style::SPACE_SM) * 0.6;
                        if ui.is_sizing_pass() {
                            log::info!("shortcuts layout: column_width={column_width:.1} cell_width={cell_width:.1}");
                        }
                        use Action::*;
                        use Direction::{Down, Left, Right, Up};

                        let bindings = &self.key_bindings;
                        ui.vertical(|ui| {
                            ui.set_width(column_width);
                            shortcut_section(ui, "LAYOUT", &[
                                (&[NewPageRight], None),
                                (&[NewContext], None),
                                (&[ParkContext], None),
                                (&[SplitHorizontal], None),
                                (&[SplitVertical], None),
                                (&[SplitRight], None),
                                (&[SplitDown], None),
                                (&[ClosePane], None),
                                (&[CloseContext], None),
                                (&[NewTab], None),
                            ], cell_width, bindings, colors);
                            ui.add_space(style::SPACE_SM);
                            shortcut_section(ui, "NAVIGATION", &[
                                (&[Navigate(Left), Navigate(Down), Navigate(Up), Navigate(Right)], Some("Move focus")),
                                (&[NextTab, PrevTab], Some("Next or previous tab")),
                                (&[SelectPane(Left), SelectPane(Down), SelectPane(Up), SelectPane(Right)], Some("Extend pane selection")),
                                (&[ZoomIn], None),
                                (&[SwitchContext(0), SwitchContext(1), SwitchContext(2), SwitchContext(3),
                                    SwitchContext(4), SwitchContext(5), SwitchContext(6), SwitchContext(7), SwitchContext(8)], Some("Switch context by position")),
                                (&[ContextZoomOut], None),
                                (&[SwapPane(Left), SwapPane(Down), SwapPane(Up), SwapPane(Right)], Some("Swap pane")),
                                (&[SendPane(Left), SendPane(Down), SendPane(Up), SendPane(Right)], Some("Send pane")),
                            ], cell_width, bindings, colors);
                        });
                        ui.add_space(style::SPACE_MD);
                        ui.add(egui::Separator::default().vertical().spacing(0.0));
                        ui.add_space(style::SPACE_MD);
                        ui.vertical(|ui| {
                            ui.set_width(column_width);
                            shortcut_section(ui, "PLUGINS & TOOLS", &[
                                (&[ToggleCommandPalette], None),
                                (&[OpenFileBrowser], None),
                                (&[OpenQuickNote], None),
                                (&[OpenNotesPicker], None),
                                (&[OpenScratchpad], None),
                                (&[ToggleSidebar], None),
                                (&[ToggleAttentionQueue], None),
                                (&[ToggleMinimap], None),
                            ], cell_width, bindings, colors);
                            ui.add_space(style::SPACE_SM);
                            shortcut_section(ui, "TERMINAL", &[
                                (&[ScrollUp, ScrollDown], Some("Scroll terminal")),
                                (&[IncreasePaneFontSize, DecreasePaneFontSize], Some("Change text size")),
                                (&[UiScaleUp, UiScaleDown], Some("Change UI scale")),
                                (&[UiScaleReset], None),
                            ], cell_width, bindings, colors);
                            ui.add_space(style::SPACE_SM);
                            shortcut_section(ui, "SETTINGS", &[
                                (&[OpenConfig], None),
                                (&[ReloadConfig], None),
                                (&[OpenSecretsManager], None),
                                (&[RenamePane], None),
                                (&[RenameContext], None),
                                (&[SetContextRootFromCwd], None),
                                (&[ToggleShortcuts], None),
                                (&[Quit], None),
                            ], cell_width, bindings, colors);
                        });
                    });

                    // ── Explorer: generated from the key table classify_key
                    // reads, so it cannot drift from the key handling (stint 1555).
                    ui.add_space(style::SPACE_MD);
                    section_label(ui, "EXPLORER", colors);
                    let explorer = crate::file_browser::explorer_shortcut_rows();
                    let half = explorer.len().div_ceil(2);
                    // Two equal columns whose gap matches the divider gap above
                    // (SPACE_MD, the 1 pt divider, SPACE_MD), so the right half
                    // starts under the right column of the sheet.
                    let column_width = (ui.available_width() - 2.0 * style::SPACE_MD) / 2.0;
                    let cell_width = (column_width - style::SPACE_SM) * 0.6;
                    ui.scope(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0 * style::SPACE_MD + 1.0;
                        ui.columns(2, |columns| {
                            for (index, (column, rows)) in
                                columns.iter_mut().zip(explorer.chunks(half)).enumerate()
                            {
                                egui::Grid::new(("shortcuts_explorer", index))
                                    .num_columns(2)
                                    .min_col_width(0.0)
                                    .max_col_width(cell_width)
                                    .spacing([style::SPACE_SM, 4.0])
                                    .show(column, |ui| {
                                        for (label, combos) in rows {
                                            let combos: Vec<&[&str]> =
                                                combos.iter().map(Vec::as_slice).collect();
                                            crate::ui::shortcuts::key_combo_list(
                                                ui, &combos, None, colors,
                                            );
                                            shortcut_description(ui, label, colors);
                                            ui.end_row();
                                        }
                                    });
                            }
                        });
                    });

                    ui.add_space(style::SPACE_MD);
                    ui.separator();
                    ui.add_space(style::SPACE_SM);
                    draw_contact_footer(ui, colors);
                }
            });
        if response.dismissed {
            self.show_shortcuts = false;
        }
    }

    /// The sheet owns the keyboard while open: Esc or its own toggle chord
    /// closes it; every other key stays with the sheet (stint 1261).
    pub(crate) fn shortcuts_handle_key(
        &mut self,
        input: &mut crate::app::input_router::PlexiInput,
    ) -> crate::app::view_trait::KeyDisposition {
        let (modifiers, key) = self.key_bindings.toggle_shortcuts;
        if input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)
            || input.consume_key(modifiers, key)
        {
            self.show_shortcuts = false;
            self.sync_shortcuts_focus();
            log::info!("shortcuts: closed from the keyboard");
        }
        crate::app::view_trait::KeyDisposition::Consumed
    }

    pub(crate) fn toggle_changelog(&mut self) {
        if self.show_changelog {
            self.close_changelog();
        } else {
            self.show_changelog = true;
            self.push_focus_layer(crate::app::FocusKind::Changelog);
            log::info!("changelog: opened");
        }
    }

    pub(crate) fn close_changelog(&mut self) {
        self.update_confirm_prompt = false;
        if self.show_changelog {
            self.show_changelog = false;
            self.reconcile_focus_layer(crate::app::FocusKind::Changelog, false);
            log::info!("changelog: closed");
        }
    }

    pub(crate) fn changelog_handle_key(
        &mut self,
        input: &mut crate::app::input_router::PlexiInput,
    ) -> crate::app::view_trait::KeyDisposition {
        if self.update_confirm_prompt && input.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
        {
            self.update_confirm_prompt = false;
            self.restart_to_update("changelog confirm");
        }
        if input.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
            if self.update_confirm_prompt {
                log::info!("update: restart confirm cancelled");
            }
            self.close_changelog();
        }
        crate::app::view_trait::KeyDisposition::Consumed
    }

    /// Running version and update status. Ready and its confirm step share
    /// the panel's one layout: the confirm swaps the message and the actions
    /// in place, so the box never jumps. Enter and Esc stay with
    /// `changelog_handle_key`; the buttons only show them.
    fn changelog_update_status(&mut self, ui: &mut egui::Ui) {
        use crate::host::update_status::UpdateLine;
        use crate::ui::button::ButtonKind;
        use crate::ui::dialog::{DialogAction, DialogShortcut};
        use crate::ui::status_panel::{StatusPanel, Tone};
        use egui::{Key, Modifiers};
        let line = self.update_status.line();
        let caption = format!("Running v{}", self.display_version);
        let now = crate::platform::clock::now_secs();
        let mut message = crate::host::update_status::status_text(&line, now);
        let mut tone = match &line {
            UpdateLine::Failed { .. } => Tone::Danger,
            _ => Tone::Neutral,
        };
        let restart_label;
        let actions: Vec<DialogAction<'_>> = match &line {
            UpdateLine::Ready { version } if self.update_confirm_prompt => {
                message = format!(
                    "Plexi will restart into v{version}. Running panes will close; \
                     your workspace will be saved."
                );
                tone = Tone::Accent;
                restart_label = format!("Restart to v{version}");
                vec![
                    DialogAction::new("restart", &restart_label, ButtonKind::Primary)
                        .shortcut(DialogShortcut::new(&["Enter"], Modifiers::NONE, Key::Enter)),
                    DialogAction::new("cancel", "Cancel", ButtonKind::Ghost)
                        .shortcut(DialogShortcut::new(&["Esc"], Modifiers::NONE, Key::Escape)),
                ]
            }
            UpdateLine::Ready { .. } => {
                tone = Tone::Accent;
                vec![DialogAction::new(
                    "offer",
                    "Restart to update",
                    ButtonKind::Primary,
                )]
            }
            _ => Vec::new(),
        };
        let busy = matches!(line, UpdateLine::Checking | UpdateLine::Downloading { .. });
        let colors = self.colors;
        let clicked = StatusPanel::new("changelog_update", &caption, &message)
            .tone(tone)
            .busy(busy)
            .actions(&actions)
            .show(ui, colors.bg_sidebar, &colors);
        match clicked {
            Some("offer") => {
                log::info!("update: restart confirm shown from the changelog");
                self.update_confirm_prompt = true;
            }
            Some("restart") => {
                self.update_confirm_prompt = false;
                self.restart_to_update("changelog");
            }
            Some("cancel") => {
                log::info!("update: restart confirm cancelled");
                self.update_confirm_prompt = false;
            }
            _ => {}
        }
    }

    pub(crate) fn draw_changelog_overlay(&mut self, ctx: &egui::Context) {
        if !self.show_changelog {
            return;
        }
        const CHANGELOG: &str = include_str!("../../CHANGELOG.md");

        let response = crate::ui::overlay::ModalShell::centered("changelog")
            .title("Changelog")
            .width(560.0)
            .show_scroll_body(ctx, &self.colors.clone(), 420.0, "changelog_body", |ui| {
                self.changelog_update_status(ui);
                ui.add_space(style::SPACE_SM);

                for line in CHANGELOG.lines() {
                    if !should_render_changelog_body_line(line) {
                        continue;
                    }
                    let clean = |s: &str| s.replace("**", "");
                    if let Some(rest) = line.strip_prefix("## ") {
                        ui.add_space(style::SPACE_XS);
                        crate::ui::typography::body_strong(ui, clean(rest), &self.colors);
                        ui.add_space(2.0);
                    } else if let Some(rest) = line.strip_prefix("### ") {
                        crate::ui::typography::caption_strong(ui, clean(rest), &self.colors);
                    } else if line.starts_with("- ") || line.starts_with("* ") {
                        crate::ui::typography::body(ui, clean(line), &self.colors);
                    } else if line.starts_with('#') || line.trim().is_empty() {
                        // skip top-level # heading and blank lines
                    } else {
                        crate::ui::typography::caption(ui, clean(line), &self.colors);
                    }
                }
            });
        if response.dismissed {
            self.close_changelog();
        }
    }

    /// Shared text-input overlay: an `ActionModal` field dialog with the
    /// commit action, "Cancel" and, for folder targets, "Browse…".
    /// Dispatches commit to `OverlayTarget`.
    pub(crate) fn draw_text_input_overlay(&mut self, ctx: &egui::Context) {
        use crate::app::input_owner::OverlaySurface;
        use crate::app::OverlayTarget;
        use crate::ui::button::ButtonKind;
        use crate::ui::dialog::{ActionModal, DialogAction, DialogField, DialogShortcut};
        use crate::ui::focus::SurfaceKey;
        use egui::{Key, Modifiers};

        // Poll async folder-picker result.
        if let Some(rx) = &self.text_overlay_browse_rx {
            if let Ok(result) = rx.try_recv() {
                if let Some(path) = result {
                    let path_str = path.display().to_string();
                    log::info!("TextInputOverlay: browse selected path={path_str}");
                    if let Some((ref mut ov, _)) = self.text_overlay {
                        ov.buffer = path_str;
                    }
                }
                self.text_overlay_browse_rx = None;
            }
        }

        let Some((overlay, target)) = self.text_overlay.as_mut() else {
            return;
        };
        let commit_label = match target {
            OverlayTarget::ContextRoot(_) => "Set root",
        };
        let mut actions = vec![
            DialogAction::new("commit", commit_label, ButtonKind::Primary)
                .shortcut(DialogShortcut::new(&["Enter"], Modifiers::NONE, Key::Enter)),
            DialogAction::new("cancel", "Cancel", ButtonKind::Secondary)
                .refuses()
                .shortcut(DialogShortcut::new(&["Esc"], Modifiers::NONE, Key::Escape)),
        ];
        if matches!(target, OverlayTarget::ContextRoot(_)) {
            actions.push(DialogAction::new(
                "browse",
                "Browse\u{2026}",
                ButtonKind::Secondary,
            ));
        }
        let response = ActionModal::new("text_input", &overlay.label, &actions)
            .width(MODAL_WIDTH)
            .show_field(
                ctx,
                &self.colors,
                DialogField::singleline(
                    egui::Id::new("text_input_overlay_field"),
                    &overlay.hint,
                    SurfaceKey::Overlay(OverlaySurface::Layer(crate::app::FocusKind::TextInput)),
                    overlay.session,
                    &mut overlay.buffer,
                ),
            );
        let commit = response.selected == Some("commit");

        if response.selected == Some("cancel") || response.dismissed {
            log::info!("TextInputOverlay: cancelled");
            self.text_overlay = None;
            self.text_overlay_browse_rx = None;
            return;
        }

        if response.selected == Some("browse") && self.text_overlay_browse_rx.is_none() {
            log::info!("TextInputOverlay: browse folder picker opened");
            let (tx, rx) = std::sync::mpsc::channel();
            self.text_overlay_browse_rx = Some(rx);
            std::thread::spawn(move || {
                let result = pick_folder();
                let _ = tx.send(result);
            });
            return;
        }

        // Socket-injected input in this frame cannot commit a root. Refuse
        // before taking the draft, so a person's typed path survives it.
        if commit && self.synthetic_input_frame {
            if let Some((overlay, OverlayTarget::ContextRoot(id))) = self.text_overlay.as_mut() {
                log::info!(
                    "TextInputOverlay: root change for context {id} refused: synthetic input"
                );
                if !overlay.label.ends_with(SYNTHETIC_NOTE) {
                    overlay.label.push_str(SYNTHETIC_NOTE);
                }
                return;
            }
        }

        if commit {
            // Take overlay state for dispatch.
            let (overlay, target) = match self.text_overlay.take() {
                Some(pair) => pair,
                None => return,
            };
            self.text_overlay_browse_rx = None;
            let raw = overlay.buffer.trim().to_string();

            match target {
                OverlayTarget::ContextRoot(id) => {
                    let Some(current) = self.context_root_for(id) else {
                        log::info!("TextInputOverlay: context {id} no longer exists");
                        return;
                    };
                    if raw.is_empty() {
                        // A context is always anchored — an empty commit leaves
                        // the root unchanged rather than un-rooting the context.
                        log::info!("TextInputOverlay: empty root input for context {id}");
                    } else {
                        // Tilde expansion.
                        let expanded = if let Some(rest) = raw.strip_prefix("~/") {
                            if let Some(home) = dirs::home_dir() {
                                home.join(rest)
                            } else {
                                std::path::PathBuf::from(&raw)
                            }
                        } else if raw.starts_with('~') && raw.len() == 1 {
                            dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from(&raw))
                        } else {
                            std::path::PathBuf::from(&raw)
                        };

                        // Resolve relative paths against the context root.
                        // Skip resolution if the raw input started with ~ (failed
                        // tilde expansion should not be treated as relative).
                        let resolved = if expanded.is_relative() && !raw.starts_with('~') {
                            current.join(&expanded)
                        } else {
                            expanded
                        };

                        log::info!(
                            "TextInputOverlay: set context root id={id} path={}",
                            resolved.display()
                        );
                        if let Err(error) =
                            self.commit_context_root(resolved, id, window_chosen_root())
                        {
                            log::info!("TextInputOverlay: root change refused: {error}");
                        }
                    }
                }
            }
        }
    }

    /// The one single-line rename dialog: an `ActionModal` field dialog with
    /// "Rename" (Enter) and "Cancel" (Esc) under `title`, over `field`; `id`
    /// names the modal. Returns `(commit, cancel)`; the caller owns what
    /// committing means.
    fn draw_single_line_rename_overlay(
        ctx: &egui::Context,
        id: &str,
        title: &str,
        field: crate::ui::dialog::DialogField<'_>,
        colors: &Colors,
    ) -> (bool, bool) {
        use crate::ui::button::ButtonKind;
        use crate::ui::dialog::{ActionModal, DialogAction, DialogShortcut};
        use egui::{Key, Modifiers};
        let actions = [
            DialogAction::new("rename", "Rename", ButtonKind::Primary)
                .shortcut(DialogShortcut::new(&["Enter"], Modifiers::NONE, Key::Enter)),
            DialogAction::new("cancel", "Cancel", ButtonKind::Secondary)
                .refuses()
                .shortcut(DialogShortcut::new(&["Esc"], Modifiers::NONE, Key::Escape)),
        ];
        let response = ActionModal::new(id, title, &actions)
            .width(MODAL_WIDTH)
            .show_field(ctx, colors, field);
        (
            response.selected == Some("rename"),
            response.selected == Some("cancel") || response.dismissed,
        )
    }

    pub(crate) fn draw_rename_pane_overlay(&mut self, ctx: &egui::Context) {
        let pane_id: PaneId = match self.renaming_pane {
            Some(id) => id,
            None => return,
        };

        let (commit, cancel) = Self::draw_single_line_rename_overlay(
            ctx,
            "rename_pane",
            "Rename Pane",
            crate::ui::dialog::DialogField::singleline(
                egui::Id::new("rename_pane_input"),
                "Pane name...",
                crate::ui::focus::SurfaceKey::Overlay(
                    crate::app::input_owner::OverlaySurface::Layer(
                        crate::app::FocusKind::RenamePane,
                    ),
                ),
                self.rename_session,
                &mut self.rename_buffer,
            ),
            &self.colors,
        );

        if cancel {
            self.renaming_pane = None;
        } else if commit {
            let new_name = self.rename_buffer.trim().to_string();
            if let Some(pane) = self.windows[self.active_window].panes.get_mut(&pane_id) {
                if let Some(t) = pane.as_terminal_mut() {
                    t.name_locked = !new_name.is_empty();
                    t.name = if new_name.is_empty() {
                        None
                    } else {
                        log::info!("rename_pane: pane {pane_id} name locked to {:?}", new_name);
                        Some(new_name.clone())
                    };
                } else if let Some(a) = pane.as_view_mut() {
                    // Let the plugin react first — TextEditorView persists the new
                    // name into the note's frontmatter title.
                    a.runtime.on_pane_renamed(&new_name);
                    a.name = if new_name.is_empty() {
                        a.runtime.display_name()
                    } else {
                        new_name.clone()
                    };
                    log::info!("rename_pane: view pane {pane_id} named {:?}", a.name);
                }
            }
            let context_root = self
                .origin_for_pane(pane_id)
                .map(|origin| origin.context_root);
            crate::host::event_log::emit_scoped(
                crate::host::event_log::HostEvent::PaneRenamed {
                    pane_id,
                    name: new_name,
                    timestamp: crate::host::event_log::now_timestamp(),
                },
                context_root.as_deref(),
            );
            self.renaming_pane = None;
        }
    }

    /// Context rename overlay; Enter commits and Escape cancels.
    pub(crate) fn draw_rename_context_overlay(&mut self, ctx: &egui::Context) {
        let Some(edit) = self.context_rename.as_mut() else {
            return;
        };

        let (commit, cancel) = Self::draw_single_line_rename_overlay(
            ctx,
            "rename_context",
            "Name this context",
            crate::ui::dialog::DialogField::singleline(
                egui::Id::new("rename_context_input"),
                "Context name...",
                crate::ui::focus::SurfaceKey::Overlay(
                    crate::app::input_owner::OverlaySurface::Layer(
                        crate::app::FocusKind::ContextRename,
                    ),
                ),
                edit.session,
                &mut edit.buffer,
            ),
            &self.colors,
        );

        if cancel {
            self.cancel_context_rename("escape");
        } else if commit {
            self.commit_context_rename();
        }
    }

    /// Modal for editing a context's description: an `ActionModal` field
    /// dialog with a multi-line field, "Save" (Cmd+Enter) and "Cancel" (Esc).
    pub(crate) fn draw_edit_description_overlay(&mut self, ctx: &egui::Context) {
        use crate::ui::button::ButtonKind;
        use crate::ui::dialog::{ActionModal, DialogAction, DialogField, DialogShortcut};
        use egui::{Key, Modifiers};
        let ctx_idx = match self.editing_description {
            Some(idx) if idx < self.router.len() => idx,
            _ => {
                self.editing_description = None;
                self.pop_focus_layer(&crate::app::FocusKind::ContextDescription);
                return;
            }
        };

        let actions =
            [
                DialogAction::new("save", "Save", ButtonKind::Primary).shortcut(
                    DialogShortcut::new(&["cmd", "Enter"], Modifiers::COMMAND, Key::Enter),
                ),
                DialogAction::new("cancel", "Cancel", ButtonKind::Secondary)
                    .refuses()
                    .shortcut(DialogShortcut::new(&["Esc"], Modifiers::NONE, Key::Escape)),
            ];
        let ctx_name = self.router.get(ctx_idx).name.to_string();
        let title = format!("Description for \"{ctx_name}\"");
        let response = ActionModal::new("context_description", &title, &actions)
            .width(MODAL_WIDTH)
            .show_field(
                ctx,
                &self.colors,
                DialogField::multiline(
                    egui::Id::new("edit_description_input"),
                    "What are you working on in this context?",
                    crate::ui::focus::SurfaceKey::Overlay(
                        crate::app::input_owner::OverlaySurface::Layer(
                            crate::app::FocusKind::ContextDescription,
                        ),
                    ),
                    self.description_session,
                    3,
                    &mut self.description_buffer,
                ),
            );
        let commit = response.selected == Some("save");
        let cancel = response.selected == Some("cancel") || response.dismissed;

        if cancel {
            log::info!("context_description: edit cancelled ctx_idx={ctx_idx}");
            self.editing_description = None;
            self.pop_focus_layer(&crate::app::FocusKind::ContextDescription);
        } else if commit {
            let new_desc = self.description_buffer.trim().to_string();
            self.router.get_mut(ctx_idx).description = if new_desc.is_empty() {
                None
            } else {
                Some(new_desc)
            };
            self.editing_description = None;
            self.pop_focus_layer(&crate::app::FocusKind::ContextDescription);
            self.mark_workspace_dirty();
            log::info!("context_description: updated ctx_idx={ctx_idx}");
        }
    }

    pub(crate) fn rename_pane_handle_key(
        &mut self,
        _input: &mut crate::app::input_router::PlexiInput,
    ) -> crate::app::view_trait::KeyDisposition {
        crate::app::view_trait::KeyDisposition::Consumed
    }

    pub(crate) fn context_rename_handle_key(
        &mut self,
        _input: &mut crate::app::input_router::PlexiInput,
    ) -> crate::app::view_trait::KeyDisposition {
        crate::app::view_trait::KeyDisposition::Consumed
    }

    pub(crate) fn context_description_handle_key(
        &mut self,
        _input: &mut crate::app::input_router::PlexiInput,
    ) -> crate::app::view_trait::KeyDisposition {
        crate::app::view_trait::KeyDisposition::Consumed
    }

    pub(crate) fn text_input_handle_key(
        &mut self,
        _input: &mut crate::app::input_router::PlexiInput,
    ) -> crate::app::view_trait::KeyDisposition {
        crate::app::view_trait::KeyDisposition::Consumed
    }
}

#[cfg(test)]
mod tests {
    use super::{should_render_changelog_body_line, should_show_changelog_line};

    #[test]
    fn changelog_filter_hides_chore_entries() {
        assert!(!should_show_changelog_line("- chore: claim stint 0015"));
        assert!(!should_show_changelog_line(
            "- chore(stint): close task 0154"
        ));
        assert!(!should_show_changelog_line(
            "* **chore(website): regenerate CLI reference docs"
        ));
    }

    #[test]
    fn changelog_filter_keeps_non_chore_entries_and_headings() {
        assert!(should_show_changelog_line("## [0.1.0] — 2026-06-11"));
        assert!(should_show_changelog_line("### Changes"));
        assert!(should_show_changelog_line("- fix: route update checks"));
        assert!(should_show_changelog_line("- feature: add plugin launcher"));
    }

    #[test]
    fn changelog_body_skips_document_intro() {
        assert!(!should_render_changelog_body_line("# Changelog"));
        assert!(!should_render_changelog_body_line(
            "Newest releases appear first."
        ));
        assert!(should_render_changelog_body_line("## [0.1.0] — 2026-06-11"));
    }
}

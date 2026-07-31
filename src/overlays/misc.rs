use super::*;

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
                        // ── Left column ──────────────────────────────
                        ui.vertical(|ui| {
                            ui.set_width(288.0);

                            section_label(ui, "LAYOUT", colors);
                            egui::Grid::new("shortcuts_layout")
                                .num_columns(2)
                                .min_col_width(80.0)
                                .spacing([style::SPACE_SM, 4.0])
                                .show(ui, |ui| {
                                    let rows: &[(&[&str], &str)] = &[
                                        (&["\u{2318}", "N"], "New window right"),
                                        (&["\u{2318}", "\u{21E7}", "N"], "New context"),
                                        (&["\u{2318}", "\u{21E7}", "U"], "Park context"),
                                        (&["\u{2318}", "D"], "Split right"),
                                        (&["\u{2318}", "\u{21E7}", "D"], "Split down"),
                                        (&["\u{2318}", "\\"], "Split right (mirror)"),
                                        (&["\u{2318}", "\u{21E7}", "\\"], "Split below (mirror)"),
                                        (&["\u{2318}", "W"], "Close pane"),
                                        (&["\u{2318}", "\u{21E7}", "W"], "Close context"),
                                        (&["\u{2318}", "T"], "New tab"),
                                    ];
                                    for (keys, desc) in rows {
                                        crate::ui::shortcuts::key_combo(ui, keys, colors);
                                        shortcut_description(ui, desc, colors);
                                        ui.end_row();
                                    }
                                });

                            ui.add_space(style::SPACE_SM);

                            section_label(ui, "NAVIGATION", colors);
                            egui::Grid::new("shortcuts_nav")
                                .num_columns(2)
                                .min_col_width(80.0)
                                .spacing([style::SPACE_SM, 4.0])
                                .show(ui, |ui| {
                                    crate::ui::shortcuts::key_combo_list(
                                        ui,
                                        &[&["\u{2318}"], &["H", "J", "K", "L"]],
                                        None,
                                        colors,
                                    );
                                    shortcut_description(
                                        ui,
                                        "Move focus (panes & windows)",
                                        colors,
                                    );
                                    ui.end_row();

                                    crate::ui::shortcuts::key_combo_list(
                                        ui,
                                        &[
                                            &["\u{2318}", "\u{21E7}", "L"],
                                            &["\u{2318}", "\u{21E7}", "H"],
                                        ],
                                        None,
                                        colors,
                                    );
                                    shortcut_description(ui, "Next / prev tab", colors);
                                    ui.end_row();

                                    crate::ui::shortcuts::key_combo_list(
                                        ui,
                                        &[
                                            &["\u{2318}", "\u{21E7}", "K"],
                                            &["\u{2318}", "\u{21E7}", "J"],
                                        ],
                                        None,
                                        colors,
                                    );
                                    shortcut_description(ui, "First / last tab", colors);
                                    ui.end_row();

                                    crate::ui::shortcuts::key_combo_list(
                                        ui,
                                        &[&["\u{2318}", "["], &["\u{2318}", "]"]],
                                        None,
                                        colors,
                                    );
                                    shortcut_description(
                                        ui,
                                        "Focus history back / forward",
                                        colors,
                                    );
                                    ui.end_row();

                                    crate::ui::shortcuts::key_combo_list(
                                        ui,
                                        &[&["\u{2318}", "1"], &["\u{2318}", "9"]],
                                        None,
                                        colors,
                                    );
                                    shortcut_description(ui, "Switch context 1–9", colors);
                                    ui.end_row();

                                    crate::ui::shortcuts::key_combo(
                                        ui,
                                        &["\u{2318}", "\u{21A9}"],
                                        colors,
                                    );
                                    shortcut_description(ui, "Zoom pane / enter context", colors);
                                    ui.end_row();

                                    crate::ui::shortcuts::key_combo_list(
                                        ui,
                                        &[&["\u{2318}", "\u{2303}", "H", "J", "K", "L"]],
                                        None,
                                        colors,
                                    );
                                    shortcut_description(ui, "Swap pane (focus follows)", colors);
                                    ui.end_row();

                                    crate::ui::shortcuts::key_combo_list(
                                        ui,
                                        &[&[
                                            "\u{2318}", "\u{2303}", "\u{2325}", "H", "J", "K", "L",
                                        ]],
                                        None,
                                        colors,
                                    );
                                    shortcut_description(ui, "Send pane (focus stays)", colors);
                                    ui.end_row();
                                });
                        });

                        // ── Vertical divider ──────────────────────────
                        ui.add_space(style::SPACE_MD);
                        ui.separator();
                        ui.add_space(style::SPACE_MD);

                        // ── Right column ─────────────────────────────
                        ui.vertical(|ui| {
                            section_label(ui, "APPS & TOOLS", colors);
                            egui::Grid::new("shortcuts_apps")
                                .num_columns(2)
                                .min_col_width(80.0)
                                .spacing([style::SPACE_SM, 4.0])
                                .show(ui, |ui| {
                                    let rows: &[(&[&str], &str)] = &[
                                        (&["\u{2318}", "P"], "Command palette"),
                                        (&["\u{2318}", "E"], "File browser"),
                                        (&["\u{2318}", "0"], "Quick note"),
                                        (&["\u{2318}", "O"], "Notes picker"),
                                        (&["\u{2318}", "\u{21E7}", "\u{2423}"], "Scratch note"),
                                        (&["\u{2318}", "B"], "Toggle sidebar"),
                                        (&["\u{2318}", "\u{21E7}", "A"], "Notifications"),
                                        (&["\u{2318}", "\u{21E7}", "M"], "Toggle minimap"),
                                    ];
                                    for (keys, desc) in rows {
                                        crate::ui::shortcuts::key_combo(ui, keys, colors);
                                        shortcut_description(ui, desc, colors);
                                        ui.end_row();
                                    }
                                });

                            ui.add_space(style::SPACE_SM);

                            section_label(ui, "TERMINAL", colors);
                            egui::Grid::new("shortcuts_terminal")
                                .num_columns(2)
                                .min_col_width(80.0)
                                .spacing([style::SPACE_SM, 4.0])
                                .show(ui, |ui| {
                                    crate::ui::shortcuts::key_combo_list(
                                        ui,
                                        &[&["\u{2318}", "\u{2191}"], &["\u{2318}", "\u{2193}"]],
                                        None,
                                        colors,
                                    );
                                    shortcut_description(ui, "Scroll terminal", colors);
                                    ui.end_row();

                                    crate::ui::shortcuts::key_combo_list(
                                        ui,
                                        &[&["\u{2318}", "="], &["\u{2318}", "-"]],
                                        None,
                                        colors,
                                    );
                                    shortcut_description(ui, "Font size +/−", colors);
                                    ui.end_row();
                                });

                            ui.add_space(style::SPACE_SM);

                            section_label(ui, "SETTINGS", colors);
                            egui::Grid::new("shortcuts_settings")
                                .num_columns(2)
                                .min_col_width(80.0)
                                .spacing([style::SPACE_SM, 4.0])
                                .show(ui, |ui| {
                                    let rows: &[(&[&str], &str)] = &[
                                        (&["\u{2318}", ","], "Open config"),
                                        (&["\u{2318}", "\u{21E7}", ","], "Reload config"),
                                        (&["\u{2318}", "\u{21E7}", "S"], "Secrets manager"),
                                        (&["\u{2318}", "R"], "Rename pane"),
                                        (&["\u{2318}", "\u{21E7}", "R"], "Rename context"),
                                        (&["\u{2318}", "\u{21E7}", "I"], "Set context root"),
                                        (&["\u{2318}", "/"], "This help"),
                                        (&["\u{2318}", "Q"], "Quit"),
                                    ];
                                    for (keys, desc) in rows {
                                        crate::ui::shortcuts::key_combo(ui, keys, colors);
                                        shortcut_description(ui, desc, colors);
                                        ui.end_row();
                                    }
                                });
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

    pub(crate) fn draw_changelog_overlay(&mut self, ctx: &egui::Context) {
        if !self.show_changelog {
            return;
        }
        const CHANGELOG: &str = include_str!("../../CHANGELOG.md");

        let response = crate::ui::overlay::ModalShell::centered("changelog")
            .title("Changelog")
            .width(560.0)
            .escape(true)
            .show_scroll_body(ctx, &self.colors.clone(), 420.0, "changelog_body", |ui| {
                if let Some(ref latest) = self.update_available.clone() {
                    egui::Frame::new()
                        .fill(self.colors.accent.gamma_multiply(0.15))
                        .stroke(Stroke::new(1.0_f32, self.colors.accent.gamma_multiply(0.4)))
                        .corner_radius(R6)
                        .inner_margin(egui::Margin::symmetric(12, 8))
                        .show(ui, |ui| {
                            if self.update_confirm_prompt {
                                ui.vertical(|ui| {
                                    ui.label(
                                        RichText::new("Plexi will restart with the new version. Your workspace will be saved.")
                                            .size(style::TEXT_HINT)
                                            .color(self.colors.accent),
                                    );
                                    ui.add_space(4.0);
                                    ui.horizontal(|ui| {
                                        let confirm_btn = egui::Button::new(
                                            RichText::new(format!("Restart to v{latest}"))
                                                .size(style::TEXT_HINT)
                                                .color(self.colors.text_on(self.colors.accent)),
                                        )
                                        .fill(self.colors.accent)
                                        .corner_radius(egui::CornerRadius::same(4));
                                        if ui.add(confirm_btn).clicked() {
                                            self.update_confirm_prompt = false;
                                            self.update_quit_pending = true;
                                        }
                                        let cancel_btn = egui::Button::new(
                                            RichText::new("Cancel")
                                                .size(style::TEXT_HINT)
                                                .color(self.colors.text_dim),
                                        )
                                        .fill(egui::Color32::TRANSPARENT)
                                        .stroke(Stroke::new(1.0_f32, self.colors.border))
                                        .corner_radius(egui::CornerRadius::same(4));
                                        if ui.add(cancel_btn).clicked() {
                                            self.update_confirm_prompt = false;
                                        }
                                    });
                                });
                            } else {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(format!("v{latest} is ready"))
                                            .size(style::TEXT_HINT)
                                            .color(self.colors.accent),
                                    );
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            let btn = egui::Button::new(
                                                RichText::new("Restart to update")
                                                    .size(style::TEXT_HINT)
                                                    .color(self.colors.text_on(self.colors.accent)),
                                            )
                                            .fill(self.colors.accent)
                                            .corner_radius(egui::CornerRadius::same(4));
                                            if ui.add(btn).clicked() {
                                                self.update_confirm_prompt = true;
                                            }
                                        },
                                    );
                                });
                            }
                        });
                    ui.add_space(style::SPACE_SM);
                }

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
            self.show_changelog = false;
        }
    }

    /// Shared text-input overlay. Renders a centered modal with a text field,
    /// optional "Browse..." button (for folder targets), and Enter/Escape
    /// commit/cancel. Dispatches commit to `OverlayTarget`.
    pub(crate) fn draw_text_input_overlay(&mut self, ctx: &egui::Context) {
        use crate::app::OverlayTarget;

        if self.text_overlay.is_none() {
            return;
        }

        // Consume Enter/Escape before the Area so they don't bleed through.
        let (commit, cancel) = ctx.input_mut(|i| {
            let enter = i.consume_key(egui::Modifiers::NONE, egui::Key::Enter);
            let esc = i.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
            (enter, esc)
        });

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

        // Re-borrow after potential mutation above.
        let (overlay, target) = match self.text_overlay.as_mut() {
            Some(pair) => pair,
            None => return,
        };
        let label = overlay.label.clone();
        let hint = overlay.hint.clone();
        let show_browse = matches!(target, OverlayTarget::ContextRoot(_));
        let mut browse_clicked = false;

        // Kit shell. Escape/Enter stay caller-side (consumed above, before
        // the Area, so they can't bleed into panes); the old scrim had no
        // click-away either, so keep that off.
        let colors = self.colors;
        crate::ui::overlay::ModalShell::centered("text_input_overlay")
            .title(&label)
            .width(MODAL_WIDTH)
            .click_away(false)
            .show(ctx, &colors, |ui| {
                let te_id = egui::Id::new("text_input_overlay_field");
                let (overlay, _target) = self.text_overlay.as_mut().unwrap();
                crate::ui::text_field::TextField::singleline(te_id, hint.as_str())
                    .surface(crate::ui::focus::SurfaceKey::Overlay(
                        crate::app::input_owner::OverlaySurface::Layer(
                            crate::app::FocusKind::TextInput,
                        ),
                    ))
                    .select_all_on_focus(true)
                    .log_name("TextInputOverlay")
                    .show(ui, &mut overlay.buffer, &self.colors);

                if show_browse {
                    ui.add_space(style::SPACE_SM);
                    ui.horizontal(|ui| {
                        if crate::ui::button::chrome_button(
                            ui,
                            "Browse\u{2026}",
                            crate::ui::button::ButtonKind::Primary,
                            &self.colors,
                            80.0,
                        )
                        .clicked()
                        {
                            browse_clicked = true;
                        }
                    });
                }
                let hints = [
                    crate::ui::hints::HintGroup::new(&["Enter"], "confirm"),
                    crate::ui::hints::HintGroup::new(&["Esc"], "cancel"),
                ];
                crate::ui::hints::HintBar::new(&hints).show(ui, &self.colors);
            });

        if cancel {
            log::info!("TextInputOverlay: cancelled");
            self.text_overlay = None;
            self.text_overlay_browse_rx = None;
            return;
        }

        if browse_clicked && self.text_overlay_browse_rx.is_none() {
            log::info!("TextInputOverlay: browse folder picker opened");
            let (tx, rx) = std::sync::mpsc::channel();
            self.text_overlay_browse_rx = Some(rx);
            std::thread::spawn(move || {
                let result = pick_folder();
                let _ = tx.send(result);
            });
            return;
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
                OverlayTarget::ContextRoot(idx) => {
                    if idx >= self.router.len() {
                        log::warn!("TextInputOverlay: context idx={idx} no longer exists");
                        return;
                    }
                    if raw.is_empty() {
                        // A context is always anchored — an empty commit leaves
                        // the root unchanged rather than un-rooting the context.
                        log::info!("TextInputOverlay: empty root input idx={idx} — root unchanged");
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
                            self.router.get(idx).root.join(&expanded)
                        } else {
                            expanded
                        };

                        log::info!(
                            "TextInputOverlay: set context root idx={idx} path={}",
                            resolved.display()
                        );
                        let ctx_id = self.router.get(idx).context_id;
                        self.set_context_root(resolved, Some(ctx_id));
                    }
                    self.save_workspace();
                }
            }
        }
    }

    pub(crate) fn draw_rename_pane_overlay(&mut self, ctx: &egui::Context) {
        let pane_id: PaneId = match self.renaming_pane {
            Some(id) => id,
            None => return,
        };

        // Consume Enter/Escape at the context level so they cannot bleed into
        // the focused pane this frame. Pairs with `FocusKind::RenamePane` —
        // the overlay owns its own commit/cancel keys.
        let (commit, cancel) = ctx.input_mut(|i| {
            let enter = i.consume_key(egui::Modifiers::NONE, egui::Key::Enter);
            let esc = i.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
            (enter, esc)
        });

        // Top-hung, scrim-less popover: the pane being renamed stays visible.
        // Enter/Escape stay caller-side (consumed above, before the Area).
        let colors = self.colors;
        crate::ui::overlay::ModalShell::centered("rename_pane_overlay")
            .title("Rename Pane")
            .width(MODAL_WIDTH)
            .anchor(Align2::CENTER_TOP, Vec2::new(0.0, 80.0))
            .scrim(false)
            .show(ctx, &colors, |ui| {
                let te_id = egui::Id::new("rename_pane_input");
                crate::ui::text_field::TextField::singleline(te_id, "Pane name...")
                    .surface(crate::ui::focus::SurfaceKey::Overlay(
                        crate::app::input_owner::OverlaySurface::Layer(
                            crate::app::FocusKind::RenamePane,
                        ),
                    ))
                    .select_all_on_focus(true)
                    .log_name("rename_pane")
                    .show(ui, &mut self.rename_buffer, &self.colors);
            });

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
                } else if let Some(a) = pane.as_app_mut() {
                    // Let the app react first — TextEditorApp persists the new
                    // name into the note's frontmatter title.
                    a.runtime.on_pane_renamed(&new_name);
                    a.name = if new_name.is_empty() {
                        a.runtime.display_name()
                    } else {
                        new_name.clone()
                    };
                    log::info!("rename_pane: app pane {pane_id} named {:?}", a.name);
                }
            }
            crate::host::event_log::emit(crate::host::event_log::HostEvent::PaneRenamed {
                pane_id,
                name: new_name,
                timestamp: crate::host::event_log::now_timestamp(),
            });
            self.renaming_pane = None;
        }
    }

    /// Modal for naming a newly-created context when the sidebar is hidden.
    /// Mirrors the inline sidebar rename but as a centred overlay so the
    /// terminal becomes immediately usable after the user hits Enter or Escape.
    pub(crate) fn draw_rename_context_overlay(&mut self, ctx: &egui::Context) {
        let ctx_idx = match self.renaming_window {
            Some(idx) => idx,
            None => return,
        };

        let (commit, cancel) = ctx.input_mut(|i| {
            let enter = i.consume_key(egui::Modifiers::NONE, egui::Key::Enter);
            let esc = i.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
            (enter, esc)
        });

        let colors = self.colors;
        crate::ui::overlay::ModalShell::centered("rename_context_overlay")
            .title("Name this context")
            .width(MODAL_WIDTH)
            .anchor(Align2::CENTER_TOP, Vec2::new(0.0, 80.0))
            .scrim(false)
            .show(ctx, &colors, |ui| {
                let te_id = egui::Id::new("rename_context_input");
                crate::ui::text_field::TextField::singleline(te_id, "Context name...")
                    .surface(crate::ui::focus::SurfaceKey::Overlay(
                        crate::app::input_owner::OverlaySurface::Layer(
                            crate::app::FocusKind::ContextRename,
                        ),
                    ))
                    .select_all_on_focus(true)
                    .log_name("rename_context")
                    .show(ui, &mut self.rename_buffer, &self.colors);
            });

        if cancel {
            self.renaming_window = None;
        } else if commit {
            let new_name = self.rename_buffer.trim().to_string();
            if !new_name.is_empty() {
                self.router.get_mut(ctx_idx).name = new_name;
                let context_id = self.router.get(ctx_idx).context_id;
                let name = self.router.get(ctx_idx).name.clone();
                crate::host::event_log::emit(crate::host::event_log::HostEvent::ContextRenamed {
                    context_id,
                    name,
                    timestamp: crate::host::event_log::now_timestamp(),
                });
            }
            self.renaming_window = None;
        }
    }

    /// Modal for editing a context's description. Multiline text input with
    /// Cmd+Enter to save, Esc to cancel.
    pub(crate) fn draw_edit_description_overlay(&mut self, ctx: &egui::Context) {
        let ctx_idx = match self.editing_description {
            Some(idx) if idx < self.router.len() => idx,
            _ => {
                self.editing_description = None;
                self.pop_focus_layer(&crate::app::FocusKind::ContextDescription);
                return;
            }
        };

        let (commit, cancel) = ctx.input_mut(|i| {
            let cmd_enter = i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter);
            let esc = i.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
            (cmd_enter, esc)
        });

        let colors = self.colors;
        let ctx_name = self.router.get(ctx_idx).name.clone();
        let title = format!("Description for \"{ctx_name}\"");
        crate::ui::overlay::ModalShell::centered("edit_description_overlay")
            .title(&title)
            .width(MODAL_WIDTH)
            .anchor(Align2::CENTER_TOP, Vec2::new(0.0, 80.0))
            .scrim(false)
            .show(ctx, &colors, |ui| {
                let te_id = egui::Id::new("edit_description_input");
                crate::ui::text_field::TextArea::multiline(
                    te_id,
                    "What are you working on in this context?",
                )
                .font(egui::TextStyle::Body.resolve(ui.style()))
                .desired_width(MODAL_WIDTH)
                .rows(3)
                .surface(crate::ui::focus::SurfaceKey::Overlay(
                    crate::app::input_owner::OverlaySurface::Layer(
                        crate::app::FocusKind::ContextDescription,
                    ),
                ))
                .select_all_on_focus(true)
                .log_name("context_description")
                .show(ui, &mut self.description_buffer, &self.colors);

                let hints = [
                    crate::ui::hints::HintGroup::new(&["\u{2318}", "Enter"], "save"),
                    crate::ui::hints::HintGroup::new(&["Esc"], "cancel"),
                ];
                crate::ui::hints::HintBar::new(&hints).show(ui, &self.colors);
            });

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
            self.save_workspace();
            log::info!("context_description: updated ctx_idx={ctx_idx}");
        }
    }

    pub(crate) fn rename_pane_handle_key(
        &mut self,
        _input: &mut crate::app::input_router::PlexiInput,
    ) -> crate::app::app_trait::KeyDisposition {
        crate::app::app_trait::KeyDisposition::Consumed
    }

    pub(crate) fn context_rename_handle_key(
        &mut self,
        _input: &mut crate::app::input_router::PlexiInput,
    ) -> crate::app::app_trait::KeyDisposition {
        crate::app::app_trait::KeyDisposition::Consumed
    }

    pub(crate) fn context_description_handle_key(
        &mut self,
        _input: &mut crate::app::input_router::PlexiInput,
    ) -> crate::app::app_trait::KeyDisposition {
        crate::app::app_trait::KeyDisposition::Consumed
    }

    pub(crate) fn text_input_handle_key(
        &mut self,
        _input: &mut crate::app::input_router::PlexiInput,
    ) -> crate::app::app_trait::KeyDisposition {
        crate::app::app_trait::KeyDisposition::Consumed
    }

    pub(crate) fn command_palette_handle_key(
        &mut self,
        _input: &mut crate::app::input_router::PlexiInput,
    ) -> crate::app::app_trait::KeyDisposition {
        crate::app::app_trait::KeyDisposition::Consumed
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
        assert!(should_show_changelog_line("- feature: add app launcher"));
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

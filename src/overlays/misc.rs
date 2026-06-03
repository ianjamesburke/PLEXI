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

impl PlexiApp {
    pub(crate) fn draw_shortcuts_overlay(&mut self, ctx: &egui::Context) {
        if !self.show_shortcuts {
            return;
        }
        let dismissed = crate::widgets::dismissable_modal(ctx, "shortcuts", |ui| {
            egui::Frame::new()
                .fill(self.colors.bg_sidebar.gamma_multiply(0.95))
                .stroke(Stroke::new(1.0, self.colors.border))
                .corner_radius(R6)
                .inner_margin(egui::Margin::symmetric(24, 20))
                .show(ui, |ui| {
                    ui.set_width(style::MODAL_WIDTH_MD);

                    ui.vertical_centered(|ui| {
                        ui.label(
                            RichText::new("Keyboard Shortcuts")
                                .size(13.0)
                                .color(self.colors.text_primary)
                                .strong(),
                        );
                    });
                    ui.add_space(style::SPACE_MD);

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
                                        (&["\u{2318}", "D"], "Split right"),
                                        (&["\u{2318}", "\u{21E7}", "D"], "Split down"),
                                        (&["\u{2318}", "\\"], "Split right (mirror)"),
                                        (&["\u{2318}", "\u{21E7}", "\\"], "Split below (mirror)"),
                                        (&["\u{2318}", "W"], "Close pane"),
                                        (&["\u{2318}", "T"], "New tab"),
                                    ];
                                    for (keys, desc) in rows {
                                        crate::widgets::key_combo(ui, keys, colors);
                                        ui.label(
                                            RichText::new(*desc)
                                                .size(style::TEXT_HINT)
                                                .color(colors.text_dim),
                                        );
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
                                    crate::widgets::key_combo_list(
                                        ui,
                                        &[&["\u{2318}"], &["H", "J", "K", "L"]],
                                        None,
                                        colors,
                                    );
                                    ui.label(
                                        RichText::new("Move focus (panes & windows)")
                                            .size(style::TEXT_HINT)
                                            .color(colors.text_dim),
                                    );
                                    ui.end_row();

                                    crate::widgets::key_combo_list(
                                        ui,
                                        &[&["\u{2318}", "\u{21E7}", "L"], &["\u{2318}", "\u{21E7}", "H"]],
                                        None,
                                        colors,
                                    );
                                    ui.label(
                                        RichText::new("Next / prev tab")
                                            .size(style::TEXT_HINT)
                                            .color(colors.text_dim),
                                    );
                                    ui.end_row();

                                    crate::widgets::key_combo_list(
                                        ui,
                                        &[&["\u{2318}", "\u{21E7}", "K"], &["\u{2318}", "\u{21E7}", "J"]],
                                        None,
                                        colors,
                                    );
                                    ui.label(
                                        RichText::new("First / last tab")
                                            .size(style::TEXT_HINT)
                                            .color(colors.text_dim),
                                    );
                                    ui.end_row();

                                    crate::widgets::key_combo_list(
                                        ui,
                                        &[&["\u{2318}", "["], &["\u{2318}", "]"]],
                                        None,
                                        colors,
                                    );
                                    ui.label(
                                        RichText::new("Focus history back / forward")
                                            .size(style::TEXT_HINT)
                                            .color(colors.text_dim),
                                    );
                                    ui.end_row();

                                    crate::widgets::key_combo_list(
                                        ui,
                                        &[&["\u{2318}", "1"], &["\u{2318}", "9"]],
                                        None,
                                        colors,
                                    );
                                    ui.label(
                                        RichText::new("Switch context 1–9")
                                            .size(style::TEXT_HINT)
                                            .color(colors.text_dim),
                                    );
                                    ui.end_row();

                                    crate::widgets::key_combo(
                                        ui,
                                        &["\u{2318}", "\u{21A9}"],
                                        colors,
                                    );
                                    ui.label(
                                        RichText::new("Zoom pane / enter context")
                                            .size(style::TEXT_HINT)
                                            .color(colors.text_dim),
                                    );
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
                                        (&["\u{2318}", "B"], "Toggle sidebar"),
                                        (&["\u{2318}", "\u{21E7}", "A"], "Notifications"),
                                        (&["\u{2318}", "\u{21E7}", "M"], "Toggle minimap"),
                                    ];
                                    for (keys, desc) in rows {
                                        crate::widgets::key_combo(ui, keys, colors);
                                        ui.label(
                                            RichText::new(*desc)
                                                .size(style::TEXT_HINT)
                                                .color(colors.text_dim),
                                        );
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
                                    crate::widgets::key_combo_list(
                                        ui,
                                        &[&["\u{2318}", "\u{2191}"], &["\u{2318}", "\u{2193}"]],
                                        None,
                                        colors,
                                    );
                                    ui.label(
                                        RichText::new("Scroll terminal")
                                            .size(style::TEXT_HINT)
                                            .color(colors.text_dim),
                                    );
                                    ui.end_row();

                                    crate::widgets::key_combo_list(
                                        ui,
                                        &[&["\u{2318}", "="], &["\u{2318}", "-"]],
                                        None,
                                        colors,
                                    );
                                    ui.label(
                                        RichText::new("Font size +/−")
                                            .size(style::TEXT_HINT)
                                            .color(colors.text_dim),
                                    );
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
                                        (&["\u{2318}", "/"], "This help"),
                                        (&["\u{2318}", "Q"], "Quit"),
                                    ];
                                    for (keys, desc) in rows {
                                        crate::widgets::key_combo(ui, keys, colors);
                                        ui.label(
                                            RichText::new(*desc)
                                                .size(style::TEXT_HINT)
                                                .color(colors.text_dim),
                                        );
                                        ui.end_row();
                                    }
                                });
                        });
                    });

                    ui.add_space(style::SPACE_MD);
                    ui.separator();
                    ui.add_space(style::SPACE_SM);
                    draw_contact_footer(ui, colors);
                });
        });
        if dismissed {
            self.show_shortcuts = false;
        }
    }

    pub(crate) fn draw_changelog_overlay(&mut self, ctx: &egui::Context) {
        if !self.show_changelog {
            return;
        }
        const CHANGELOG: &str = include_str!("../../CHANGELOG.md");

        let dismissed = crate::widgets::dismissable_modal(ctx, "changelog", |ui| {
            egui::Frame::new()
                .fill(self.colors.bg_sidebar.gamma_multiply(0.95))
                .stroke(Stroke::new(1.0, self.colors.border))
                .corner_radius(R6)
                .inner_margin(egui::Margin::symmetric(24, 20))
                .show(ui, |ui| {
                    ui.set_width(560.0);
                    ui.set_max_height(480.0);

                    ui.label(
                        RichText::new("Changelog")
                            .size(13.0)
                            .color(self.colors.text_primary)
                            .strong(),
                    );

                    if let Some(ref latest) = self.update_available.clone() {
                        ui.add_space(8.0);
                        egui::Frame::new()
                            .fill(self.colors.accent.gamma_multiply(0.15))
                            .stroke(Stroke::new(1.0, self.colors.accent.gamma_multiply(0.4)))
                            .corner_radius(R6)
                            .inner_margin(egui::Margin::symmetric(12, 8))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(format!("Run plexi update to upgrade to v{latest}"))
                                            .size(style::TEXT_HINT)
                                            .color(self.colors.accent),
                                    );
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            crate::widgets::copy_button(
                                                ui,
                                                egui::Id::new("update_banner_copy"),
                                                "plexi update",
                                            );
                                        },
                                    );
                                });
                            });
                    }

                    ui.add_space(8.0);

                    egui::ScrollArea::vertical()
                        .max_height(420.0)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            ui.set_width(512.0);
                            for line in CHANGELOG.lines() {
                                let clean = |s: &str| s.replace("**", "");
                                if line.starts_with("## ") {
                                    ui.add_space(6.0);
                                    ui.label(
                                        RichText::new(clean(&line[3..]))
                                            .size(style::TEXT_BODY)
                                            .color(self.colors.text_primary)
                                            .strong(),
                                    );
                                    ui.add_space(2.0);
                                } else if line.starts_with("### ") {
                                    ui.label(
                                        RichText::new(clean(&line[4..]))
                                            .size(style::TEXT_HINT)
                                            .color(self.colors.text_dim)
                                            .strong(),
                                    );
                                } else if line.starts_with("- ") || line.starts_with("* ") {
                                    ui.label(
                                        RichText::new(clean(line))
                                            .size(style::TEXT_HINT)
                                            .color(self.colors.text_primary),
                                    );
                                } else if line.starts_with('#') || line.trim().is_empty() {
                                    // skip top-level # heading and blank lines
                                } else {
                                    ui.label(
                                        RichText::new(clean(line))
                                            .size(style::TEXT_HINT)
                                            .color(self.colors.text_dim),
                                    );
                                }
                            }
                        });
                });
        });
        if dismissed {
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

        // Scrim
        let screen_rect = ctx.screen_rect();
        egui::Area::new(egui::Id::new("text_input_overlay_scrim"))
            .fixed_pos(screen_rect.min)
            .order(egui::Order::Middle)
            .show(ctx, |ui| {
                ui.painter().rect_filled(
                    screen_rect,
                    0.0,
                    Color32::from_black_alpha(style::SCRIM_ALPHA),
                );
            });

        egui::Area::new(egui::Id::new("text_input_overlay"))
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(self.colors.bg_sidebar)
                    .stroke(Stroke::new(1.0, self.colors.border))
                    .corner_radius(style::RADIUS_MD)
                    .inner_margin(egui::Margin::symmetric(
                        style::MODAL_PADDING_H,
                        style::MODAL_PADDING_V,
                    ))
                    .show(ui, |ui| {
                        ui.set_width(MODAL_WIDTH);
                        ui.label(
                            RichText::new(&label)
                                .size(13.0)
                                .color(self.colors.text_primary)
                                .strong(),
                        );
                        ui.add_space(6.0);

                        let te_id = egui::Id::new("text_input_overlay_field");
                        let (overlay, _target) = self.text_overlay.as_mut().unwrap();
                        let te = crate::widgets::styled_text_input(
                            ui,
                            &mut overlay.buffer,
                            hint.as_str(),
                            te_id,
                            &self.colors,
                        );

                        // One-shot focus guard.
                        if !overlay.focus_requested {
                            te.request_focus();
                            if let Some(mut state) =
                                egui::TextEdit::load_state(ui.ctx(), te_id)
                            {
                                state
                                    .cursor
                                    .set_char_range(Some(egui::text::CCursorRange::two(
                                        egui::text::CCursor::new(0),
                                        egui::text::CCursor::new(overlay.buffer.len()),
                                    )));
                                state.store(ui.ctx(), te_id);
                            }
                            overlay.focus_requested = true;
                            log::info!("TextInputOverlay: focus requested");
                        }

                        if show_browse {
                            ui.add_space(style::SPACE_SM);
                            ui.horizontal(|ui| {
                                if ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new("Browse\u{2026}")
                                                .size(style::TEXT_CAPTION)
                                                .color(self.colors.text_primary),
                                        )
                                        .fill(self.colors.bg_active),
                                    )
                                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                                    .clicked()
                                {
                                    browse_clicked = true;
                                }
                                ui.add_space(style::SPACE_SM);
                                ui.label(
                                    RichText::new("Enter to confirm, Esc to cancel")
                                        .size(style::TEXT_CAPTION)
                                        .color(self.colors.text_dim),
                                );
                            });
                        }
                    });
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
                        log::info!("TextInputOverlay: clear context root idx={idx}");
                        self.router.get_mut(idx).root = None;
                    } else {
                        // Tilde expansion.
                        let expanded = if raw.starts_with("~/") {
                            if let Some(home) = dirs::home_dir() {
                                home.join(&raw[2..])
                            } else {
                                std::path::PathBuf::from(&raw)
                            }
                        } else if raw.starts_with('~') && raw.len() == 1 {
                            dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from(&raw))
                        } else {
                            std::path::PathBuf::from(&raw)
                        };

                        // Resolve relative paths against context path.
                        // Skip resolution if the raw input started with ~ (failed
                        // tilde expansion should not be treated as relative).
                        let resolved = if expanded.is_relative() && !raw.starts_with('~') {
                            self.router.get(idx).path.join(&expanded)
                        } else {
                            expanded
                        };

                        log::info!(
                            "TextInputOverlay: set context root idx={idx} path={}",
                            resolved.display()
                        );
                        self.router.get_mut(idx).root = Some(resolved);
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
        // the focused pane this frame. Pairs with `FocusLayer::RenamePane` —
        // the overlay owns its own commit/cancel keys.
        let (commit, cancel) = ctx.input_mut(|i| {
            let enter = i.consume_key(egui::Modifiers::NONE, egui::Key::Enter);
            let esc = i.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
            (enter, esc)
        });

        egui::Area::new(egui::Id::new("rename_pane_overlay"))
            .anchor(Align2::CENTER_TOP, Vec2::new(0.0, 80.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(self.colors.bg_sidebar)
                    .stroke(Stroke::new(1.0, self.colors.border))
                    .corner_radius(style::RADIUS_LG)
                    .inner_margin(egui::Margin::symmetric(16, 12))
                    .show(ui, |ui| {
                        ui.set_width(MODAL_WIDTH);
                        ui.label(
                            RichText::new("Rename Pane")
                                .size(13.0)
                                .color(self.colors.text_primary)
                                .strong(),
                        );
                        ui.add_space(6.0);

                        let te_id = egui::Id::new("rename_pane_input");
                        let te = crate::widgets::styled_text_input(
                            ui,
                            &mut self.rename_buffer,
                            "Pane name...",
                            te_id,
                            &self.colors,
                        );

                        // One-shot focus: only request on the first render frame.
                        // Re-requesting every frame (guarded by `!te.has_focus()`) lets
                        // any widget rendered later in the same frame steal focus
                        // permanently — egui resolves `request_focus()` conflicts
                        // last-caller-wins within a frame.
                        if !self.rename_pane_focus_requested {
                            te.request_focus();
                            if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), te_id) {
                                state
                                    .cursor
                                    .set_char_range(Some(egui::text::CCursorRange::two(
                                        egui::text::CCursor::new(0),
                                        egui::text::CCursor::new(self.rename_buffer.len()),
                                    )));
                                state.store(ui.ctx(), te_id);
                            }
                            self.rename_pane_focus_requested = true;
                            log::info!("rename_pane: focus requested for TextEdit");
                        }
                    });
            });

        if cancel {
            self.renaming_pane = None;
            self.rename_pane_focus_requested = false;
        } else if commit {
            let new_name = self.rename_buffer.trim().to_string();
            if let Some(pane) =
                self.windows[self.active_window].panes.get_mut(&pane_id)
            {
                if let Some(t) = pane.as_terminal_mut() {
                    t.name_locked = !new_name.is_empty();
                    t.name = if new_name.is_empty() {
                        None
                    } else {
                        log::info!("rename_pane: pane {pane_id} name locked to {:?}", new_name);
                        Some(new_name)
                    };
                }
            }
            self.renaming_pane = None;
            self.rename_pane_focus_requested = false;
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

        egui::Area::new(egui::Id::new("rename_context_overlay"))
            .anchor(Align2::CENTER_TOP, Vec2::new(0.0, 80.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(self.colors.bg_sidebar)
                    .stroke(Stroke::new(1.0, self.colors.border))
                    .corner_radius(style::RADIUS_LG)
                    .inner_margin(egui::Margin::symmetric(16, 12))
                    .show(ui, |ui| {
                        ui.set_width(MODAL_WIDTH);
                        ui.label(
                            RichText::new("Name this context")
                                .size(13.0)
                                .color(self.colors.text_primary)
                                .strong(),
                        );
                        ui.add_space(6.0);

                        let te_id = egui::Id::new("rename_context_input");
                        let te = crate::widgets::styled_text_input(
                            ui,
                            &mut self.rename_buffer,
                            "Context name...",
                            te_id,
                            &self.colors,
                        );

                        if !te.has_focus() {
                            te.request_focus();
                            if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), te_id) {
                                state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                                    egui::text::CCursor::new(0),
                                    egui::text::CCursor::new(self.rename_buffer.len()),
                                )));
                                state.store(ui.ctx(), te_id);
                            }
                        }
                    });
            });

        if cancel {
            self.renaming_window = None;
        } else if commit {
            let new_name = self.rename_buffer.trim().to_string();
            if !new_name.is_empty() {
                self.router.get_mut(ctx_idx).name = new_name;
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
                self.pop_focus_layer(&crate::app::FocusLayer::ContextDescription);
                return;
            }
        };

        let (commit, cancel) = ctx.input_mut(|i| {
            let cmd_enter = i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter);
            let esc = i.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
            (cmd_enter, esc)
        });

        egui::Area::new(egui::Id::new("edit_description_overlay"))
            .anchor(Align2::CENTER_TOP, Vec2::new(0.0, 80.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(self.colors.bg_sidebar)
                    .stroke(Stroke::new(1.0, self.colors.border))
                    .corner_radius(R6)
                    .inner_margin(egui::Margin::symmetric(16, 12))
                    .show(ui, |ui| {
                        ui.set_width(MODAL_WIDTH);
                        let ctx_name = self.router.get(ctx_idx).name.clone();
                        ui.label(
                            RichText::new(format!("Description for \"{}\"", ctx_name))
                                .size(13.0)
                                .color(self.colors.text_primary)
                                .strong(),
                        );
                        ui.add_space(6.0);

                        let te_id = egui::Id::new("edit_description_input");
                        let te = ui.scope(|ui| {
                            ui.visuals_mut().text_cursor.stroke.width = 1.5;
                            ui.visuals_mut().text_cursor.stroke.color = self.colors.accent;
                            ui.visuals_mut().extreme_bg_color = self.colors.bg_active;
                            ui.visuals_mut().widgets.active.bg_stroke = egui::Stroke::new(1.0, self.colors.accent);
                            ui.visuals_mut().widgets.inactive.bg_stroke = egui::Stroke::new(1.0, self.colors.border);
                            ui.add(
                                egui::TextEdit::multiline(&mut self.description_buffer)
                                    .id(te_id)
                                    .desired_width(MODAL_WIDTH)
                                    .desired_rows(3)
                                    .hint_text("What are you working on in this context?")
                                    .font(egui::TextStyle::Body)
                                    .margin(egui::Margin::symmetric(8, 5)),
                            )
                        }).inner;

                        ui.add_space(4.0);
                        ui.label(
                            RichText::new("Cmd+Enter to save  ·  Esc to cancel")
                                .size(11.0)
                                .color(self.colors.text_dim),
                        );

                        if !self.description_focus_requested {
                            te.request_focus();
                            if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), te_id) {
                                state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                                    egui::text::CCursor::new(0),
                                    egui::text::CCursor::new(self.description_buffer.len()),
                                )));
                                state.store(ui.ctx(), te_id);
                            }
                            self.description_focus_requested = true;
                        }
                    });
            });

        if cancel {
            log::info!("context_description: edit cancelled ctx_idx={ctx_idx}");
            self.editing_description = None;
            self.description_focus_requested = false;
            self.pop_focus_layer(&crate::app::FocusLayer::ContextDescription);
        } else if commit {
            let new_desc = self.description_buffer.trim().to_string();
            self.router.get_mut(ctx_idx).description = if new_desc.is_empty() { None } else { Some(new_desc) };
            self.editing_description = None;
            self.description_focus_requested = false;
            self.pop_focus_layer(&crate::app::FocusLayer::ContextDescription);
            self.save_workspace();
            log::info!("context_description: updated ctx_idx={ctx_idx}");
        }
    }

    pub(crate) fn rename_pane_handle_key(
        &mut self,
        _ctx: &egui::Context,
    ) -> crate::app_trait::KeyDisposition {
        crate::app_trait::KeyDisposition::Consumed
    }

    pub(crate) fn context_rename_handle_key(
        &mut self,
        _ctx: &egui::Context,
    ) -> crate::app_trait::KeyDisposition {
        crate::app_trait::KeyDisposition::Consumed
    }

    pub(crate) fn context_description_handle_key(
        &mut self,
        _ctx: &egui::Context,
    ) -> crate::app_trait::KeyDisposition {
        crate::app_trait::KeyDisposition::Consumed
    }

    pub(crate) fn text_input_handle_key(
        &mut self,
        _ctx: &egui::Context,
    ) -> crate::app_trait::KeyDisposition {
        crate::app_trait::KeyDisposition::Consumed
    }

    pub(crate) fn command_palette_handle_key(
        &mut self,
        _ctx: &egui::Context,
    ) -> crate::app_trait::KeyDisposition {
        crate::app_trait::KeyDisposition::Consumed
    }
}

use crate::app_trait::AppCommand;
use crate::style;
use crate::theme::Colors;
use crate::tiling::PaneId;
use egui::{Align, Align2, Color32, CornerRadius, Layout, RichText, Stroke, Vec2};

use crate::app::PlexiApp;

pub(crate) const MODAL_WIDTH: f32 = 400.0;
const R6: CornerRadius = CornerRadius::same(6);

fn draw_contact_footer(ui: &mut egui::Ui, colors: &Colors) {
    ui.vertical_centered(|ui| {
        ui.hyperlink_to(
            RichText::new("❤️  Buy Me a Coffee").size(style::TEXT_BODY),
            "https://buymeacoffee.com/ianjamesbu8",
        );
        ui.add_space(style::SPACE_SM);
        ui.label(
            RichText::new(
                "If you have any ideas, want to help, or just want to say what's up...",
            )
            .size(style::TEXT_CAPTION)
            .color(colors.text_dim),
        );
        ui.add_space(style::SPACE_SM / 2.0);
        {
            let email = "ADHDISNTREAL@GMAIL.COM";
            let mailto = "mailto:ADHDisntreal@gmail.com";
            let font_id = egui::FontId::proportional(style::TEXT_CAPTION);
            let email_w = ui.fonts(|f| {
                f.layout_no_wrap(email.to_string(), font_id, colors.text_dim)
                    .size()
                    .x
            });
            let btn_w = 24.0;
            let gap = ui.spacing().item_spacing.x;
            let pad = ((ui.available_width() - email_w - gap - btn_w) / 2.0).max(0.0);
            ui.horizontal(|ui| {
                ui.add_space(pad);
                ui.hyperlink_to(
                    RichText::new(email)
                        .size(style::TEXT_CAPTION)
                        .color(colors.text_dim),
                    mailto,
                );
                crate::widgets::copy_button(
                    ui,
                    egui::Id::new("shortcuts_email_copy"),
                    "ADHDisntreal@gmail.com",
                );
            });
        }
    });
}

fn section_label(ui: &mut egui::Ui, title: &str, colors: &Colors) {
    ui.label(
        RichText::new(title)
            .size(style::TEXT_HINT)
            .color(colors.text_dim)
            .strong(),
    );
    ui.add_space(2.0);
}

impl PlexiApp {
    pub(crate) fn draw_toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            // Workspace dots
            let sidebar_contexts: Vec<usize> = (0..self.router.len()).collect();
            if sidebar_contexts.len() > 1 {
                let dot_radius = 4.0;
                let dot_spacing = 12.0;
                let total_width = (sidebar_contexts.len() as f32) * dot_spacing;
                let (rect, _) = ui.allocate_exact_size(
                    Vec2::new(total_width, ui.available_height()),
                    egui::Sense::hover(),
                );
                let y = rect.center().y;
                let start_x = rect.left() + dot_radius;
                for (dot_i, ctx_i) in sidebar_contexts.iter().enumerate() {
                    let cx = start_x + (dot_i as f32) * dot_spacing;
                    let color = if *ctx_i == self.router.active_idx() {
                        self.colors.accent
                    } else {
                        self.colors.bg_active
                    };
                    ui.painter()
                        .circle_filled(egui::pos2(cx, y), dot_radius, color);
                }
                ui.add_space(4.0);
            }

            // Right side — help button + notification badge
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .add(
                        egui::Button::new(
                            RichText::new("?").size(12.0).color(self.colors.text_dim),
                        )
                        .frame(false)
                        .min_size(egui::vec2(24.0, 0.0)),
                    )
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text("Keyboard shortcuts (\u{2318}/)")
                    .clicked()
                {
                    self.show_shortcuts = !self.show_shortcuts;
                }

                let version_label = if self.update_available.is_some() {
                    RichText::new(format!("\u{2191} v{}", env!("CARGO_PKG_VERSION")))
                        .size(10.0)
                        .color(self.colors.accent)
                } else {
                    RichText::new(format!("v{}", env!("CARGO_PKG_VERSION")))
                        .size(10.0)
                        .color(self.colors.text_dim)
                };
                let hover_text = if self.update_available.is_some() {
                    "Update available — click to open changelog"
                } else {
                    "Changelog"
                };
                if ui
                    .add(egui::Button::new(version_label).frame(false))
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text(hover_text)
                    .clicked()
                {
                    self.show_changelog = !self.show_changelog;
                }

                let notif_count = self.visible_notification_count();
                if notif_count > 0 {
                    let badge_text = if notif_count > 9 {
                        "9+".to_string()
                    } else {
                        notif_count.to_string()
                    };
                    let btn = egui::Button::new(
                        RichText::new(format!("\u{1F514} {badge_text}"))
                            .size(12.0)
                            .color(self.colors.accent),
                    )
                    .frame(false);
                    if ui
                        .add(btn)
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .on_hover_text("Notifications (\u{2318}\u{21E7}A)")
                        .clicked()
                    {
                        self.show_notification_modal = !self.show_notification_modal;
                        if self.show_notification_modal && self.current_notify_id.is_none() {
                            self.current_notify_id = self.select_highest_priority();
                        }
                    }
                }
            });
        });
    }

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
                                        RichText::new("Move between panes")
                                            .size(style::TEXT_HINT)
                                            .color(colors.text_dim),
                                    );
                                    ui.end_row();

                                    crate::widgets::key_combo_list(
                                        ui,
                                        &[&["\u{2318}", "\u{21E7}"], &["H", "J", "K", "L"]],
                                        None,
                                        colors,
                                    );
                                    ui.label(
                                        RichText::new("Navigate windows")
                                            .size(style::TEXT_HINT)
                                            .color(colors.text_dim),
                                    );
                                    ui.end_row();

                                    crate::widgets::key_combo_list(
                                        ui,
                                        &[&["\u{2318}", "]"], &["\u{2318}", "["]],
                                        None,
                                        colors,
                                    );
                                    ui.label(
                                        RichText::new("Next/prev tab")
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
                                        RichText::new("Zoom pane")
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
                                        (&["\u{2318}", "\u{21E7}", "R"], "Rename pane"),
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
        const CHANGELOG: &str = include_str!("../CHANGELOG.md");

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
                                        RichText::new(format!("\u{2191} v{latest} available"))
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
                    .corner_radius(R6)
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
                        let te = ui.add(
                            egui::TextEdit::singleline(&mut self.rename_buffer)
                                .id(te_id)
                                .desired_width(MODAL_WIDTH)
                                .hint_text("Pane name...")
                                .font(egui::TextStyle::Body),
                        );

                        // Auto-focus and select all
                        if !te.has_focus() {
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
                        }
                    });
            });

        if cancel {
            self.renaming_pane = None;
        } else if commit {
            let new_name = self.rename_buffer.trim().to_string();
            if let Some(pane) =
                self.windows[self.active_window].panes.get_mut(&pane_id)
            {
                if let Some(t) = pane.as_terminal_mut() {
                    t.name = if new_name.is_empty() {
                        None
                    } else {
                        Some(new_name)
                    };
                }
            }
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

        egui::Area::new(egui::Id::new("rename_context_overlay"))
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
                        ui.label(
                            RichText::new("Name this context")
                                .size(13.0)
                                .color(self.colors.text_primary)
                                .strong(),
                        );
                        ui.add_space(6.0);

                        let te_id = egui::Id::new("rename_context_input");
                        let te = ui.add(
                            egui::TextEdit::singleline(&mut self.rename_buffer)
                                .id(te_id)
                                .desired_width(MODAL_WIDTH)
                                .hint_text("Context name...")
                                .font(egui::TextStyle::Body),
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

    pub(crate) fn draw_quit_confirm_overlay(&self, ctx: &egui::Context) {
        let count = self.quit_press_count;
        egui::Area::new(egui::Id::new("quit_confirm_overlay"))
            .anchor(Align2::CENTER_BOTTOM, Vec2::new(0.0, -40.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(self.colors.bg_sidebar)
                    .stroke(Stroke::new(1.0, self.colors.border))
                    .corner_radius(R6)
                    .inner_margin(egui::Margin::symmetric(16, 10))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(format!(
                                    "\u{2318}Q pressed {} of 3 — press again to quit",
                                    count
                                ))
                                .size(12.0)
                                .color(self.colors.text_dim),
                            );
                            ui.add_space(8.0);
                            for i in 1u8..=3 {
                                let color = if i <= count {
                                    self.colors.accent
                                } else {
                                    self.colors.bg_active
                                };
                                let (rect, _) = ui.allocate_exact_size(
                                    Vec2::new(8.0, 8.0),
                                    egui::Sense::hover(),
                                );
                                ui.painter()
                                    .circle_filled(rect.center(), 4.0, color);
                            }
                        });
                    });
            });
    }

    pub(crate) fn draw_confirm_close(&mut self, ctx: &egui::Context) {
        let mut confirmed = false;
        let mut cancelled = false;

        // Consume Enter/Escape at the context level so they cannot bleed
        // through to the focused pane this frame. `ui.input(|i| key_pressed)`
        // is a *read-only* check — it does not remove the event — which is
        // how this modal used to leak a confirming Enter into the pane behind
        // it (e.g. the backlog app opened the selected note on Cmd+W → Enter).
        // Combined with the `FocusLayer::ConfirmClose` capture, this is the
        // systemic fix: overlay owns focus, overlay drains its own keys.
        ctx.input_mut(|i| {
            if i.consume_key(egui::Modifiers::NONE, egui::Key::Enter) {
                confirmed = true;
            }
            if i.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
                cancelled = true;
            }
        });

        // Scrim — visually reinforces modal capture and mirrors the command
        // palette / notification modal chrome.
        let screen_rect = ctx.screen_rect();
        egui::Area::new(egui::Id::new("confirm_close_scrim"))
            .fixed_pos(screen_rect.min)
            .order(egui::Order::Middle)
            .show(ctx, |ui| {
                ui.painter().rect_filled(
                    screen_rect,
                    0.0,
                    egui::Color32::from_black_alpha(120),
                );
                let scrim_response = ui.allocate_rect(screen_rect, egui::Sense::click());
                if scrim_response.clicked() {
                    cancelled = true;
                }
            });

        egui::Area::new(egui::Id::new("confirm_close_overlay"))
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(self.colors.bg_sidebar)
                    .stroke(egui::Stroke::new(1.0, self.colors.border))
                    .corner_radius(R6)
                    .inner_margin(egui::Margin::symmetric(20, 16))
                    .show(ui, |ui| {
                        ui.set_width(MODAL_WIDTH);
                        ui.label(
                            RichText::new("Close pane?")
                                .size(13.0)
                                .color(self.colors.text_primary)
                                .strong(),
                        );
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new("The running process will be terminated.")
                                .size(12.0)
                                .color(self.colors.text_dim),
                        );
                        ui.add_space(12.0);
                        ui.horizontal(|ui| {
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("Close")
                                            .size(12.0)
                                            .color(self.colors.text_primary),
                                    )
                                    .fill(self.colors.bg_active),
                                )
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .clicked()
                            {
                                confirmed = true;
                            }
                            ui.add_space(8.0);
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("Cancel")
                                            .size(12.0)
                                            .color(self.colors.text_dim),
                                    )
                                    .frame(false),
                                )
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .clicked()
                            {
                                cancelled = true;
                            }
                            ui.add_space(12.0);
                            crate::widgets::key_chip(ui, "Enter", &self.colors);
                            ui.label(
                                RichText::new("confirm")
                                    .size(style::TEXT_HINT)
                                    .color(self.colors.text_dim),
                            );
                            ui.add_space(style::SPACE_SM);
                            crate::widgets::key_chip(ui, "Esc", &self.colors);
                            ui.label(
                                RichText::new("cancel")
                                    .size(style::TEXT_HINT)
                                    .color(self.colors.text_dim),
                            );
                        });
                    });
            });

        if confirmed {
            self.pending_close = false;
            if self.execute_close_pane() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            } else {
                self.save_workspace();
            }
        } else if cancelled {
            self.pending_close = false;
        }
    }

    /// Primary notification surface: a keyboard-first centered modal over the
    /// work area. Renders the front of the queue.
    ///
    /// Enter / option-select / input-submit dispatches exactly one
    /// `DeliverNotifyAction` and removes the notification from the queue.
    /// Esc defers: the modal closes but the notification stays in the
    /// queue (reopen with Cmd+Shift+A to come back to it). No
    /// NotifyAction is dispatched on Esc — the app hasn't been answered.
    /// Required notifications reject Esc.
    ///
    /// Input map (all kinds):
    ///   Esc        — defer (only when `required == false`); keeps in queue
    ///
    /// `kind = message`:
    ///   Enter | Space — Acknowledge
    ///
    /// `kind = choice`:
    ///   ↑/↓ or j/k   — move focus
    ///   Enter | Space — confirm focused option
    ///   1-9          — direct-select the Nth option
    ///   per-option `shortcut` — direct-select that option
    ///
    /// `kind = input`:
    ///   text typing  — edits buffer
    ///   Enter        — submit (only if non-empty OR `required == false`)
    pub(crate) fn draw_notification_modal(&mut self, ctx: &egui::Context) -> Vec<AppCommand> {
        use crate::app_protocol::NotifyKind;
        use crate::app::notification_image;

        let mut cmds: Vec<AppCommand> = Vec::new();

        // Resolve the currently-displayed notification by id. If the pinned
        // id was removed under us (dismissed on another code path) or was
        // never set, pick the highest-priority remaining — never arbitrarily
        // index into the Vec.
        if self.pending_notifications.is_empty() {
            // Show an empty-state card so Cmd+Shift+A always gives feedback.
            let screen_rect = ctx.screen_rect();
            egui::Area::new(egui::Id::new("notification_modal_scrim"))
                .order(egui::Order::Foreground)
                .fixed_pos(screen_rect.min)
                .interactable(true)
                .show(ctx, |ui| {
                    let (rect, _) = ui.allocate_exact_size(
                        screen_rect.size(),
                        egui::Sense::click(),
                    );
                    ui.painter().rect_filled(
                        rect,
                        CornerRadius::ZERO,
                        Color32::from_black_alpha(style::SCRIM_ALPHA),
                    );
                });
            egui::Area::new(egui::Id::new("notification_modal"))
                .order(egui::Order::Tooltip)
                .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    egui::Frame::new()
                        .fill(self.colors.bg_sidebar)
                        .stroke(Stroke::new(1.0, self.colors.border))
                        .corner_radius(style::RADIUS_LG)
                        .inner_margin(egui::Margin::symmetric(
                            style::MODAL_PADDING_H,
                            style::MODAL_PADDING_V,
                        ))
                        .show(ui, |ui| {
                            ui.set_width(style::MODAL_WIDTH_NOTIFY);
                            ui.vertical_centered(|ui| {
                                ui.add_space(style::SPACE_MD);
                                ui.label(
                                    RichText::new("No notifications")
                                        .size(style::TEXT_BODY)
                                        .color(self.colors.text_dim),
                                );
                                ui.add_space(style::SPACE_MD);
                            });
                        });
                });
            let esc_pressed = ctx.input(|i| i.key_pressed(egui::Key::Escape));
            if esc_pressed {
                self.show_notification_modal = false;
            }
            return cmds;
        }
        if self
            .current_notify_id
            .as_ref()
            .map(|id| !self.pending_notifications.iter().any(|n| &n.notify_id == id))
            .unwrap_or(true)
        {
            self.current_notify_id = self.select_highest_priority();
        }
        let Some(current_id) = self.current_notify_id.clone() else {
            self.show_notification_modal = false;
            return cmds;
        };
        let Some(notif) = self
            .pending_notifications
            .iter()
            .find(|n| n.notify_id == current_id)
            .cloned()
        else {
            self.show_notification_modal = false;
            self.current_notify_id = None;
            return cmds;
        };

        // Reset per-notification transient state when the front of the queue
        // changes. notify_id is the stable identifier; fall back on title if
        // the app didn't set one (rare — still gives reasonable reset behavior).
        let front_key = if notif.notify_id.is_empty() {
            format!("__no_id__:{}", notif.title)
        } else {
            notif.notify_id.clone()
        };
        if self.modal_state_notify_id != front_key {
            self.modal_state_notify_id = front_key;
            self.modal_focused_option = 0;
            self.modal_input_buffer.clear();
        }

        // Resolve the image attachment (#74) once per frame, before borrowing
        // any other `self` field into the egui closure. `notification_image::resolve`
        // is idempotent — it caches into `self.notification_images` keyed by
        // `notify_id`, so subsequent frames reuse the same TextureHandle.
        let image_state = notification_image::resolve(self, ctx, &notif);

        let screen_rect = ctx.screen_rect();

        // Dim the whole work area. The scrim swallows clicks so UI behind can't
        // accidentally eat them.
        egui::Area::new(egui::Id::new("notification_modal_scrim"))
            .order(egui::Order::Foreground)
            .fixed_pos(screen_rect.min)
            .interactable(true)
            .show(ctx, |ui| {
                let (rect, _) = ui.allocate_exact_size(
                    screen_rect.size(),
                    egui::Sense::click(),
                );
                ui.painter().rect_filled(
                    rect,
                    CornerRadius::ZERO,
                    Color32::from_black_alpha(style::SCRIM_ALPHA),
                );
            });

        let level_color = match notif.level.as_str() {
            "error" => Color32::from_rgb(0xff, 0x55, 0x55),
            "warn" => Color32::from_rgb(0xf1, 0xfa, 0x8c),
            _ => self.colors.accent,
        };

        // Live position + total, recomputed every frame. Total reflects the
        // current queue size so if a new notification arrives while this
        // modal is open, the count updates on the next render without
        // displacing the pinned view.
        let (position_idx, queue_len) = self
            .position_of_current()
            .unwrap_or((1, self.pending_notifications.len()));
        let mut action_cmd: Option<AppCommand> = None;

        // ── Keyboard handling ──────────────────────────────────────────────
        // Done before rendering so focus index is current when we draw the
        // highlighted option. Typing into the input field is handled by the
        // TextEdit widget below (it reads pressed keys from egui directly).
        // Cmd+Enter is consumed (not just read) so the multiline `TextEdit`
        // can't see it and misinterpret it as a newline or some other
        // widget-local action. Bare Enter deliberately flows through to
        // TextEdit for the input kind (it inserts a newline) and to our own
        // read for message/choice kinds (it confirms).
        let cmd_enter_pressed = ctx.input_mut(|i| {
            i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter)
        });

        let (
            enter_pressed,
            space_pressed,
            esc_pressed,
            up_pressed,
            down_pressed,
            digit_pressed,
            shortcut_pressed,
        ) = ctx.input(|i| {
            // Bare Enter (no modifiers) — used for message/choice submit.
            let enter = i.key_pressed(egui::Key::Enter)
                && !i.modifiers.command
                && !i.modifiers.shift
                && !i.modifiers.alt
                && !i.modifiers.ctrl;
            let space = i.key_pressed(egui::Key::Space);
            let esc = i.key_pressed(egui::Key::Escape);
            let up = i.key_pressed(egui::Key::ArrowUp)
                || i.key_pressed(egui::Key::K);
            let down = i.key_pressed(egui::Key::ArrowDown)
                || i.key_pressed(egui::Key::J);
            let mut digit: Option<usize> = None;
            for (n, key) in [
                (1, egui::Key::Num1), (2, egui::Key::Num2), (3, egui::Key::Num3),
                (4, egui::Key::Num4), (5, egui::Key::Num5), (6, egui::Key::Num6),
                (7, egui::Key::Num7), (8, egui::Key::Num8), (9, egui::Key::Num9),
            ] {
                if i.key_pressed(key) {
                    digit = Some(n - 1);
                    break;
                }
            }
            // Collect typed characters for per-option shortcut matching.
            let mut shortcut: Option<char> = None;
            for ev in &i.events {
                if let egui::Event::Key { key, pressed: true, modifiers, .. } = ev {
                    if modifiers.is_none() {
                        if let Some(name) = key.name().chars().next() {
                            let c = name.to_ascii_lowercase();
                            if c.is_ascii_alphabetic() {
                                shortcut = Some(c);
                                break;
                            }
                        }
                    }
                }
            }
            (enter, space, esc, up, down, digit, shortcut)
        });

        // Apply keyboard to modal state BEFORE rendering (so focus ring is fresh).
        match notif.kind {
            NotifyKind::Choice if !notif.options.is_empty() => {
                let n = notif.options.len();
                if up_pressed && self.modal_focused_option > 0 {
                    self.modal_focused_option -= 1;
                }
                if down_pressed && self.modal_focused_option + 1 < n {
                    self.modal_focused_option += 1;
                }
            }
            _ => {}
        }

        egui::Area::new(egui::Id::new("notification_modal"))
            .order(egui::Order::Tooltip)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(self.colors.bg_sidebar)
                    .stroke(Stroke::new(1.0, self.colors.border))
                    .corner_radius(style::RADIUS_LG)
                    .inner_margin(egui::Margin::symmetric(
                        style::MODAL_PADDING_H,
                        style::MODAL_PADDING_V,
                    ))
                    .show(ui, |ui| {
                        ui.set_width(style::MODAL_WIDTH_NOTIFY);

                        // Header: level dot + kind label · queue indicator.
                        ui.horizontal(|ui| {
                            let (dot_rect, _) = ui.allocate_exact_size(
                                Vec2::new(10.0, 10.0),
                                egui::Sense::hover(),
                            );
                            ui.painter()
                                .circle_filled(dot_rect.center(), 5.0, level_color);
                            ui.add_space(style::SPACE_SM);
                            let kind_label = match notif.kind {
                                NotifyKind::Message => "MESSAGE",
                                NotifyKind::Choice => "CHOICE",
                                NotifyKind::Input => "INPUT",
                            };
                            ui.label(
                                RichText::new(format!(
                                    "{}  ·  {}",
                                    notif.level.to_uppercase(),
                                    kind_label
                                ))
                                .size(style::TEXT_HINT)
                                .color(self.colors.text_dim)
                                .strong(),
                            );
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if queue_len > 1 {
                                    // Reversed order because the layout is
                                    // right-to-left: trailing text first,
                                    // then combos in reverse, then the
                                    // position/total label on the far left.
                                    crate::widgets::key_chip(
                                        ui, "]", &self.colors,
                                    );
                                    crate::widgets::key_chip(
                                        ui, "\u{2318}", &self.colors,
                                    );
                                    ui.add_space(8.0);
                                    crate::widgets::key_chip(
                                        ui, "[", &self.colors,
                                    );
                                    crate::widgets::key_chip(
                                        ui, "\u{2318}", &self.colors,
                                    );
                                    ui.add_space(8.0);
                                    ui.label(
                                        RichText::new(format!(
                                            "{position_idx} of {queue_len}  ·  cycle"
                                        ))
                                        .size(style::TEXT_HINT)
                                        .color(self.colors.text_dim),
                                    );
                                }
                            });
                        });

                        ui.add_space(style::SPACE_XL);

                        // Title — centered, large.
                        ui.vertical_centered(|ui| {
                            ui.label(
                                RichText::new(&notif.title)
                                    .size(style::TEXT_TITLE_XL)
                                    .color(self.colors.text_primary)
                                    .strong(),
                            );
                        });

                        // Body — centered under the title.
                        if !notif.body.is_empty() {
                            ui.add_space(style::SPACE_MD);
                            ui.vertical_centered(|ui| {
                                ui.set_max_width(style::MODAL_WIDTH_NOTIFY - 120.0);
                                ui.label(
                                    RichText::new(&notif.body)
                                        .size(style::TEXT_BODY)
                                        .color(self.colors.text_primary),
                                );
                            });
                        }

                        // Image attachment (#74) — renders above the
                        // kind-specific body / action buttons. Sized to fit
                        // the modal width with aspect ratio preserved, max
                        // height 200 px. Placeholder badges render the
                        // user-visible reason text.
                        if let Some(state) = &image_state {
                            ui.add_space(style::SPACE_MD);
                            ui.vertical_centered(|ui| {
                                draw_notification_image(ui, state, &self.colors);
                            });
                        }

                        ui.add_space(style::SPACE_XL);

                        if notif.tombstoned {
                            // Source ended — show a dim label and a plain Dismiss button.
                            // Action buttons are hidden since the app can no longer respond.
                            ui.add_space(style::SPACE_SM);
                            ui.vertical_centered(|ui| {
                                ui.label(
                                    RichText::new("Source ended")
                                        .size(style::TEXT_BODY)
                                        .color(self.colors.text_dim)
                                        .italics(),
                                );
                            });
                            ui.add_space(style::SPACE_SM);
                            ui.add_space(style::SPACE_MD);
                            ui.vertical_centered(|ui| {
                                let resp = primary_button(ui, "Dismiss", &self.colors, 180.0);
                                if resp.clicked() {
                                    if let Some(n) = self.pending_notifications
                                        .iter()
                                        .find(|n| n.notify_id == current_id)
                                        .cloned()
                                    {
                                        self.pending_notifications
                                            .retain(|x| x.notify_id != current_id);
                                        self.current_notify_id = None;
                                        if !n.notify_id.is_empty()
                                            && !n.notify_id.starts_with("__host__:")
                                        {
                                            action_cmd =
                                                Some(AppCommand::DeliverNotifyAction {
                                                    pane_id: n.sender_pane_id,
                                                    notify_id: n.notify_id.clone(),
                                                    action_label: "tombstone_dismiss"
                                                        .to_string(),
                                                    value: Some(
                                                        "tombstone_dismiss".to_string(),
                                                    ),
                                                    response_file: n.response_file.clone(),
                                                    host_action: None,
                                                });
                                        }
                                    }
                                }
                            });
                            ui.add_space(style::SPACE_MD);
                        } else {
                        // Kind-specific body.
                        match notif.kind {
                            NotifyKind::Message => {}
                            NotifyKind::Choice => {
                                for (idx, opt) in notif.options.iter().enumerate() {
                                    let focused = idx == self.modal_focused_option;
                                    let shortcut_hint = opt
                                        .shortcut
                                        .as_ref()
                                        .map(|s| format!("[{}]", s.to_uppercase()))
                                        .unwrap_or_else(|| format!("[{}]", idx + 1));
                                    let resp = option_button(
                                        ui,
                                        &opt.label,
                                        &shortcut_hint,
                                        focused,
                                        &self.colors,
                                    );
                                    if resp.clicked() {
                                        let value = if opt.value.is_empty() {
                                            opt.label.clone()
                                        } else {
                                            opt.value.clone()
                                        };
                                        action_cmd = Some(AppCommand::DeliverNotifyAction {
                                            pane_id: notif.sender_pane_id,
                                            notify_id: notif.notify_id.clone(),
                                            action_label: opt.label.clone(),
                                            value: Some(value),
                                            response_file: notif.response_file.clone(),
                                            host_action: opt.host_action.clone(),
                                        });
                                    }
                                    ui.add_space(style::SPACE_SM);
                                }
                            }
                            NotifyKind::Input => {
                                if let Some(prompt) = &notif.input_prompt {
                                    ui.vertical_centered(|ui| {
                                        ui.label(
                                            RichText::new(prompt)
                                                .size(style::TEXT_CAPTION)
                                                .color(self.colors.text_dim),
                                        );
                                    });
                                    ui.add_space(style::SPACE_SM);
                                }
                                // Multiline editor: Enter inserts a newline,
                                // Cmd+Enter submits (handled in the keyboard
                                // pre-pass below). Scrolls vertically once it
                                // exceeds the visible row count.
                                let te = egui::TextEdit::multiline(
                                    &mut self.modal_input_buffer,
                                )
                                .desired_width(f32::INFINITY)
                                .desired_rows(6)
                                .font(egui::FontId::proportional(16.0))
                                .margin(egui::Margin::symmetric(12, 10))
                                .hint_text("Type. Enter for newline, \u{2318}\u{21B5} to submit.");
                                let resp = ui.add(te);
                                resp.request_focus();
                            }
                        }

                        ui.add_space(style::SPACE_MD);

                        // Footer: primary action for message kind, plus a
                        // centered hint row describing keyboard shortcuts.
                        if matches!(notif.kind, NotifyKind::Message) {
                            ui.vertical_centered(|ui| {
                                let resp = primary_button(
                                    ui,
                                    "Acknowledge",
                                    &self.colors,
                                    180.0,
                                );
                                if resp.clicked() {
                                    action_cmd = Some(AppCommand::DeliverNotifyAction {
                                        pane_id: notif.sender_pane_id,
                                        notify_id: notif.notify_id.clone(),
                                        action_label: "acknowledge".to_string(),
                                        value: None,
                                        response_file: notif.response_file.clone(),
                                        host_action: None,
                                    });
                                }
                            });
                            ui.add_space(style::SPACE_MD);
                        }
                        } // end !tombstoned

                        // Footer keyboard hints — separator + centered hint row per kind.
                        // Not shown for tombstoned notifications (only a Dismiss button).
                        //
                        // Centering strategy: egui's vertical_centered expands all child
                        // rects to full available width (Layout::top_down(Center) sets
                        // frame_size.x = max(desired, available)), so placing a ui.horizontal
                        // inside vertical_centered does NOT center it — content paints at x=0.
                        // Fix: pre-measure the row width with ui.fonts, then use
                        // allocate_ui_with_layout(exact_size) — justify_and_align then places
                        // the child rect at center_x − hint_w/2, which IS centered.
                        if !notif.tombstoned {
                            ui.add_space(style::SPACE_SM);
                            ui.separator();
                            ui.add_space(style::SPACE_SM);

                            // Measure hint row width to match what key_combo_list renders.
                            // key_combo_list: item_spacing=0, INTER_COMBO_GAP=10 between
                            // combos, TRAILING_GAP=10 before label. INTRA_COMBO_GAP=2
                            // between chips within a combo (e.g. ⌘+↵).
                            let mono12 = egui::FontId::monospace(style::TEXT_CAPTION);
                            let prop11 = egui::FontId::proportional(style::TEXT_HINT);
                            let (hint_w, hint_h) = ui.fonts(|f| {
                                let chip_w = |s: &str| -> f32 {
                                    let g = f.layout_no_wrap(
                                        s.to_string(), mono12.clone(), egui::Color32::WHITE,
                                    );
                                    let h = g.size().y + 6.0; // KEYCAP_PAD_V*2
                                    (g.size().x + 12.0_f32).max(h) // KEYCAP_PAD_H*2
                                };
                                let lbl_w = |s: &str| -> f32 {
                                    f.layout_no_wrap(
                                        s.to_string(), prop11.clone(), egui::Color32::WHITE,
                                    ).size().x
                                };
                                let kcl = |combos: &[&[&str]], trailing: Option<&str>| -> f32 {
                                    let mut w = 0.0_f32;
                                    for (i, keys) in combos.iter().enumerate() {
                                        if i > 0 { w += 10.0; } // INTER_COMBO_GAP
                                        for (j, key) in keys.iter().enumerate() {
                                            if j > 0 { w += 2.0; } // INTRA_COMBO_GAP
                                            w += chip_w(key);
                                        }
                                    }
                                    if let Some(t) = trailing { w += 10.0 + lbl_w(t); }
                                    w
                                };
                                // Between groups: add_space(8) + "·" label + add_space(8)
                                let dot = || 8.0 + lbl_w("·") + 8.0;
                                let w = match notif.kind {
                                    NotifyKind::Message => {
                                        kcl(&[&["Enter"], &["Space"]], Some("acknowledge"))
                                        + if !notif.required {
                                            dot() + kcl(&[&["Esc"]], Some("dismiss"))
                                        } else { 0.0 }
                                    }
                                    NotifyKind::Choice => {
                                        kcl(&[&["↑↓"], &["j/k"]], Some("navigate"))
                                        + dot() + kcl(&[&["Enter"], &["1-9"]], Some("select"))
                                        + if !notif.required {
                                            dot() + kcl(&[&["Esc"]], Some("dismiss"))
                                        } else { 0.0 }
                                    }
                                    NotifyKind::Input => {
                                        kcl(&[&["Enter"]], Some("newline"))
                                        + dot() + kcl(&[&["\u{2318}", "\u{21B5}"]], Some("submit"))
                                        + if !notif.required {
                                            dot() + kcl(&[&["Esc"]], Some("dismiss"))
                                        } else { 0.0 }
                                    }
                                };
                                let h = {
                                    let g = f.layout_no_wrap(
                                        "A".to_string(), mono12.clone(), egui::Color32::WHITE,
                                    );
                                    g.size().y + 6.0 // chip_h
                                };
                                (w, h)
                            });

                            // Allocate the exact width centered in the parent top_down(Center).
                            // justify_and_align places child_rect at center_x − hint_w/2.
                            ui.vertical_centered(|ui| {
                                ui.allocate_ui_with_layout(
                                    egui::Vec2::new(hint_w, hint_h),
                                    egui::Layout::left_to_right(egui::Align::Center),
                                    |ui| {
                                        ui.spacing_mut().item_spacing.x = 0.0;
                                        let dim = |s: &str| {
                                            RichText::new(s)
                                                .size(style::TEXT_HINT)
                                                .color(self.colors.text_dim)
                                        };
                                        match notif.kind {
                                            NotifyKind::Message => {
                                                crate::widgets::key_combo_list(
                                                    ui,
                                                    &[&["Enter"], &["Space"]],
                                                    Some("acknowledge"),
                                                    &self.colors,
                                                );
                                                if !notif.required {
                                                    ui.add_space(style::SPACE_SM);
                                                    ui.label(dim("·"));
                                                    ui.add_space(style::SPACE_SM);
                                                    crate::widgets::key_combo_list(
                                                        ui,
                                                        &[&["Esc"]],
                                                        Some("dismiss"),
                                                        &self.colors,
                                                    );
                                                }
                                            }
                                            NotifyKind::Choice => {
                                                crate::widgets::key_combo_list(
                                                    ui,
                                                    &[&["↑↓"], &["j/k"]],
                                                    Some("navigate"),
                                                    &self.colors,
                                                );
                                                ui.add_space(style::SPACE_SM);
                                                ui.label(dim("·"));
                                                ui.add_space(style::SPACE_SM);
                                                crate::widgets::key_combo_list(
                                                    ui,
                                                    &[&["Enter"], &["1-9"]],
                                                    Some("select"),
                                                    &self.colors,
                                                );
                                                if !notif.required {
                                                    ui.add_space(style::SPACE_SM);
                                                    ui.label(dim("·"));
                                                    ui.add_space(style::SPACE_SM);
                                                    crate::widgets::key_combo_list(
                                                        ui,
                                                        &[&["Esc"]],
                                                        Some("dismiss"),
                                                        &self.colors,
                                                    );
                                                }
                                            }
                                            NotifyKind::Input => {
                                                crate::widgets::key_combo_list(
                                                    ui,
                                                    &[&["Enter"]],
                                                    Some("newline"),
                                                    &self.colors,
                                                );
                                                ui.add_space(style::SPACE_SM);
                                                ui.label(dim("·"));
                                                ui.add_space(style::SPACE_SM);
                                                crate::widgets::key_combo_list(
                                                    ui,
                                                    &[&["\u{2318}", "\u{21B5}"]],
                                                    Some("submit"),
                                                    &self.colors,
                                                );
                                                if !notif.required {
                                                    ui.add_space(style::SPACE_SM);
                                                    ui.label(dim("·"));
                                                    ui.add_space(style::SPACE_SM);
                                                    crate::widgets::key_combo_list(
                                                        ui,
                                                        &[&["Esc"]],
                                                        Some("dismiss"),
                                                        &self.colors,
                                                    );
                                                }
                                            }
                                        }
                                    },
                                );
                            });
                        } // end !tombstoned keyboard hint
                    });
            });

        // ── Resolve keyboard input into an action_cmd (only if not already
        //    produced by a mouse click). Mouse wins; keyboard is a fallback.
        // Tombstoned notifications have no keyboard actions — only the mouse Dismiss.
        if action_cmd.is_none() && !notif.tombstoned {
            match notif.kind {
                NotifyKind::Message => {
                    if enter_pressed || space_pressed {
                        action_cmd = Some(AppCommand::DeliverNotifyAction {
                            pane_id: notif.sender_pane_id,
                            notify_id: notif.notify_id.clone(),
                            action_label: "acknowledge".to_string(),
                            value: None,
                            response_file: notif.response_file.clone(),
                            host_action: None,
                        });
                    }
                }
                NotifyKind::Choice if !notif.options.is_empty() => {
                    // Direct-select by digit, then per-option shortcut, then Enter.
                    let mut picked: Option<usize> = None;
                    if let Some(d) = digit_pressed {
                        if d < notif.options.len() {
                            picked = Some(d);
                        }
                    }
                    if picked.is_none() {
                        if let Some(c) = shortcut_pressed {
                            for (i, opt) in notif.options.iter().enumerate() {
                                if let Some(sc) = &opt.shortcut {
                                    if sc.to_ascii_lowercase().chars().next() == Some(c) {
                                        picked = Some(i);
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    if picked.is_none() && (enter_pressed || space_pressed) {
                        picked = Some(self.modal_focused_option);
                    }
                    if let Some(idx) = picked {
                        let opt = &notif.options[idx];
                        let value = if opt.value.is_empty() {
                            opt.label.clone()
                        } else {
                            opt.value.clone()
                        };
                        action_cmd = Some(AppCommand::DeliverNotifyAction {
                            pane_id: notif.sender_pane_id,
                            notify_id: notif.notify_id.clone(),
                            action_label: opt.label.clone(),
                            value: Some(value),
                            response_file: notif.response_file.clone(),
                            host_action: opt.host_action.clone(),
                        });
                    }
                }
                NotifyKind::Input => {
                    // Bare Enter inserts a newline into the multiline field;
                    // Cmd+Enter is the commit chord.
                    if cmd_enter_pressed {
                        let buf = self.modal_input_buffer.trim().to_string();
                        if !buf.is_empty() || !notif.required {
                            action_cmd = Some(AppCommand::DeliverNotifyAction {
                                pane_id: notif.sender_pane_id,
                                notify_id: notif.notify_id.clone(),
                                action_label: "submit".to_string(),
                                value: Some(buf),
                                response_file: notif.response_file.clone(),
                                host_action: None,
                            });
                        }
                    }
                }
                _ => {}
            }
        }

        // Esc defers (not cancels) unless the notification is required.
        //
        // Defer = close the modal but keep the notification in the queue.
        // Cmd+Shift+A will bring it back as the front-most on reopen.
        // No NotifyAction is dispatched to the app — the app hasn't been
        // answered yet, it just got postponed.
        //
        // For required notifications, Esc does nothing: the user must
        // acknowledge or pick an option.
        if action_cmd.is_none() && esc_pressed && !notif.required {
            self.show_notification_modal = false;
            self.modal_focused_option = 0;
            self.modal_input_buffer.clear();
            self.modal_state_notify_id.clear();
            // `current_notify_id` intentionally stays set so the same
            // notification is front-most on next reopen. No queue mutation,
            // no NotifyAction — that's what makes it defer, not cancel.
            return cmds;
        }

        if let Some(cmd) = action_cmd {
            use crate::app_trait::AppCommand;
            // Check for snooze before removing from queue. Snooze sets
            // deliver_after in place — the notification stays but becomes
            // invisible and no DeliverNotifyAction is sent to the app (the
            // CLI stays blocked until the user picks a non-snooze choice).
            let snooze_secs = if let AppCommand::DeliverNotifyAction { ref host_action, .. } = cmd {
                host_action.as_deref()
                    .and_then(|a| a.strip_prefix("snooze:"))
                    .and_then(|s| s.parse::<u64>().ok())
            } else {
                None
            };

            if let Some(delay_secs) = snooze_secs {
                let wake = std::time::Instant::now() + std::time::Duration::from_secs(delay_secs);
                if let Some(n) = self.pending_notifications.iter_mut()
                    .find(|n| n.notify_id == current_id)
                {
                    n.deliver_after = Some(wake);
                    log::info!("notify:snooze: notify_id={} delay={}s", current_id, delay_secs);
                }
                self.modal_focused_option = 0;
                self.modal_input_buffer.clear();
                self.modal_state_notify_id.clear();
                let next = self.select_highest_priority();
                if next.is_some() {
                    self.current_notify_id = next;
                } else {
                    self.show_notification_modal = false;
                    self.current_notify_id = None;
                }
            } else {
                // Acknowledgment / option-select / input-submit — real answer.
                // Remove the notification, deliver the NotifyAction, pick next.
                self.pending_notifications
                    .retain(|n| n.notify_id != current_id);
                self.modal_focused_option = 0;
                self.modal_input_buffer.clear();
                self.modal_state_notify_id.clear();
                if self.pending_notifications.is_empty() {
                    self.show_notification_modal = false;
                    self.current_notify_id = None;
                } else {
                    self.current_notify_id = self.select_highest_priority();
                }
                cmds.push(cmd);
            }
        }

        cmds
    }
}

/// Full-width option button for the notification modal's `choice` kind.
///
/// Egui's built-in `Button` left-aligns its label and gives you no hook to
/// center it inside a wider fixed-width rect. We paint the rect, label, and
/// shortcut-hint manually so:
///   • The label sits center-horizontal and center-vertical.
///   • The shortcut hint (e.g. `[Y]`) sits right-aligned in the gutter.
///   • The focused option gets an accent fill + darker text for contrast.
fn option_button(
    ui: &mut egui::Ui,
    label: &str,
    shortcut_hint: &str,
    focused: bool,
    colors: &Colors,
) -> egui::Response {
    let width = ui.available_width();
    let height = style::BUTTON_H_LG;
    let (rect, resp) =
        ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::click());

    let (bg, fg, hint_color) = if focused {
        (
            colors.accent,
            Color32::BLACK,
            Color32::from_black_alpha(140),
        )
    } else {
        (colors.bg_hover, colors.text_primary, colors.text_dim)
    };

    // Hover lifts the bg slightly for non-focused options (focused is already
    // the accent and looks clearly active — no hover lift needed).
    let actual_bg = if resp.hovered() && !focused {
        colors.bg_active
    } else {
        bg
    };

    let painter = ui.painter();
    painter.rect_filled(rect, style::RADIUS_MD, actual_bg);

    // Centered label.
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(style::TEXT_BODY + 1.0),
        fg,
    );

    // Right-gutter shortcut hint.
    if !shortcut_hint.is_empty() {
        let hint_pos = egui::pos2(
            rect.right() - 18.0,
            rect.center().y,
        );
        painter.text(
            hint_pos,
            Align2::RIGHT_CENTER,
            shortcut_hint,
            egui::FontId::proportional(style::TEXT_CAPTION),
            hint_color,
        );
    }

    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    resp
}

/// Primary action button — used for the message-kind Acknowledge. Fixed width,
/// center-aligned label, accent fill. Keyboard dispatch is handled separately.
fn primary_button(
    ui: &mut egui::Ui,
    label: &str,
    colors: &Colors,
    width: f32,
) -> egui::Response {
    let height = style::BUTTON_H_MD;
    let (rect, resp) =
        ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::click());

    let bg = if resp.hovered() {
        colors.accent.gamma_multiply(1.15)
    } else {
        colors.accent
    };
    let painter = ui.painter();
    painter.rect_filled(rect, style::RADIUS_MD, bg);
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(style::TEXT_BODY + 1.0),
        Color32::BLACK,
    );
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

/// Maximum displayed image height inside the notification card. Width is
/// constrained by the card's available width; aspect ratio is preserved.
const NOTIFICATION_IMAGE_MAX_H: f32 = 200.0;

/// Render an attached notification image — either the loaded texture (with
/// aspect-preserving scaling) or a placeholder badge with the failure
/// reason. Called from the modal renderer above the kind-specific body.
fn draw_notification_image(
    ui: &mut egui::Ui,
    state: &crate::app::NotificationImageState,
    colors: &Colors,
) {
    use crate::app::NotificationImageState as S;
    match state {
        S::Ready(texture, w, h) => {
            // Fit within the card's available width and the global
            // 200 px height cap, preserving aspect ratio.
            let avail_w = ui.available_width();
            let (orig_w, orig_h) = (*w as f32, *h as f32);
            let scale = (avail_w / orig_w).min(NOTIFICATION_IMAGE_MAX_H / orig_h);
            // Only ever shrink — never upscale a small image and pixelate it.
            let scale = scale.min(1.0).max(0.0);
            let display = Vec2::new(orig_w * scale, orig_h * scale);
            ui.add(egui::Image::new((texture.id(), display)).corner_radius(style::RADIUS_MD));
        }
        S::Placeholder { reason } => {
            // Draw a small badge so the user sees that an image was attached
            // but couldn't be rendered. Keeps the modal layout stable.
            let badge_w = 220.0;
            let badge_h = 36.0;
            let (rect, _) = ui.allocate_exact_size(
                Vec2::new(badge_w, badge_h),
                egui::Sense::hover(),
            );
            ui.painter()
                .rect_filled(rect, style::RADIUS_MD, colors.bg_hover);
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                format!("[image: {reason}]"),
                egui::FontId::proportional(style::TEXT_CAPTION),
                colors.text_dim,
            );
        }
        S::Pending => {
            // Show a placeholder so the layout doesn't jump on the next frame
            // when the texture arrives. "loading" is the user-visible label.
            let badge_w = 160.0;
            let badge_h = 36.0;
            let (rect, _) = ui.allocate_exact_size(
                Vec2::new(badge_w, badge_h),
                egui::Sense::hover(),
            );
            ui.painter()
                .rect_filled(rect, style::RADIUS_MD, colors.bg_hover);
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                "[image: loading…]",
                egui::FontId::proportional(style::TEXT_CAPTION),
                colors.text_dim,
            );
        }
    }
}

impl PlexiApp {
    /// Render the spatial-grid minimap in the top-right corner of the work
    /// area. Uses an `egui::Area` so it floats above all content.
    pub(crate) fn draw_minimap_overlay(&mut self, ctx: &egui::Context) {
        if !self.minimap.visible {
            return;
        }

        let content_rect = ctx.screen_rect();
        let active_context = self.active_window;
        let colors = self.colors;
        let ws_id = self.router.active().context_id;
        let ws_name = self.router.active().name.clone();

        // Anchor the Area at the actual panel position so its bounding rect
        // doesn't overlap the sidebar. Previously anchored at content_rect.min
        // (0,0), which made layer_id_at return Foreground for the top sidebar
        // rows, suppressing their hover state entirely (#852).
        let Some(panel_rect) = crate::minimap::minimap_panel_rect(
            ctx,
            content_rect,
            &self.windows,
            ws_id,
            &ws_name,
        ) else {
            return;
        };
        log::debug!("minimap area anchored at {:?}", panel_rect.min);

        egui::Area::new(egui::Id::new("minimap_overlay"))
            .order(egui::Order::Foreground)
            .fixed_pos(panel_rect.min)
            .interactable(true)
            .show(ctx, |ui| {
                if let Some(clicked_idx) = crate::minimap::render_minimap(
                    ui,
                    content_rect,
                    &self.windows,
                    active_context,
                    &self.last_page_x_per_row,
                    &colors,
                    ws_id,
                    &ws_name,
                ) {
                    let old = &self.windows[self.active_window];
                    self.last_page_x_per_row.insert(old.grid_y, old.grid_x);
                    self.active_window = clicked_idx;
                    let new = &self.windows[clicked_idx];
                    self.last_page_x_per_row.insert(new.grid_y, new.grid_x);
                    self.context_active_window.insert(ws_id, new.window_id);
                }
            });
    }

    pub(crate) fn draw_welcome_screen(&self, ui: &mut egui::Ui) {
        let colors = self.colors;
        let center = ui.max_rect().center();
        let box_rect = egui::Rect::from_center_size(center, egui::vec2(480.0, 560.0));

        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(box_rect), |ui| {
            egui::Frame::new()
                .fill(colors.bg_sidebar)
                .stroke(Stroke::new(1.0, colors.border))
                .corner_radius(style::RADIUS_MD)
                .inner_margin(egui::Margin::symmetric(
                    style::MODAL_PADDING_H,
                    style::MODAL_PADDING_V,
                ))
                .show(ui, |ui| {
                    ui.add_space(style::SPACE_XL);
                    {
                        let logo_size = style::TEXT_TITLE_XL + 4.0; // 32px — matches SVG native size
                        let gap = style::SPACE_SM;
                        let font_id = egui::FontId::proportional(style::TEXT_TITLE_XL);
                        let text_w = ui.fonts(|f| {
                            f.layout_no_wrap(
                                "PLEXI".to_string(),
                                font_id,
                                colors.text_primary,
                            )
                            .size()
                            .x
                        });
                        let total_w = logo_size + gap + text_w;
                        let pad = ((ui.available_width() - total_w) / 2.0).max(0.0);

                        ui.horizontal(|ui| {
                            ui.add_space(pad);
                            let (logo_rect, _) = ui.allocate_exact_size(
                                egui::vec2(logo_size, logo_size),
                                egui::Sense::hover(),
                            );
                            let painter = ui.painter().clone();
                            let scale = logo_size / 32.0;
                            let cell = egui::vec2(10.0 * scale, 10.0 * scale);
                            let s1 = 5.0 * scale;
                            let s2 = 17.0 * scale;
                            let rx =
                                egui::CornerRadius::same((1.5 * scale).round() as u8);
                            let outline = Stroke::new(
                                0.9 * scale,
                                egui::Color32::from_rgb(0xe4, 0xe4, 0xe7),
                            );
                            let purple = egui::Color32::from_rgb(0x3b, 0x07, 0x64);
                            for (dx, dy, filled) in [
                                (s1, s1, false),
                                (s2, s1, false),
                                (s1, s2, false),
                                (s2, s2, true),
                            ] {
                                let r = egui::Rect::from_min_size(
                                    logo_rect.min + egui::vec2(dx, dy),
                                    cell,
                                );
                                if filled {
                                    painter.rect_filled(r, rx, purple);
                                } else {
                                    painter.rect_stroke(r, rx, outline, egui::StrokeKind::Inside);
                                }
                            }
                            ui.add_space(gap);
                            ui.label(
                                RichText::new("PLEXI")
                                    .size(style::TEXT_TITLE_XL)
                                    .color(colors.text_primary)
                                    .strong(),
                            );
                        });
                    }
                    ui.add_space(style::SPACE_XL);

                    // Each entry: (chip groups for one combo, description).
                    // Every modifier/key is a separate chip — no combined strings.
                    let shortcuts: &[(&[&str], &str)] = &[
                        (&["⌘", "N"], "new terminal"),
                        (&["⌘", "⇧", "N"], "new context"),
                        (&["⌘", "H", "J", "K", "L"], "move between panes"),
                        (&["⌘", "⇧", "H", "J", "K", "L"], "move between windows"),
                        (&["⌘", "/"], "keyboard shortcuts"),
                    ];

                    for (keys, desc) in shortcuts {
                        ui.horizontal(|ui| {
                            crate::widgets::key_combo(ui, keys, &colors);
                            ui.add_space(style::SPACE_SM);
                            ui.label(
                                RichText::new(*desc)
                                    .size(style::TEXT_BODY)
                                    .color(colors.text_dim),
                            );
                        });
                        ui.add_space(style::SPACE_SM);
                    }

                    ui.add_space(style::SPACE_XL);
                    ui.label(
                        RichText::new("Things to try")
                            .size(style::TEXT_CAPTION)
                            .color(colors.text_dim)
                            .strong(),
                    );
                    ui.add_space(style::SPACE_SM);

                    let tips: &[&str] = &[
                        "Right-click a context → Move to Top to keep your most important projects first",
                        "⌘P jumps to any pane across all contexts instantly",
                        "⌘⇧N opens a fresh context — use it like a virtual desktop",
                    ];
                    for tip in tips {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("·")
                                    .size(style::TEXT_BODY)
                                    .color(colors.text_dim),
                            );
                            ui.add_space(style::SPACE_SM / 2.0);
                            ui.label(
                                RichText::new(*tip)
                                    .size(style::TEXT_CAPTION)
                                    .color(colors.text_dim),
                            );
                        });
                        ui.add_space(style::SPACE_SM / 2.0);
                    }

                    ui.add_space(style::SPACE_XL);
                    ui.separator();
                    ui.add_space(style::SPACE_MD);
                    draw_contact_footer(ui, &colors);
                });
        });
    }

    pub(crate) fn draw_cli_setup_modal(&mut self, ctx: &egui::Context) {
        let cli_name = crate::cli_setup::cli_name();
        let colors = self.colors;

        egui::Area::new(egui::Id::new("cli_setup_modal"))
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(colors.bg_sidebar)
                    .stroke(Stroke::new(1.0, colors.border))
                    .corner_radius(R6)
                    .inner_margin(egui::Margin::symmetric(24, 20))
                    .show(ui, |ui| {
                        ui.set_width(400.0);

                        ui.vertical_centered(|ui| {
                            ui.label(
                                RichText::new(format!("Install `{cli_name}`?"))
                                    .size(style::TEXT_BODY)
                                    .color(colors.text_primary)
                                    .strong(),
                            );
                            ui.add_space(style::SPACE_SM);
                            ui.label(
                                RichText::new("Lets shell scripts, agents, and hooks:")
                                    .size(style::TEXT_CAPTION)
                                    .color(colors.text_dim),
                            );
                            ui.add_space(2.0);
                            for item in &["send notifications", "trigger actions", "pipe data into Plexi"] {
                                ui.horizontal(|ui| {
                                    ui.add_space(style::SPACE_SM);
                                    ui.label(
                                        RichText::new(format!("· {item}"))
                                            .size(style::TEXT_CAPTION)
                                            .color(colors.text_dim),
                                    );
                                });
                            }
                            ui.add_space(style::SPACE_SM);
                            ui.label(
                                RichText::new(format!(
                                    "Symlinks /usr/local/bin/{cli_name} → this binary."
                                ))
                                .size(style::TEXT_CAPTION)
                                .color(colors.text_dim),
                            );
                            ui.add_space(style::SPACE_MD);

                            if let Some(err) = &self.cli_setup_error.clone() {
                                ui.label(
                                    RichText::new(format!("Install failed: {err}"))
                                        .size(style::TEXT_CAPTION)
                                        .color(egui::Color32::from_rgb(220, 80, 80)),
                                );
                                ui.add_space(style::SPACE_SM);
                                ui.label(
                                    RichText::new("Run manually:")
                                        .size(style::TEXT_CAPTION)
                                        .color(colors.text_dim),
                                );
                                ui.add_space(2.0);
                                ui.label(
                                    RichText::new(format!(
                                        "sudo ln -sf /Applications/Plexi.app/Contents/MacOS/plexi /usr/local/bin/{cli_name}"
                                    ))
                                    .size(style::TEXT_CAPTION)
                                    .color(colors.text_primary)
                                    .monospace(),
                                );
                                ui.add_space(style::SPACE_SM);
                            }

                            ui.horizontal(|ui| {
                                // Centre the two buttons manually.
                                let btn_w = 100.0;
                                let gap = style::SPACE_SM;
                                let pad = ((ui.available_width() - btn_w * 2.0 - gap) / 2.0).max(0.0);
                                ui.add_space(pad);

                                let install_btn = ui.add(
                                    egui::Button::new(
                                        RichText::new("Install")
                                            .size(style::TEXT_BODY)
                                            .color(colors.text_primary),
                                    )
                                    .min_size(egui::vec2(btn_w, 28.0)),
                                );
                                ui.add_space(gap);
                                let skip_btn = ui.add(
                                    egui::Button::new(
                                        RichText::new("Not now")
                                            .size(style::TEXT_BODY)
                                            .color(colors.text_dim),
                                    )
                                    .min_size(egui::vec2(btn_w, 28.0)),
                                );

                                if install_btn.clicked() {
                                    match crate::cli_setup::install_symlink() {
                                        Ok(msg) => {
                                            log::info!("cli_setup: {msg}");
                                            crate::cli_setup::mark_prompted();
                                            self.show_cli_setup_prompt = false;
                                            self.cli_setup_error = None;
                                        }
                                        Err(e) => {
                                            log::warn!("cli_setup: install failed: {e}");
                                            self.cli_setup_error = Some(e);
                                        }
                                    }
                                }
                                if skip_btn.clicked() {
                                    log::info!("cli_setup: user chose Not now — will ask again next launch");
                                    self.show_cli_setup_prompt = false;
                                    self.cli_setup_error = None;
                                }
                            });
                        });
                    });
            });

        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            log::info!("cli_setup: dismissed via Escape — will ask again next launch");
            self.show_cli_setup_prompt = false;
        }
    }
}

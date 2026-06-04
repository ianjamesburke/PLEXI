use super::*;

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
        let Some(panel_rect) = crate::render::minimap::minimap_panel_rect(
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
                if let Some(clicked_idx) = crate::render::minimap::render_minimap(
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
                    ui.add_space(style::SPACE_MD);
                    ui.label(
                        RichText::new(
                            "Caution: early-stage passion project. If you encounter \
                             any issues, don't hesitate to reach out.",
                        )
                        .size(style::TEXT_HINT)
                        .color(colors.text_dim)
                        .italics(),
                    );
                    ui.add_space(style::SPACE_XL);

                    // Each entry: (chip groups for one combo, description).
                    // Every modifier/key is a separate chip — no combined strings.
                    let shortcuts: &[(&[&str], &str)] = &[
                        (&["⌘", "N"], "new terminal"),
                        (&["⌘", "E"], "file browser"),
                        (&["⌘", "P"], "command palette"),
                        (&["⌘", "⇧", "N"], "new context"),
                        (&["⌘", "/"], "keyboard shortcuts"),
                    ];

                    for (keys, desc) in shortcuts {
                        ui.horizontal(|ui| {
                            crate::ui::widgets::key_combo(ui, keys, &colors);
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
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("New to Plexi? Run ")
                                .size(style::TEXT_CAPTION)
                                .color(colors.text_dim),
                        );
                        ui.label(
                            RichText::new("plexi demo")
                                .size(style::TEXT_CAPTION)
                                .color(colors.text_dim)
                                .monospace()
                                .background_color(colors.bg_active),
                        );
                        ui.label(
                            RichText::new(" in any terminal to get started.")
                                .size(style::TEXT_CAPTION)
                                .color(colors.text_dim),
                        );
                    });
                    ui.add_space(style::SPACE_SM);
                    ui.hyperlink_to(
                        RichText::new("Read the docs at plexiapp.com/docs")
                            .size(style::TEXT_CAPTION)
                            .color(colors.text_dim),
                        "https://plexiapp.com/docs",
                    );

                    ui.add_space(style::SPACE_MD);
                    ui.separator();
                    ui.add_space(style::SPACE_MD);
                    draw_contact_footer(ui, &colors);
                });
        });
    }

    pub(crate) fn draw_cli_setup_modal(&mut self, ctx: &egui::Context) {
        let cli_name = crate::cli::setup::cli_name();
        let colors = self.colors;
        let cmd = crate::cli::setup::INSTALL_COMMAND;

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
                                RichText::new(format!("Install `{cli_name}`"))
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
                            ui.add_space(style::SPACE_MD);
                            ui.label(
                                RichText::new(
                                    "Open Terminal and run this command.\n\
                                     You'll be asked for your password."
                                )
                                .size(style::TEXT_CAPTION)
                                .color(colors.text_dim),
                            );
                            ui.add_space(style::SPACE_SM);
                            ui.label(
                                RichText::new("Run this in the terminal:")
                                    .size(style::TEXT_CAPTION)
                                    .color(colors.text_dim)
                                    .strong(),
                            );
                            ui.add_space(2.0);

                            // Copyable install command with code-block styling.
                            egui::Frame::new()
                                .fill(colors.bg_darkest)
                                .corner_radius(R6)
                                .inner_margin(egui::Margin::symmetric(12, 8))
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            RichText::new(cmd)
                                                .size(style::TEXT_CAPTION)
                                                .color(colors.text_primary)
                                                .monospace(),
                                        );
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            crate::ui::widgets::copy_button(
                                                ui,
                                                egui::Id::new("cli_setup_copy"),
                                                cmd,
                                            );
                                        });
                                    });
                                });

                            ui.add_space(style::SPACE_MD);

                            // Show result feedback from a previous check.
                            if let Some(false) = self.cli_setup_check_result {
                                ui.label(
                                    RichText::new("CLI not found. Run the command above and try again.")
                                        .size(style::TEXT_CAPTION)
                                        .color(colors.accent),
                                );
                                ui.add_space(style::SPACE_SM);
                            }

                            ui.horizontal(|ui| {
                                let check_btn = ui.add(
                                    egui::Button::new(
                                        RichText::new("Check for success")
                                            .size(style::TEXT_BODY)
                                            .color(colors.text_primary),
                                    )
                                    .fill(colors.bg_active)
                                    .min_size(egui::vec2(150.0, 28.0)),
                                );

                                if check_btn.clicked() {
                                    log::info!("cli_setup: user clicked Check for success");
                                    if crate::cli::setup::is_installed() {
                                        log::info!("cli_setup: CLI found - closing modal");
                                        crate::cli::setup::mark_prompted();
                                        self.cli_setup_check_result = None;
                                        self.show_cli_setup_prompt = false;
                                    } else {
                                        log::info!("cli_setup: CLI not found - showing retry");
                                        self.cli_setup_check_result = Some(false);
                                    }
                                }

                                let skip_btn = ui.add(
                                    egui::Button::new(
                                        RichText::new("Not now")
                                            .size(style::TEXT_BODY)
                                            .color(colors.text_dim),
                                    )
                                    .min_size(egui::vec2(100.0, 28.0)),
                                );

                                if skip_btn.clicked() {
                                    log::info!("cli_setup: user chose Not now - will ask again next launch");
                                    self.show_cli_setup_prompt = false;
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

    pub(crate) fn draw_completions_banner(&mut self, ctx: &egui::Context) {
        if !self.show_completions_banner {
            return;
        }
        let colors = self.colors;
        let cmd = crate::cli::setup::INSTALL_COMMAND;

        egui::Area::new(egui::Id::new("completions_banner"))
            .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -20.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(colors.bg_sidebar)
                    .stroke(Stroke::new(1.0, colors.border))
                    .corner_radius(R6)
                    .inner_margin(egui::Margin::symmetric(16, 10))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("Shell completions aren't set up.")
                                    .size(style::TEXT_CAPTION)
                                    .color(colors.text_dim),
                            );
                            ui.add_space(style::SPACE_SM);
                            egui::Frame::new()
                                .fill(colors.bg_darkest)
                                .corner_radius(R6)
                                .inner_margin(egui::Margin::symmetric(8, 4))
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            RichText::new(cmd)
                                                .size(style::TEXT_HINT)
                                                .color(colors.text_primary)
                                                .monospace(),
                                        );
                                        ui.add_space(4.0);
                                        crate::ui::widgets::copy_button(
                                            ui,
                                            egui::Id::new("completions_banner_copy"),
                                            cmd,
                                        );
                                    });
                                });
                            ui.add_space(style::SPACE_SM);
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("Done")
                                            .size(style::TEXT_CAPTION)
                                            .color(colors.text_primary),
                                    )
                                    .fill(colors.bg_active)
                                    .min_size(egui::vec2(50.0, 22.0)),
                                )
                                .clicked()
                            {
                                log::info!("cli_setup: completions banner — user clicked Done, marking sentinel");
                                crate::cli::setup::completions_mark_prompted();
                                self.show_completions_banner = false;
                            }
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("Not now")
                                            .size(style::TEXT_CAPTION)
                                            .color(colors.text_dim),
                                    )
                                    .min_size(egui::vec2(60.0, 22.0)),
                                )
                                .clicked()
                            {
                                log::info!("cli_setup: completions banner — user clicked Not now (session-only dismiss)");
                                self.show_completions_banner = false;
                            }
                        });
                    });
            });
    }
}

impl PlexiApp {
    pub(crate) fn cli_setup_prompt_handle_key(
        &mut self,
        _ctx: &egui::Context,
    ) -> crate::app::app_trait::KeyDisposition {
        crate::app::app_trait::KeyDisposition::Consumed
    }
}

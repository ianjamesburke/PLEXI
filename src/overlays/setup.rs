use super::*;

impl PlexiApp {
    /// Render the spatial-grid minimap in the top-right corner of the work
    /// area. Uses an `egui::Area` so it floats above all content.
    pub(crate) fn draw_minimap_overlay(&mut self, ctx: &egui::Context) {
        if !self.minimap.visible
            || !crate::release::feature_enabled(crate::release::ReleaseFeature::PageGrid)
        {
            return;
        }

        let content_rect = ctx.content_rect();
        let active_context = self.active_window;
        let colors = self.colors;
        let ws_id = self.router.active().context_id;
        let ws_name = self.router.active().name.to_string();

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
                    crate::render::minimap::MinimapRenderParams {
                        content_rect,
                        windows: &self.windows,
                        active_window: active_context,
                        last_visited: &self.last_page_x_per_row,
                        colors: &colors,
                        workspace_id: ws_id,
                        workspace_name: &ws_name,
                    },
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

        ui.scope_builder(egui::UiBuilder::new().max_rect(box_rect), |ui| {
            egui::Frame::new()
                .fill(colors.bg_sidebar)
                .stroke(Stroke::new(1.0_f32, colors.border))
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
                        let text_w = ui.fonts_mut(|f| {
                            f.layout_no_wrap("PLEXI".to_string(), font_id, colors.text_primary)
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
                            let rx = crate::ui::style::radius_frac(scale, 1.5);
                            let outline =
                                Stroke::new(0.9 * scale, egui::Color32::from_rgb(0xe4, 0xe4, 0xe7));
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
                    ui.add_space(style::SPACE_MD);
                    ui.label(
                        RichText::new(
                            "Brand new to Plexi? Press Command+N to open a new pane, \
                             then type plexi demo.",
                        )
                        .size(style::TEXT_CAPTION)
                        .color(colors.text_dim),
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
                        let combos = [*keys];
                        crate::ui::shortcuts::key_combo_list_with_body(
                            ui,
                            &combos,
                            &colors,
                            |ui| {
                                ui.add(
                                    egui::Label::new(
                                        crate::ui::shortcuts::shortcut_hint_label(desc, &colors)
                                            .size(style::TEXT_BODY),
                                    )
                                    .wrap(),
                                );
                            },
                        );
                        ui.add_space(style::SPACE_SM);
                    }

                    ui.add_space(style::SPACE_XL);

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
}

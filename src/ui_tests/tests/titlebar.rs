use super::*;

#[test]
fn native_buttons_safe_area_respects_scale_and_geometry() {
    for scale in [0.8_f32, 1.0, 1.3, 2.0] {
        for width in [400.0_f32, 700.0, 1600.0] {
            let mut h = PlexiUiHarness::new_sized_scaled(width, 600.0, scale);
            h.with_app_mut(|app| {
                app.ctx.data_mut(|d| {
                    d.insert_temp(
                        egui::Id::new("native_titlebar_test_metrics"),
                        egui::vec2(110.0, 32.0),
                    )
                });
            });
            h.run_steps(3);
            let (screen, color, row) = h.with_app(|app| {
                (
                    app.ctx.content_rect(),
                    app.colors.bg_toolbar,
                    app.ctx
                        .data(|d| d.get_temp::<egui::Rect>(egui::Id::new("titlebar_row_rect")))
                        .unwrap(),
                )
            });
            let height = h
                .harness()
                .output()
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Rect(r)
                        if r.fill == color
                            && (r.rect.width() - screen.width()).abs() < 1.0
                            && (r.rect.top() - screen.top()).abs() < 1.0 =>
                    {
                        Some(r.rect.height())
                    }
                    _ => None,
                })
                .expect("toolbar background");
            assert!(
                (height - crate::ui::style::TOOLBAR_H.max(32.0 / scale)).abs() < 0.1,
                "scale {scale}: wrong toolbar height {height}"
            );
            assert!(
                row.left() + 0.1 >= 110.0 / scale,
                "scale {scale}: toolbar enters native button area: {row:?}"
            );
            #[cfg(target_os = "macos")]
            h.with_app(|app| {
                let title = app
                    .ctx
                    .data(|d| d.get_temp::<egui::Rect>(egui::Id::new("titlebar_title_rect")))
                    .unwrap();
                assert!(
                    !title.is_positive() || title.left() + 0.1 >= 110.0 / scale,
                    "title enters native button area: {title:?}"
                );
            });
        }
    }
}

#[cfg(target_os = "macos")]
#[test]
fn titlebar_title_never_overlaps_toolbar_at_any_scale_or_width() {
    let mut saw_centered = false;
    let mut saw_shifted = false;
    let mut saw_elided = false;
    let mut saw_hidden = false;
    for scale in [0.8_f32, 1.0, 1.3, 1.6, 2.0] {
        for width in [
            200.0_f32, 250.0, 300.0, 350.0, 400.0, 425.0, 450.0, 475.0, 500.0, 525.0, 550.0,
            575.0, 600.0, 700.0, 800.0, 1000.0, 1600.0,
        ] {
            let mut h = PlexiUiHarness::new_sized_scaled(width, 600.0, scale);
            h.with_app_mut(|app| {
                app.router.get_mut(0).name = "attention demo — needs_you".to_string().into()
            });
            h.run_steps(3);
            h.with_app(|app| {
                let get = |name| app.ctx.data(|d| d.get_temp::<egui::Rect>(egui::Id::new(name)));
                let title = get("titlebar_title_rect").expect("title geometry must be published");
                let visible = app.ctx.data(|d| d.get_temp::<String>(egui::Id::new("titlebar_visible_title"))).unwrap();
                if visible.is_empty() {
                    saw_hidden = true;
                } else if visible.ends_with('…') {
                    saw_elided = true;
                } else if (title.center().x - app.ctx.content_rect().center().x).abs() < 0.1 {
                    saw_centered = true;
                } else {
                    saw_shifted = true;
                }
                for name in ["help", "version", "attention_mailbox", "gallery"] {
                    let item = app.ctx.data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("toolbar_ctl_rect", name))))
                        .or_else(|| app.ctx.data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("toolbar_ctl_rect", "overflow"))))).expect("toolbar item or named overflow");
                    assert!(!title.intersects(item), "scale {scale}, width {width}: title {title:?} overlaps {name} {item:?}");
                }
            });
        }
    }
    assert!(saw_centered && saw_shifted && saw_elided && saw_hidden, "must exercise center, shift, truncate, and hide: centered={saw_centered} shifted={saw_shifted} elided={saw_elided} hidden={saw_hidden}");
}

#[cfg(target_os = "macos")]
#[test]
fn titlebar_compact_controls_preserve_native_exclusion() {
    for scale in [1.0, 1.3, 2.0] {
        let mut h = PlexiUiHarness::new_sized_scaled(400.0, 600.0, scale);
        h.with_app_mut(|app| {
            app.ctx.data_mut(|d| {
                d.insert_temp(
                    egui::Id::new("native_titlebar_test_metrics"),
                    egui::vec2(110.0, 32.0),
                )
            })
        });
        h.run_steps(3);
        let rects = h.with_app(|app| {
            let row = app
                .ctx
                .data(|d| d.get_temp::<egui::Rect>(egui::Id::new("titlebar_row_rect")))
                .unwrap();
            let rects: Vec<_> = [
                "gallery",
                "help",
                "version",
                "attention_mailbox",
                "overflow",
            ]
            .into_iter()
            .filter_map(|name| {
                app.ctx
                    .data(|d| {
                        d.get_temp::<egui::Rect>(egui::Id::new(("toolbar_ctl_rect", name)))
                    })
                    .map(|rect| (name, rect))
            })
            .collect();
            for (name, rect) in &rects {
                assert!(
                    rect.left() >= row.left() && rect.right() <= row.right(),
                    "{scale}: {name} {rect:?} outside {row:?}"
                );
            }
            rects
        });
        if let Some((_, rect)) = rects.iter().find(|(name, _)| *name == "overflow") {
            assert!(h.harness().query_by_label("Toolbar actions").is_some());
            for pressed in [true, false] {
                h.harness().input_mut().events.extend([
                    egui::Event::PointerMoved(rect.center()),
                    egui::Event::PointerButton {
                        pos: rect.center(),
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ]);
                h.step();
            }
            assert!(h.harness().query_by_label("Changelog").is_some());
            assert!(h.harness().query_by_label("Keyboard shortcuts").is_some());
        }
    }
}

#[cfg(target_os = "macos")]
#[test]
fn screenshot_titlebar_scales() {
    for theme in ["plexi-dusk", "plexi-dawn"] {
        for scale in [0.8_f32, 1.0, 1.3, 2.0] {
            for width in [400.0_f32, 700.0, 1600.0] {
                let mut h = PlexiUiHarness::new_sized_scaled(width, 600.0, scale);
                std::fs::write(
                    h.workspace_root().join("review-notes.txt"),
                    "Review the pending request\n",
                )
                .unwrap();
                let id = add_focused_pane(&mut h);
                h.with_app_mut(|app| {
                    app.ctx.data_mut(|d| {
                        d.insert_temp(
                            egui::Id::new("native_titlebar_test_metrics"),
                            egui::vec2(70.0, 28.0),
                        )
                    });
                    let cfg = crate::ui::theme::preset_colors(theme).unwrap();
                    app.colors = crate::ui::theme::Colors::from_config(&cfg);
                    app.theme = crate::ui::theme::terminal_theme(
                        &cfg,
                        crate::config::DEFAULT_TERMINAL_MINIMUM_CONTRAST,
                    );
                    crate::ui::theme::setup_style(&app.ctx, &app.colors);
                    app.router.get_mut(0).name = "attention demo".to_string().into();
                    app.windows[app.active_window]
                        .panes
                        .get_mut(&id)
                        .unwrap()
                        .as_view_mut()
                        .unwrap()
                        .name = "needs_you".into();
                    app.enqueue_ask(
                        "titlebar-fixture",
                        id,
                        "Review the pending request",
                        1_000,
                        Vec::new(),
                    );
                });
                h.run_steps(4);
                // Registered toolbar fixture: the title, attention chip and
                // actions fit, contrast and clear the native safe area at
                // every width, zoom and theme.
                assert_layout_surface(&h, "titlebar");
                h.save_screenshot(&evidence_png!(format!("_{theme}_{scale}_{width}")))
                    .unwrap();
            }
        }
    }
}

/// Sample the pixel one point inside the left edge of `rect`, away from the label.
fn edge_pixel(image: &image::RgbaImage, rect: egui::Rect) -> image::Rgba<u8> {
    *image.get_pixel(rect.left() as u32 + 3, rect.center().y as u32)
}

/// Hover `rect` and render the frame.
fn render_hovering(h: &mut PlexiUiHarness, rect: egui::Rect) -> image::RgbaImage {
    h.harness()
        .input_mut()
        .events
        .push(egui::Event::PointerMoved(rect.center()));
    h.run_steps(2);
    h.render().expect("render hovered menu")
}

/// Stint 1211: kit menu items keep egui's hover highlight (a Ghost button
/// is transparent at rest and paints the theme hover fill). Covers the
/// sidebar context menu and the toolbar overflow menu in both themes.
#[cfg(target_os = "macos")]
#[test]
fn screenshot_context_and_overflow_menu_hover_states() {
    for theme in ["plexi-dusk", "plexi-dawn"] {
        let apply_theme = |h: &mut PlexiUiHarness| {
            h.with_app_mut(|app| {
                let cfg = crate::ui::theme::preset_colors(theme).unwrap();
                app.colors = crate::ui::theme::Colors::from_config(&cfg);
                app.theme = crate::ui::theme::terminal_theme(
                    &cfg,
                    crate::config::DEFAULT_TERMINAL_MINIMUM_CONTRAST,
                );
                crate::ui::theme::setup_style(&app.ctx, &app.colors);
            });
        };

        // Sidebar context menu: secondary click on the first context row.
        let mut h = PlexiUiHarness::new_sized(900.0, 600.0);
        apply_theme(&mut h);
        h.with_app_mut(|app| app.sidebar_visible = true);
        h.run_steps(4);
        let name = h.with_app(|app| app.router.get(0).name.to_string());
        // The context name also labels the title bar; the sidebar row is the leftmost match.
        let row = h
            .harness()
            .get_all_by_label(&name)
            .map(|node| node.rect())
            .min_by(|a, b| a.left().total_cmp(&b.left()))
            .expect("sidebar context row");
        for pressed in [true, false] {
            h.harness().input_mut().events.extend([
                egui::Event::PointerMoved(row.center()),
                egui::Event::PointerButton {
                    pos: row.center(),
                    button: egui::PointerButton::Secondary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]);
            h.step();
        }
        h.run_steps(3);
        let rename = h.harness().get_by_label("Rename").rect();
        let description = h.harness().get_by_label("Edit Description").rect();
        let at_rest = h.render().expect("render context menu");
        let rest_pixel = edge_pixel(&at_rest, rename);
        h.save_screenshot(&evidence_png!(format!("_context_rest_{theme}")))
            .unwrap();
        let hovered = render_hovering(&mut h, rename);
        h.save_screenshot(&evidence_png!(format!("_context_hover_{theme}")))
            .unwrap();
        assert_ne!(
            edge_pixel(&hovered, rename),
            rest_pixel,
            "{theme}: hovered context item must be highlighted"
        );
        assert_eq!(
            edge_pixel(&hovered, description),
            edge_pixel(&at_rest, description),
            "{theme}: only the hovered item is highlighted"
        );

        // Toolbar overflow menu: compact titlebar.
        // 300 points minus the 110pt native safe area is narrower than the
        // full toolbar, so the titlebar is compact. Zoom stays 1.0 so points are pixels.
        let mut h = PlexiUiHarness::new_sized(300.0, 600.0);
        apply_theme(&mut h);
        h.with_app_mut(|app| {
            app.ctx.data_mut(|d| {
                d.insert_temp(
                    egui::Id::new("native_titlebar_test_metrics"),
                    egui::vec2(110.0, 32.0),
                )
            })
        });
        h.run_steps(3);
        let trigger = h.harness().get_by_label("Toolbar actions").rect();
        for pressed in [true, false] {
            h.harness().input_mut().events.extend([
                egui::Event::PointerMoved(trigger.center()),
                egui::Event::PointerButton {
                    pos: trigger.center(),
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]);
            h.step();
        }
        h.run_steps(2);
        let shortcuts = h.harness().get_by_label("Keyboard shortcuts").rect();
        let changelog = h.harness().get_by_label("Changelog").rect();
        let at_rest = h.render().expect("render overflow menu");
        h.save_screenshot(&evidence_png!(format!("_overflow_rest_{theme}")))
            .unwrap();
        let hovered = render_hovering(&mut h, changelog);
        h.save_screenshot(&evidence_png!(format!("_overflow_hover_{theme}")))
            .unwrap();
        assert_ne!(
            edge_pixel(&hovered, changelog),
            edge_pixel(&at_rest, changelog),
            "{theme}: hovered overflow item must be highlighted"
        );
        assert_eq!(
            edge_pixel(&hovered, shortcuts),
            edge_pixel(&at_rest, shortcuts),
            "{theme}: only the hovered item is highlighted"
        );
    }
}

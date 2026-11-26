use super::*;

#[test]
fn quit_toast_hugs_content_within_bound() {
    let mut h = PlexiUiHarness::new_sized_scaled(720.0, 800.0, 1.0);
    h.with_app_mut(|app| {
        app.quit_coordinator.hold_keyboard_presses(1);
    });
    h.run_steps(3);
    let rect = h.with_app(|app| {
        app.ctx
            .memory(|m| m.area_rect(egui::Id::new("quit_confirm_overlay")))
            .unwrap()
    });
    assert!(
        rect.width() < crate::ui::style::TOAST_MAX_WIDTH - 80.0,
        "quit toast should hug content: {rect:?}"
    );
}

#[test]
fn screenshot_toast_shrink_variants() {
    for theme in ["plexi-dusk", "plexi-dawn"] {
        for title in [
            "Meeting",
            "Release review with a very long title ".repeat(20).as_str(),
        ] {
            let mut h = PlexiUiHarness::new_sized_scaled(720.0, 800.0, 1.0);
            let path = h.workspace_root().join("meeting.md");
            std::fs::write(&path, "Meeting notes").unwrap();
            add_focused_pane(&mut h);
            h.with_app_mut(|app| {
                let cfg = crate::ui::theme::preset_colors(theme).unwrap();
                app.colors = crate::ui::theme::Colors::from_config(&cfg);
                app.theme = crate::ui::theme::terminal_theme(
                    &cfg,
                    crate::config::DEFAULT_TERMINAL_MINIMUM_CONTRAST,
                );
                crate::ui::theme::setup_style(&app.ctx, &app.colors);
                app.notes_picker_entries = vec![crate::notes::NotePickerEntry {
                    path: path.clone(),
                    title: title.into(),
                    preview: String::new(),
                    search_text: String::new(),
                    tier_label: None,
                    modified: None,
                }];
                app.push_focus_layer(crate::app::FocusKind::NotesPicker);
                assert!(app.notes_picker_delete_entry(0));
                app.notes_picker_shown_paths.clear();
            });
            h.run_steps(3);
            let rect = h.with_app(|app| {
                app.ctx
                    .memory(|m| m.area_rect(egui::Id::new("notes_delete_toast")))
                    .unwrap()
            });
            assert!(
                rect.width() <= crate::ui::style::TOAST_MAX_WIDTH + 1.0,
                "action exceeds bound: {rect:?}"
            );
            if title == "Meeting" {
                assert!(
                    rect.width() < crate::ui::style::TOAST_MAX_WIDTH - 80.0,
                    "short action must hug content: {rect:?}"
                );
            }
            assert!(h.harness().query_by_label("Undo").is_some());
            let kind = if title == "Meeting" { "short" } else { "long" };
            h.save_screenshot(&evidence_png!(&format!("_{theme}_{kind}")))
                .unwrap();
            h.with_app_mut(|app| {
                app.notes_picker_undo = None;
            });
            for count in [1, 2] {
                h.with_app_mut(|app| app.quit_coordinator.hold_keyboard_presses(count));
                h.run_steps(3);
                h.save_screenshot(&evidence_png!(&format!("_{theme}_quit_{count}")))
                    .unwrap();
            }
        }
    }
}

#[test]
fn bounded_toast_long_title_keeps_named_undo() {
    for keyboard in [false, true] {
        let mut h = PlexiUiHarness::new_sized_scaled(520.0, 600.0, 2.0);
        let path = h.workspace_root().join("long-note.md");
        std::fs::write(&path, "original bytes").unwrap();
        h.with_app_mut(|app| {
            app.notes_picker_entries = vec![crate::notes::NotePickerEntry {
                path: path.clone(),
                title: "Release review with an adversarially long note title ".repeat(20),
                preview: String::new(),
                search_text: String::new(),
                tier_label: None,
                modified: None,
            }];
            app.push_focus_layer(crate::app::FocusKind::NotesPicker);
            assert!(app.notes_picker_delete_entry(0));
            app.notes_picker_shown_paths.clear();
        });
        h.run_steps(3);
        let screen = h.with_app(|app| app.ctx.content_rect());
        let painted_undo =
            h.harness()
                .output()
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == "Undo" => {
                        Some(egui::Rect::from_min_size(text.pos, text.galley.size()))
                    }
                    _ => None,
                });
        assert!(
            painted_undo.is_some_and(|rect| screen.contains_rect(rect)),
            "Undo must actually paint its full label inside the viewport: {painted_undo:?}"
        );
        assert!(!path.exists());
        if keyboard {
            h.harness()
                .key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
        } else {
            assert!(h.harness().query_by_label("Undo").is_some());
            let rect = h.with_app(|app| {
                app.ctx
                    .data(|data| {
                        data.get_temp::<egui::Rect>(egui::Id::new("notes_delete_undo_rect"))
                    })
                    .unwrap()
            });
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
        }
        h.run_steps(3);
        assert!(path.exists(), "Undo activation failed: keyboard={keyboard}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "original bytes");
        h.with_app(|app| {
            assert!(app
                .notes_picker_notice
                .as_ref()
                .is_some_and(|(text, _)| text.starts_with("Restored:")))
        });
    }
}

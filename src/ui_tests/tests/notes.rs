use super::*;

/// The QuickNote capture card is content-sized: an empty or one-line note
/// opens short (no screen-fraction reservation), and a tall note grows the
/// card while it stays inside the window.
#[test]
fn quick_note_card_is_content_sized_and_grows_with_content() {
    let mut h = PlexiUiHarness::new_sized(1280.0, 800.0);
    h.run_steps(2);
    h.with_app_mut(|app| app.open_quick_note_modal());
    h.run_steps(3);
    let screen = h.with_app(|app| app.ctx.content_rect());
    let empty = ui_scale_area_rect(&h, "quick_note_modal");
    assert_inside("quick note", 1.0, empty, screen);
    assert!(
        empty.height() < 200.0,
        "empty quick-note card must be content-sized, not a screen fraction: {empty:?} of {screen:?}"
    );

    h.with_app_mut(|app| {
        app.quick_note_text = (1..=40)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
    });
    h.run_steps(3);
    let long = ui_scale_area_rect(&h, "quick_note_modal");
    assert!(
        long.height() > empty.height(),
        "long note must grow the card"
    );
    assert!(
        long.height() <= screen.height(),
        "long quick-note card leaves the window: {long:?}"
    );
}

/// A staged QuickNote attachment renders an inline preview (stint 1047):
/// the card grows to fit a decodable image, and a staged file that fails
/// to decode falls back to the shared placeholder instead of an empty gap.
#[test]
fn quick_note_staged_attachment_renders_preview_or_placeholder() {
    let mut h = PlexiUiHarness::new_sized(1280.0, 800.0);
    h.run_steps(2);
    h.with_app_mut(|app| {
        app.open_quick_note_modal();
        app.quick_note_text = "look at this".to_string();
    });
    h.run_steps(3);
    let baseline = ui_scale_area_rect(&h, "quick_note_modal");

    let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/notes-drop.png");
    h.with_app_mut(|app| {
        app.stage_quick_note_image(fixture.clone())
            .expect("fixture must decode")
    });
    h.run_steps(3);
    let with_preview = ui_scale_area_rect(&h, "quick_note_modal");
    assert!(
        with_preview.height() > baseline.height() + 40.0,
        "a staged attachment must render a preview taller than the placeholder: \
             {baseline:?} -> {with_preview:?}"
    );

    // The same path, corrupted after staging: the shared placeholder must
    // still paint (grow the card) rather than the full image or a gap.
    let broken = std::env::temp_dir().join(format!("plexi-1047-{}.png", std::process::id()));
    std::fs::copy(&fixture, &broken).expect("copy fixture");
    h.with_app_mut(|app| {
        app.quick_note_attachments.clear();
        app.stage_quick_note_image(broken.clone())
            .expect("staged before corruption")
    });
    std::fs::write(&broken, b"not an image").expect("corrupt staged file");
    h.run_steps(3);
    let with_placeholder = ui_scale_area_rect(&h, "quick_note_modal");
    let _ = std::fs::remove_file(&broken);
    assert!(
        with_placeholder.height() > baseline.height(),
        "an undecodable staged attachment must paint its placeholder: \
             {baseline:?} -> {with_placeholder:?}"
    );
    assert!(
        with_placeholder.height() < with_preview.height(),
        "the placeholder must be shorter than the decoded preview: \
             {with_placeholder:?} vs {with_preview:?}"
    );
}

/// The notes picker keeps its preview half only while the modal is at
/// least `MODAL_WIDTH_MD` wide; narrower, the list takes the whole modal.
/// At every size the modal stays on screen and its rows stay inside
/// their allocations.
#[test]
fn screenshot_notes_picker_wide_and_narrow_layout() {
    // A body line only the preview half paints: list rows show the
    // note's first line, never this one.
    const PREVIEW_ONLY: &str = "Close P1 focus layer input leaks";
    for (width, height, scale, suffix, preview) in [
        (1440.0, 900.0, 1.0, "_wide", Some(true)),
        (640.0, 520.0, 1.0, "_narrow", Some(false)),
        (1440.0, 900.0, 1.5, "_scale", None),
    ] {
        let mut h = PlexiUiHarness::new_sized_scaled(width, height, scale);
        add_focused_pane(&mut h);
        seed_note_picker(&mut h);
        h.run_steps(3);
        let screen = h.with_app(|app| app.ctx.content_rect());
        assert_inside(
            "notes picker",
            scale,
            ui_scale_area_rect(&h, "notes_picker"),
            screen,
        );
        assert_layout_surface(&h, "list");
        if let Some(expected) = preview {
            let shown = painted_texts(&mut h)
                .iter()
                .any(|(_, text)| text.contains(PREVIEW_ONLY));
            assert_eq!(
                shown, expected,
                "preview half at {width}x{height} must be shown={expected}"
            );
        }
        h.save_screenshot(&evidence_png!(suffix)).unwrap();
    }
}

/// Stint 1420: a captured note (envelope, then a blank line, then text)
/// previews with its first line at the top of the preview, not one blank
/// line down. Pinned by `note_body_starts_at_the_first_line_after_the_capture_envelope`;
/// this PNG is the before/after evidence.
#[test]
fn screenshot_notes_picker_preview_starts_at_the_first_line() {
    let mut h = PlexiUiHarness::new_sized(1440.0, 900.0);
    add_focused_pane(&mut h);
    let path =
        std::env::temp_dir().join(format!("plexi-1420-capture-{}.md", std::process::id()));
    std::fs::write(
        &path,
        "---\ntitle: \"\"\ncaptured_at: \"2026-11-13T05:35:00Z\"\nsource: \"scratchpad\"\n---\n\nFirst line of the note\nsecond line\n",
    )
    .expect("write capture");
    h.with_app_mut(|app| {
        app.open_notes_picker();
        app.notes_picker_entries = vec![crate::notes::NotePickerEntry {
            path: path.clone(),
            title: "First line of the note".to_string(),
            preview: "First line of the note".to_string(),
            search_text: "first line of the note".to_string(),
            tier_label: None,
            modified: None,
        }];
        app.notes_picker_selected = 0;
        app.notes_picker_last_drawn_selected = None;
    });
    let start = Instant::now();
    let body = loop {
        h.step();
        let found = painted_texts(&mut h)
            .into_iter()
            .find(|(_, text)| text.contains("second line"));
        if let Some((_, text)) = found {
            break text;
        }
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "preview never painted"
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    h.save_screenshot(&evidence_png_path(enclosing_fn_name!(), ""))
        .expect("save screenshot");
    let _ = std::fs::remove_file(&path);
    assert!(
        body.starts_with("First line of the note"),
        "the preview must start at the note's first line: {body:?}"
    );
}

// Visual smoke coverage lives in `tests/scenes/*.toml`, executed by
// `scenes::tests::scene_suite`. Run one ad hoc with `just scene <file>`.

/// Note editor pane: frontmatter is hidden from the buffer and the
/// frontmatter title renders as a header row.
#[test]
fn text_editor_note_hides_frontmatter_and_shows_title() {
    let mut h = PlexiUiHarness::new();
    h.step();

    let path =
        std::env::temp_dir().join(format!("plexi-ui-note-title-{}.md", std::process::id()));
    std::fs::write(
        &path,
        "---\ntitle: \"Groceries\"\nsource: \"scratchpad\"\n---\nmilk and eggs\n",
    )
    .expect("seed note");

    h.with_app_mut(|app| {
        let pane_id = app.host.alloc_pane_id();
        let view_pane = ViewPane {
            incarnation: Default::default(),
            pip_status: None,
            id: pane_id,
            runtime: ViewRuntime::Builtin(Box::new(
                crate::app::text_editor_view::TextEditorView::new_for_test_note(path.clone()),
            )),
            workspace_root: crate::testing::scratch_context_root("ui"),
            permissions: PluginPermissions::builtin(),
            manifest_id: "text-editor".to_string(),
            name: "Text Editor".to_string(),
            pane_group: None,
            linked_pane_id: None,
            overlay_replaced: None,
            agent: None,
            slots: std::collections::HashMap::new(),
            semantic_state: Default::default(),
        };
        let win = &mut app.windows[app.active_window];
        win.panes
            .insert(pane_id, Pane::View(Box::new(view_pane)))
            .unwrap();
        let tile_id = win.tree.tiles.insert_pane(pane_id);
        if win.tree.root.is_none() {
            win.tree.root = Some(tile_id);
        }
        win.focused_pane = Some(tile_id);
    });
    h.run_steps(2);

    // Title row is a real ui.label; the YAML block lives only in
    // `note_header` and never reaches the TextEdit buffer (unit-tested
    // via `split_note` in text_editor_app.rs). The file name and save
    // indicator share that header with a file opened from the browser.
    let file_name = path.file_name().unwrap().to_string_lossy().into_owned();
    h.harness().get_by_label("Groceries");
    h.harness().get_by_label(&file_name);
    h.harness().get_by_label("Saved");
    let leaked: Vec<_> = painted_texts(&mut h)
        .into_iter()
        .filter(|(_, text)| text.contains("source:"))
        .collect();
    assert!(leaked.is_empty(), "frontmatter painted: {leaked:?}");
    h.save_screenshot(&evidence_png!()).expect("render failed");

    let _ = std::fs::remove_file(&path);
}

/// A markdown file opened outside the notes tier (the file-browser path)
/// uses the same header as a note: the file name and the save indicator.
#[test]
fn text_editor_file_header_shows_file_name_and_save_indicator() {
    let mut h = PlexiUiHarness::new();
    h.step();

    let path =
        std::env::temp_dir().join(format!("plexi-ui-file-header-{}.md", std::process::id()));
    std::fs::write(&path, "# hello\n").expect("seed file");
    let file_name = path.file_name().unwrap().to_string_lossy().into_owned();

    h.with_app_mut(|app| {
        let pane_id = app.host.alloc_pane_id();
        let view_pane = ViewPane {
            incarnation: Default::default(),
            pip_status: None,
            id: pane_id,
            runtime: ViewRuntime::Builtin(Box::new(
                crate::app::text_editor_view::TextEditorView::new(path.clone()),
            )),
            workspace_root: crate::testing::scratch_context_root("ui"),
            permissions: PluginPermissions::builtin(),
            manifest_id: "text-editor".to_string(),
            name: file_name.clone(),
            pane_group: None,
            linked_pane_id: None,
            overlay_replaced: None,
            agent: None,
            slots: std::collections::HashMap::new(),
            semantic_state: Default::default(),
        };
        let win = &mut app.windows[app.active_window];
        win.panes
            .insert(pane_id, Pane::View(Box::new(view_pane)))
            .unwrap();
        let tile_id = win.tree.tiles.insert_pane(pane_id);
        if win.tree.root.is_none() {
            win.tree.root = Some(tile_id);
        }
        win.focused_pane = Some(tile_id);
    });
    h.run_steps(2);

    h.harness().get_by_label(&file_name);
    h.harness().get_by_label("Saved");

    let _ = std::fs::remove_file(&path);
}

/// Stint 0786: a plain file shows the dirty dot, the close prompt and the
/// conflict state in the header/prompt row; closing a dirty file through
/// the host close path is vetoed until the user answers.
#[test]
fn text_editor_file_dirty_close_and_conflict_surfaces() {
    let mut h = PlexiUiHarness::new_sized(900.0, 320.0);
    h.step();
    let path = std::env::temp_dir().join(format!("plexi-ui-0786-{}.txt", std::process::id()));
    std::fs::write(&path, "fn main() {}\n").expect("seed file");
    let file_name = path.file_name().unwrap().to_string_lossy().into_owned();

    let pane_id = h.with_app_mut(|app| {
        let pane_id = app.host.alloc_pane_id();
        let view_pane = ViewPane {
            incarnation: Default::default(),
            pip_status: None,
            id: pane_id,
            runtime: ViewRuntime::Builtin(Box::new(
                crate::app::text_editor_view::TextEditorView::new(path.clone()),
            )),
            workspace_root: crate::testing::scratch_context_root("ui"),
            permissions: PluginPermissions::builtin(),
            manifest_id: "text-editor".to_string(),
            name: file_name.clone(),
            pane_group: None,
            linked_pane_id: None,
            overlay_replaced: None,
            agent: None,
            slots: std::collections::HashMap::new(),
            semantic_state: Default::default(),
        };
        let win = &mut app.windows[app.active_window];
        win.panes
            .insert(pane_id, Pane::View(Box::new(view_pane)))
            .unwrap();
        let tile_id = win.tree.tiles.insert_pane(pane_id);
        if win.tree.root.is_none() {
            win.tree.root = Some(tile_id);
        }
        win.focused_pane = Some(tile_id);
        pane_id
    });
    let with_editor =
        |h: &mut PlexiUiHarness,
         f: &mut dyn FnMut(&mut crate::app::text_editor_view::TextEditorView)| {
            h.with_app_mut(|app| {
                let win = &mut app.windows[app.active_window];
                if let Some(Pane::View(a)) = win.panes.get_mut(&pane_id) {
                    if let ViewRuntime::Builtin(b) = &mut a.runtime {
                        let ed = b
                            .as_any_mut()
                            .downcast_mut::<crate::app::text_editor_view::TextEditorView>()
                            .expect("text editor");
                        f(ed);
                    }
                }
            });
        };
    h.run_steps(2);
    h.harness().get_by_label("Saved");

    // Dirty: the dot and the Unsaved label, and no autosave to disk.
    with_editor(&mut h, &mut |ed| {
        ed.test_set_unsaved("fn main() { println!(\"edited\"); }\n")
    });
    h.run_steps(2);
    h.harness().get_by_label("Unsaved");
    // The zoom path bar's column pips also render "●", so only require the dirty dot.
    assert!(h.harness().get_all_by_label("\u{25CF}").count() >= 1);
    h.save_screenshot(&evidence_png!("_dirty"))
        .expect("render dirty");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "fn main() {}\n");

    // Host close path: vetoed, prompt shown, pane still there.
    h.with_app_mut(|app| {
        assert!(
            app.focused_view_vetoes_close(),
            "dirty plain file vetoes close"
        );
        assert!(app.windows[app.active_window].panes.contains_key(&pane_id));
    });
    h.run_steps(2);
    h.harness().get_by_label("Unsaved changes");
    h.save_screenshot(&evidence_png!("_close_prompt"))
        .expect("render close prompt");

    // Cancel, then an agent writes the file: conflict state.
    with_editor(&mut h, &mut |ed| {
        use crate::app::view_trait::View as _;
        assert!(!ed.request_close());
    });
    std::fs::write(&path, "agent version\n").expect("external write");
    with_editor(&mut h, &mut |ed| {
        assert!(ed.test_check_disk());
    });
    h.run_steps(2);
    h.harness().get_by_label("Changed on disk");
    h.save_screenshot(&evidence_png!("_conflict"))
        .expect("render conflict");

    with_editor(&mut h, &mut |ed| ed.test_discard_unsaved());
    let _ = std::fs::remove_file(&path);
}

/// Visual review surface for tiered notes (stint 0746): the picker as a root
/// context's "master view" — the active context's own tier unchipped, notes
/// rolled up from nested tiers chipped with their relative path, and the
/// global tier last. Geometry assertions cannot prove the chips read well, so
/// this renders realistic seeded content.
#[test]
fn screenshot_notes_picker_tier_chips() {
    let mut h = PlexiUiHarness::new_sized(900.0, 760.0);
    h.step();
    add_focused_pane(&mut h);
    h.step();

    h.with_app_mut(|app| {
        app.open_notes_picker();
        let mk =
            |title: &str, preview: &str, label: Option<&str>| crate::notes::NotePickerEntry {
                path: std::env::temp_dir().join(format!("plexi-tier-chip-{title}.md")),
                title: title.to_string(),
                preview: preview.to_string(),
                search_text: title.to_lowercase(),
                tier_label: label.map(|s| s.to_string()),
                modified: None,
            };
        app.notes_picker_entries = vec![
            mk(
                "north star — what v1 has to prove",
                "one workspace you never leave",
                None,
            ),
            mk("pricing notes", "seat-based vs usage, land on seats", None),
            mk(
                "wasm wire format decisions",
                "postcard over JSON for frame payloads",
                Some("crates/runtime"),
            ),
            mk(
                "font bootstrap ordering trap",
                "ui-medium binds on the next frame, not this one",
                Some("crates/runtime"),
            ),
            mk(
                "onboarding copy pass",
                "cut the second paragraph entirely",
                Some("website"),
            ),
            mk(
                "ask Sam about the staging cert",
                "expires 2026-08-03, renewal is manual",
                Some("global"),
            ),
            mk(
                "release checklist",
                "tag, changelog, notarize, publish",
                Some("global"),
            ),
        ];
        app.notes_picker_selected = 0;
    });
    h.run_steps(2);

    let out = evidence_png!();
    h.save_screenshot(&out).expect("render failed");
    h.with_app(|app| {
        assert!(
            app.notes_picker_entries
                .iter()
                .any(|e| e.tier_label.as_deref() == Some("crates/runtime")),
            "nested-tier rows must reach the render with their relative label"
        );
        assert!(
            app.notes_picker_entries
                .iter()
                .any(|e| e.tier_label.as_deref() == Some("global")),
            "global-tier rows must reach the render chipped as global"
        );
    });
    println!("Screenshot saved to {out}");
}

/// Visual review surface for the single recency pile (stint 1615): project and
/// global notes interleaved newest first with project labels, then Tab narrows
/// to the current project.
#[test]
fn screenshot_notes_picker_recency_pile() {
    let mut h = PlexiUiHarness::new_sized(900.0, 640.0);
    h.step();
    add_focused_pane(&mut h);
    h.step();

    h.with_app_mut(|app| {
        app.open_notes_picker();
        let now = std::time::SystemTime::now();
        let mk = |title: &str, preview: &str, label: Option<&str>, mins_ago: u64| {
            let path = std::env::temp_dir().join(format!("plexi-recency-pile-{title}.md"));
            let _ = std::fs::write(&path, format!("# {title}\n\n{preview}\n"));
            crate::notes::NotePickerEntry {
                path,
                title: title.to_string(),
                preview: preview.to_string(),
                search_text: title.to_lowercase(),
                tier_label: label.map(|s| s.to_string()),
                modified: Some(now - std::time::Duration::from_secs(mins_ago * 60)),
            }
        };
        app.notes_picker_entries = vec![
            mk("release checklist", "tag, changelog, notarize, publish", Some("global"), 90),
            mk("north star: what v1 has to prove", "one workspace you never leave", None, 5),
            mk("ask Sam about the staging cert", "expires 2026-08-03", Some("global"), 30),
            mk("pricing notes", "seat-based vs usage, land on seats", None, 600),
            mk("wasm wire format decisions", "postcard over JSON", Some("crates/runtime"), 12),
            mk("onboarding copy pass", "cut the second paragraph", Some("website"), 2000),
        ];
        app.notes_picker_selected = 0;
    });
    h.run_steps(2);
    h.save_screenshot(&evidence_png!("_all")).expect("render failed");

    h.harness().key_press(egui::Key::Tab);
    h.run_steps(2);
    h.with_app(|app| assert_eq!(app.notes_picker_filtered().len(), 2));
    h.save_screenshot(&evidence_png!("_project")).expect("render failed");
}

/// Notes picker overlay: open → step → still visible and renders.
#[test]
fn notes_picker_overlay_smoke() {
    let mut h = PlexiUiHarness::new();
    h.step();
    add_focused_pane(&mut h);
    h.step();

    h.with_app_mut(|app| {
        app.open_notes_picker();
        // Deterministic entries — don't depend on the machine's notes dir.
        let mk = |name: &str, label: Option<&str>| crate::notes::NotePickerEntry {
            path: std::env::temp_dir().join(format!("plexi-ui-{name}.md")),
            title: name.to_string(),
            preview: format!("{name} preview"),
            search_text: name.to_lowercase(),
            tier_label: label.map(|s| s.to_string()),
            modified: None,
        };
        app.notes_picker_entries =
            vec![mk("context-note", None), mk("global-note", Some("global"))];
        app.notes_picker_selected = 0;
    });
    // Two steps: egui Areas spend their first frame on an invisible
    // sizing pass; the modal is visible from the second frame.
    h.run_steps(2);

    h.with_app(|app| {
        assert!(app
            .focus_stack
            .contains(&crate::app::FocusKind::NotesPicker));
    });
    // The modal title appears once and no tier header is emitted.
    let notes_labels = h.harness().get_all_by_label("Notes").count();
    assert_eq!(notes_labels, 1, "the modal title, with no duplicate header");
    assert!(h.harness().query_by_label("global").is_none(), "no tier headers");
    h.harness().get_by_label("All notes · Tab: current project");
    h.harness().key_press(egui::Key::Tab);
    h.run_steps(2);
    h.harness().get_by_label("Current project · Tab: all notes");
    h.with_app(|app| {
        assert_eq!(app.notes_picker_filtered().len(), 1);
        assert_eq!(app.notes_picker_entries[app.notes_picker_filtered()[0]].title, "context-note");
    });
    h.harness().key_press(egui::Key::Tab);
    h.run_steps(2);
    h.with_app(|app| assert_eq!(app.notes_picker_filtered().len(), 2));
    h.save_screenshot(&evidence_png!()).expect("render failed");
}

#[test]
fn screenshot_notes_delete_toast_themes() {
    for scale in [1.0, 2.0] {
        let mut h = PlexiUiHarness::new_sized_scaled(720.0, 800.0, scale);
        let root = h.workspace_root().to_path_buf();
        let removed = root.join("meeting.md");
        let kept = root.join("plans.md");
        std::fs::write(&removed, "Meeting notes\nReview the release checklist.").unwrap();
        std::fs::write(&kept, "Plans\nFinish the review.").unwrap();
        add_focused_pane(&mut h);
        h.with_app_mut(|app| {
            app.notes_picker_entries = vec![
                crate::notes::NotePickerEntry {
                    path: removed.clone(),
                    title: "Meeting notes — release review /workspaces/project/very-long-unbroken-identifier 👩🏽‍💻 ".repeat(8),
                    preview: "Review the release checklist.".into(),
                    search_text: "meeting".into(),
                    tier_label: None,
                    modified: None,
                },
                crate::notes::NotePickerEntry {
                    path: kept.clone(),
                    title: "Plans".into(),
                    preview: "Finish the review.".into(),
                    search_text: "plans".into(),
                    tier_label: None,
                    modified: None,
                },
            ];
            app.push_focus_layer(crate::app::FocusKind::NotesPicker);
            assert!(app.notes_picker_delete_entry(0));
            // Keep this explicit fixture while the unrelated background index settles.
            app.notes_picker_shown_paths.clear();
        });
        for theme in ["plexi-dusk", "plexi-dawn"] {
            h.with_app_mut(|app| {
                if app.notes_picker_undo.is_none() {
                    assert!(app.notes_picker_delete_entry(0));
                    app.notes_picker_shown_paths.clear();
                }
                app.notes_picker_status = None;
                let cfg = crate::ui::theme::preset_colors(theme).unwrap();
                app.colors = crate::ui::theme::Colors::from_config(&cfg);
                app.theme = crate::ui::theme::terminal_theme(
                    &cfg,
                    crate::config::DEFAULT_TERMINAL_MINIMUM_CONTRAST,
                );
                crate::ui::theme::setup_style(&app.ctx, &app.colors);
            });
            h.run_steps(3);
            h.save_screenshot(&evidence_png!(&format!("_{theme}_{scale}")))
                .unwrap();
            h.with_app_mut(|app| {
                app.notes_picker_status = Some(format!(
                    "Can't undo: {}",
                    "recovery path unavailable /very/long/path ".repeat(12)
                ))
            });
            h.run_steps(2);
            h.save_screenshot(&evidence_png!(&format!("_{theme}_{scale}_error")))
                .unwrap();
            h.with_app_mut(|app| {
                app.notes_picker_undo_delete();
                app.notes_picker_shown_paths.clear();
            });
            h.run_steps(2);
            h.save_screenshot(&evidence_png!(&format!("_{theme}_{scale}_restored")))
                .unwrap();
            h.with_app_mut(|app| {
                app.notes_picker_notice = None;
                app.quit_coordinator.hold_keyboard_presses(1);
            });
            h.run_steps(2);
            h.save_screenshot(&evidence_png!(&format!("_{theme}_{scale}_quit")))
                .unwrap();
            h.with_app_mut(|app| {
                app.quit_coordinator.hold_keyboard_presses(0);
                app.config_notice = Some((
                    "Reloaded. This repo overrides theme.preset (plexi-night).".into(),
                    std::time::Instant::now(),
                ));
            });
            h.run_steps(2);
            h.save_screenshot(&evidence_png!(&format!("_{theme}_{scale}_config_notice")))
                .unwrap();
            h.with_app_mut(|app| app.config_notice = None);
        }
    }
}

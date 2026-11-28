use super::*;

#[test]
fn screenshot_file_browser_middle_elision_distinguishes_suffixes() {
    let dir = tempfile::tempdir().unwrap();
    let prefix = "Q3 Board Deck — FINAL (revised) v12 [approved by legal]";
    for suffix in ["-signed.pdf", "-draft.HEIC"] {
        std::fs::write(dir.path().join(format!("{prefix}{suffix}")), "fixture").unwrap();
    }
    let mut h = PlexiUiHarness::new_sized(520.0, 420.0);
    h.open_file_browser(dir.path().to_path_buf());
    h.run_steps(3);
    let mut labels = Vec::new();
    for shape in &h.harness().output().shapes {
        if let egui::Shape::Text(text) = &shape.shape {
            if text.galley.text().starts_with("Q3") {
                labels.push(text.galley.text().to_owned());
            }
        }
    }
    assert!(
        labels
            .iter()
            .any(|s| s.contains('…') && s.ends_with("-signed.pdf")),
        "{labels:?}"
    );
    assert!(
        labels
            .iter()
            .any(|s| s.contains('…') && s.ends_with("-draft.HEIC")),
        "{labels:?}"
    );
    h.save_screenshot(&evidence_png!())
        .expect("filename screenshot");
}

#[test]
fn screenshot_file_browser_external_mutation_lifecycle() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("alpha.txt"), "first file").unwrap();
    std::fs::write(dir.path().join("bravo.txt"), "second file").unwrap();
    std::fs::write(dir.path().join("charlie.txt"), "third file").unwrap();

    let mut h = PlexiUiHarness::new_sized(680.0, 480.0);
    h.open_file_browser(dir.path().to_path_buf());
    h.run_steps(3);

    // External mutations: create, rename, delete
    std::fs::write(dir.path().join("0-from-agent.txt"), "agent content").unwrap();
    std::fs::rename(dir.path().join("charlie.txt"), dir.path().join("echo.txt")).unwrap();
    std::fs::remove_file(dir.path().join("alpha.txt")).unwrap();

    std::thread::sleep(crate::testing::load_aware_timeout(
        std::time::Duration::from_millis(300),
    ));
    h.run_steps(10);

    let shot_path = std::env::temp_dir().join("plexi-evidence-file-browser-mutation.png");
    h.save_screenshot(shot_path.to_str().unwrap())
        .expect("save screenshot of file browser after external mutation");

    assert!(shot_path.exists(), "screenshot PNG must exist");
    let bytes = std::fs::read(&shot_path).expect("read screenshot PNG");
    assert!(bytes.len() > 100, "screenshot PNG must be non-empty");
    assert_eq!(&bytes[1..4], b"PNG", "file must have PNG magic bytes");

    let _ = std::fs::remove_file(&shot_path);
}

/// Regression for stint 0351: `HostRequest::KeyPane` (the `plexi pane key`
/// socket path) must drive a native view pane through its real
/// `App::handle_key` — not the PGAP event queue, which native plugins never
/// read. Enter on the File Explorer's selected directory must run the
/// plugin's own Activate flow (navigate_into), and the response file must
/// report the disposition so driving agents can detect ignored keys.
#[test]
fn pane_key_ipc_drives_native_file_browser_handle_key() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir(dir.path().join("inner")).expect("mkdir");

    let mut h = PlexiUiHarness::new();
    h.open_file_browser(dir.path().to_path_buf());
    h.step();

    let pane_id = h.with_app(|app| {
        app.windows[app.active_window]
            .panes
            .iter()
            .find_map(|(id, p)| p.as_view().map(|_| *id))
            .expect("file browser pane must exist after open_file_browser")
    });

    let rf_dir = tempfile::tempdir().expect("tempdir");
    let rf = rf_dir.path().join("key-response.json");
    h.with_app_mut(|app| {
        app.handle_pane_ipc_request(crate::broker::caller::RequestOrigin::HostSystem, plexi_protocol::HostRequest::KeyPane {
            pane_id,
            key: "enter".to_string(),
            response_file: Some(rf.to_string_lossy().into_owned()),
        })
    });

    let content = std::fs::read_to_string(&rf).expect("KeyPane must write a response file");
    let v: serde_json::Value = serde_json::from_str(&content).expect("response must be JSON");
    assert_eq!(v["ok"], true, "response must be ok; got {content}");
    assert_eq!(
        v["disposition"], "consumed",
        "File Explorer must consume Enter; got {content}"
    );

    // Enter on the selected directory (dirs sort first, selected=0) runs
    // the plugin's real Activate flow — cwd must now be the subdirectory.
    let cwd = h.with_app(|app| {
        app.windows[app.active_window]
            .panes
            .get(&pane_id)
            .and_then(|p| p.as_view())
            .and_then(|a| a.runtime.serialize_state())
            .and_then(|s| s["cwd"].as_str().map(str::to_string))
            .expect("file browser must serialize cwd")
    });
    assert!(
        cwd.ends_with("inner"),
        "Enter must navigate into the selected directory; cwd={cwd}"
    );
}

#[test]
fn file_browser_rename_text_field_commits_on_enter() {
    let dir = tempfile::tempdir().expect("tempdir");
    let original = dir.path().join("original.txt");
    let renamed = dir.path().join("renamed.txt");
    std::fs::write(&original, b"contents").expect("write fixture");

    let mut h = PlexiUiHarness::new_sized(720.0, 520.0);
    h.open_file_browser(dir.path().to_path_buf());
    h.run_steps(2);
    h.harness()
        .key_down_modifiers(egui::Modifiers::NONE, egui::Key::F2);
    h.step();
    h.harness()
        .key_up_modifiers(egui::Modifiers::NONE, egui::Key::F2);
    h.step();
    h.run_steps(3);

    h.harness()
        .key_down_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.step();
    h.harness()
        .key_up_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.step();
    h.harness()
        .input_mut()
        .events
        .push(egui::Event::Text("renamed.txt".to_string()));
    h.step();
    h.harness()
        .key_down_modifiers(egui::Modifiers::NONE, egui::Key::Enter);
    h.step();
    h.harness()
        .key_up_modifiers(egui::Modifiers::NONE, egui::Key::Enter);
    h.step();
    h.run_steps(2);

    assert!(!original.exists(), "Enter must rename the original path");
    assert!(renamed.exists(), "Enter must create the renamed path");
    assert_eq!(
        std::fs::read(&renamed).expect("read renamed file"),
        b"contents"
    );
}

// ── Explorer dialogs on the kit (stint 1557) ───────────────────────────────

/// An Explorer over a fresh directory holding one uniquely named file
/// (unique so a copy left in the real Trash is never mistaken for this
/// run's), with that file selected. Returns the harness, the directory,
/// the file and the Explorer's pane id.
fn explorer_with_one_file(
    width: f32,
    height: f32,
) -> (
    PlexiUiHarness,
    tempfile::TempDir,
    std::path::PathBuf,
    crate::spatial::tiling::PaneId,
) {
    explorer_with_one_file_after(width, height, |_| {})
}

/// [`explorer_with_one_file`], running `before` on the harness before the
/// Explorer opens.
fn explorer_with_one_file_after(
    width: f32,
    height: f32,
    before: impl FnOnce(&mut PlexiUiHarness),
) -> (
    PlexiUiHarness,
    tempfile::TempDir,
    std::path::PathBuf,
    crate::spatial::tiling::PaneId,
) {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join(format!(
        "plexi-1557-{}.txt",
        crate::platform::clock::now_nanos()
    ));
    std::fs::write(&file, b"contents").expect("write fixture");
    let mut h = PlexiUiHarness::new_sized(width, height);
    before(&mut h);
    h.open_file_browser(dir.path().to_path_buf());
    h.run_steps(3);
    let pane = h.with_app(|app| {
        app.windows[app.active_window]
            .panes
            .iter()
            .find_map(|(id, p)| p.as_view().map(|_| *id))
            .expect("the Explorer pane")
    });
    (h, dir, file, pane)
}

fn open_explorer_trash(h: &mut PlexiUiHarness) {
    h.harness()
        .key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Backspace);
    h.run_steps(3);
    assert!(
        h.harness().query_by_label("Cancel").is_some(),
        "Cmd+Backspace opens Move to Trash"
    );
}

fn open_explorer_rename(h: &mut PlexiUiHarness) {
    h.harness().key_press(egui::Key::F2);
    h.run_steps(3);
    assert!(
        h.harness().query_by_label("Cancel").is_some(),
        "F2 opens Rename"
    );
}

/// Remove `file`'s copy from the real Trash after a test trashed it.
fn clear_from_trash(file: &std::path::Path) {
    #[cfg(target_os = "macos")]
    if let (Some(home), Some(name)) = (dirs::home_dir(), file.file_name()) {
        let _ = std::fs::remove_file(home.join(".Trash").join(name));
    }
}

/// Stint 1557: Move to Trash is destructive, so it takes only a fresh,
/// real Enter: not an Enter held from before the dialog, an injected Enter
/// or click, or Shift+Enter. A fresh Enter trashes and closes the dialog.
#[test]
fn explorer_trash_takes_only_a_fresh_real_enter() {
    // Enter goes down before the Explorer opens (in the Explorer it would
    // open the file), so the dialog sees only its repeats.
    let (mut h, _dir, file, _pane) = explorer_with_one_file_after(900.0, 600.0, |h| {
        h.harness().key_down(egui::Key::Enter);
        h.run_steps(1);
    });
    let open = |h: &mut PlexiUiHarness| h.harness().query_by_label("Cancel").is_some();
    open_explorer_trash(&mut h);
    arm_destructive(&mut h);
    for _ in 0..3 {
        h.harness().key_down(egui::Key::Enter);
        h.step();
    }
    h.run_steps(2);
    assert!(file.exists(), "a held Enter trashed the file");
    assert!(open(&mut h), "a held Enter closed the dialog");
    h.harness().key_up(egui::Key::Enter);
    h.run_steps(1);
    injected(&mut h, key_tap(egui::Key::Enter));
    assert!(file.exists(), "an injected Enter trashed the file");
    let trash = h
        .harness()
        .query_all_by_label("Move to Trash")
        .last()
        .expect("the Move to Trash button")
        .rect();
    let click = injected_click(&h, trash);
    injected(&mut h, click);
    assert!(file.exists(), "an injected click trashed the file");
    h.harness()
        .key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::Enter);
    h.run_steps(3);
    assert!(file.exists(), "Shift+Enter trashed the file");
    assert!(open(&mut h));
    h.harness().key_press(egui::Key::Enter);
    h.run_steps(3);
    assert!(!file.exists(), "a fresh real Enter trashes the file");
    assert!(!open(&mut h), "the dialog closes once the file is trashed");
    clear_from_trash(&file);
}

/// Stint 1557: Esc and Cancel close Move to Trash and keep the file.
#[test]
fn explorer_trash_esc_and_cancel_keep_the_file() {
    let (mut h, _dir, file, _pane) = explorer_with_one_file(900.0, 600.0);
    for by_button in [false, true] {
        open_explorer_trash(&mut h);
        if by_button {
            h.harness().get_by_label("Cancel").click();
        } else {
            h.harness().key_press(egui::Key::Escape);
        }
        h.run_steps(3);
        assert!(
            h.harness().query_by_label("Cancel").is_none(),
            "by_button={by_button}: the dialog closes"
        );
        assert!(file.exists(), "by_button={by_button}: the file stays");
    }
}

/// The label of the focused dialog button. A dialog's title can repeat
/// its action's label ("Move to Trash", "Rename"), so only buttons count.
fn focused_button(h: &mut PlexiUiHarness, labels: &[&str]) -> Option<String> {
    let focused = h.with_app(|app| {
        app.ctx
            .memory(|m| m.focused())
            .and_then(|id| app.ctx.read_response(id))
            .map(|r| r.rect)
    })?;
    labels
        .iter()
        .find(|label| {
            h.harness()
                .query_all_by(|node| {
                    node.role() == egui::accesskit::Role::Button
                        && node.label().as_deref() == Some(**label)
                })
                .any(|node| node.rect().intersects(focused))
        })
        .map(|label| label.to_string())
}

/// Stints 1493, 1505, 1514 on Move to Trash: Tab enters at Cancel (the
/// safe action), steps to Move to Trash and wraps both ways; Enter on the
/// focused Cancel cancels.
#[test]
fn explorer_trash_tab_enters_at_cancel_and_wraps() {
    let labels = ["Cancel", "Move to Trash"];
    let (mut h, _dir, file, _pane) = explorer_with_one_file(900.0, 600.0);
    open_explorer_trash(&mut h);
    arm_destructive(&mut h);
    let mut seen = Vec::new();
    for _ in 0..3 {
        tab(&mut h, false);
        seen.push(focused_button(&mut h, &labels));
    }
    assert_eq!(
        seen,
        ["Cancel", "Move to Trash", "Cancel"].map(|l| Some(l.to_string())),
        "Tab enters at Cancel and wraps"
    );
    tab(&mut h, true);
    assert_eq!(
        focused_button(&mut h, &labels).as_deref(),
        Some("Move to Trash"),
        "Shift+Tab from Cancel wraps to Move to Trash"
    );
    tab(&mut h, true);
    assert_eq!(focused_button(&mut h, &labels).as_deref(), Some("Cancel"));
    h.harness().key_press(egui::Key::Enter);
    h.run_steps(3);
    assert!(
        file.exists(),
        "Enter on the focused Cancel trashed the file"
    );
    assert!(h.harness().query_by_label("Cancel").is_none());
}

/// Stint 1557 on Rename: the field takes focus on open; Tab walks Rename,
/// Cancel, and wraps to the field; Enter on the focused Cancel cancels.
#[test]
fn explorer_rename_tab_walks_field_then_buttons() {
    let (mut h, dir, file, pane) = explorer_with_one_file(900.0, 600.0);
    open_explorer_rename(&mut h);
    let field = egui::Id::new(("file_browser_rename_input", pane));
    let focused = |h: &mut PlexiUiHarness| h.with_app(|app| app.ctx.memory(|m| m.focused()));
    assert_eq!(focused(&mut h), Some(field), "the field takes focus");
    let labels = ["Rename", "Cancel"];
    tab(&mut h, false);
    assert_eq!(focused_button(&mut h, &labels).as_deref(), Some("Rename"));
    tab(&mut h, false);
    assert_eq!(focused_button(&mut h, &labels).as_deref(), Some("Cancel"));
    tab(&mut h, false);
    assert_eq!(focused(&mut h), Some(field), "Tab wraps back to the field");
    tab(&mut h, true);
    assert_eq!(focused_button(&mut h, &labels).as_deref(), Some("Cancel"));
    h.harness().key_press(egui::Key::Enter);
    h.run_steps(3);
    assert!(h.harness().query_by_label("Cancel").is_none());
    assert!(
        file.exists(),
        "Enter on the focused Cancel renamed the file"
    );
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

/// Stint 1557: the Rename button commits the typed name; Esc cancels.
#[test]
fn explorer_rename_button_commits_and_esc_cancels() {
    let (mut h, dir, file, _pane) = explorer_with_one_file(900.0, 600.0);
    open_explorer_rename(&mut h);
    h.harness().key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(h.harness().query_by_label("Cancel").is_none());
    assert!(file.exists(), "Esc kept the name");
    open_explorer_rename(&mut h);
    h.harness()
        .input_mut()
        .events
        .push(egui::Event::Text("renamed.txt".to_string()));
    h.step();
    h.harness()
        .query_all_by_label("Rename")
        .last()
        .expect("the Rename button")
        .click();
    h.run_steps(3);
    assert!(!file.exists(), "the Rename button renamed the file");
    assert!(dir.path().join("renamed.txt").exists());
}

/// Visual review (stint 1557): Move to Trash and Rename are `ActionModal`s
/// ("Cancel Esc" with "Move to Trash ↵" / "Rename ↵"), at a roomy and a
/// narrow window. Reviewed against "Unsaved changes".
#[test]
fn screenshot_explorer_dialogs() {
    for (width, tag) in [(900.0, "wide"), (360.0, "narrow")] {
        let (mut h, _dir, _file, _pane) = explorer_with_one_file(width, 560.0);
        open_explorer_trash(&mut h);
        h.save_screenshot(&evidence_png!(format!("_trash_{tag}")))
            .expect("trash screenshot");
        h.harness().key_press(egui::Key::Escape);
        h.run_steps(3);
        open_explorer_rename(&mut h);
        h.save_screenshot(&evidence_png!(format!("_rename_{tag}")))
            .expect("rename screenshot");
    }
}

/// Stint 0528 evidence: file browser rows + preview panel at ppp 2.0 —
/// integer point sizes only (was 9.5/10.5).
#[test]
fn screenshot_file_browser_typography_ppp2() {
    let dir = tempfile::tempdir().expect("seed dir");
    for name in ["notes.md", "render.rs", "todo.txt"] {
        std::fs::write(
            dir.path().join(name),
            "alpha\nbravo\ncharlie\ndelta\necho\n",
        )
        .expect("seed file");
    }
    std::fs::create_dir(dir.path().join("assets")).expect("seed subdir");

    let mut h = PlexiUiHarness::new_sized_ppp(900.0, 560.0, 2.0);
    h.step();
    h.with_app_mut(|app| {
        let pane_id = app.host.alloc_pane_id();
        let view_pane = ViewPane {
            incarnation: Default::default(),
            pip_status: None,
            id: pane_id,
            runtime: ViewRuntime::Builtin(Box::new(crate::file_browser::FileBrowserView::new(
                dir.path().to_path_buf(),
            ))),
            workspace_root: dir.path().to_path_buf(),
            permissions: PluginPermissions::builtin(),
            manifest_id: "file-browser".to_string(),
            name: "Files".to_string(),
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
    h.run_steps(3);
    // Fractional point sizes land glyphs between physical pixels at 2x;
    // every row name must paint at an integer size.
    fn font_sizes(shape: &egui::Shape, out: &mut Vec<(String, f32)>) {
        match shape {
            egui::Shape::Text(text) => {
                for section in &text.galley.job.sections {
                    out.push((text.galley.text().to_owned(), section.format.font_id.size));
                }
            }
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    font_sizes(shape, out);
                }
            }
            _ => {}
        }
    }
    let mut sizes = Vec::new();
    for shape in &h.harness().output().shapes {
        font_sizes(&shape.shape, &mut sizes);
    }
    let rows: Vec<_> = sizes
        .iter()
        .filter(|(text, _)| {
            ["notes.md", "render.rs", "todo.txt"]
                .iter()
                .any(|name| text.contains(name))
        })
        .collect();
    assert!(!rows.is_empty(), "file rows must paint: {sizes:?}");
    for (text, size) in rows {
        assert_eq!(size.fract(), 0.0, "{text:?} paints at {size}pt");
    }
    h.save_screenshot(&evidence_png!()).expect("render failed");
}

#[test]
fn file_browser_responsive_table_and_font_scaling_screenshots() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let parent_dir = tmp.path().join("workspace-parent");
    let leaf_dir = parent_dir.join("active-leaf");
    std::fs::create_dir_all(&leaf_dir).expect("create dirs");
    std::fs::write(leaf_dir.join("README.md"), b"# Project\nOverview").expect("write");
    std::fs::write(
        leaf_dir
            .join("very_long_component_filename_that_must_elide_cleanly_in_narrow_columns.rs"),
        b"fn main() {}",
    )
    .expect("write");
    std::fs::create_dir_all(leaf_dir.join("subfolder")).expect("mkdir");
    std::fs::write(parent_dir.join("root-file.toml"), b"[package]").expect("write");

    let mut h = PlexiUiHarness::new_sized_ppp(440.0, 520.0, 2.0);
    let leaf_clone = leaf_dir.clone();
    h.with_app_mut(|app| {
        app.sidebar_visible = false;
        app.open_builtin_view_pane(
            Box::new(crate::file_browser::FileBrowserView::new(
                leaf_clone.clone(),
            )),
            PluginPermissions::builtin(),
            leaf_clone,
            None,
            None,
            None,
        );
    });
    h.run_steps(3);
    assert!(
        !h.host_has_label("compact") && !h.host_has_label("details"),
        "stray mode label must not be rendered in toolbar"
    );
    h.save_screenshot(&evidence_png!("narrow"))
        .expect("render narrow file_browser");

    // Resize to wide width and verify unified table renders with breadcrumbs.
    h.harness().set_size(egui::vec2(880.0, 520.0));
    h.run_steps(3);
    h.save_screenshot(&evidence_png!("wide"))
        .expect("render wide file_browser");

    // Scale font size up via adjust_focused_pane_font_size and verify render.
    h.with_app_mut(|app| {
        app.adjust_focused_pane_font_size(4.0);
    });
    h.run_steps(3);
    h.save_screenshot(&evidence_png!("scaled"))
        .expect("render scaled file_browser");

    // Click ancestor breadcrumb "workspace-parent" (converting from kittest's
    // 2x physical-pixel rect to points) and verify navigation to parent_dir.
    let crumb_pos = h.harness().get_by_label("workspace-parent").rect().center() / 2.0;
    h.harness()
        .input_mut()
        .events
        .push(egui::Event::PointerMoved(crumb_pos));
    h.harness()
        .input_mut()
        .events
        .push(egui::Event::PointerButton {
            pos: crumb_pos,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        });
    h.harness()
        .input_mut()
        .events
        .push(egui::Event::PointerButton {
            pos: crumb_pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        });
    h.run_steps(3);
    h.with_app_mut(|app| {
        let fb = app.windows[app.active_window]
            .panes
            .values_mut()
            .find_map(|p| {
                let ViewRuntime::Builtin(b) = &mut p.as_view_mut()?.runtime else {
                    return None;
                };
                b.as_any_mut()
                    .downcast_mut::<crate::file_browser::FileBrowserView>()
            })
            .expect("FileBrowserView pane");
        assert_eq!(
            fb.cwd, parent_dir,
            "clicking breadcrumb must navigate to ancestor"
        );
    });
}

#[test]
fn screenshot_file_browser_deep_search_narrow_and_wide() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path().join("workspace-root");
    let src_dir = root.join("src").join("search");
    let docs_dir = root.join("docs");
    std::fs::create_dir_all(&src_dir).expect("mkdir src/search");
    std::fs::create_dir_all(&docs_dir).expect("mkdir docs");
    std::fs::write(
        src_dir.join("search_session.rs"),
        b"pub struct SearchSession;\n",
    )
    .expect("write search_session.rs");
    std::fs::write(
        src_dir.join("search_bar_widget.rs"),
        b"pub struct SearchBar;\n",
    )
    .expect("write search_bar_widget.rs");
    std::fs::write(
        docs_dir.join("architecture.md"),
        b"# Architecture\nUnified upstream search primitive for path and content discovery.\n",
    )
    .expect("write architecture.md");

    let mut h = PlexiUiHarness::new_sized_ppp(440.0, 480.0, 2.0);
    let root_clone = root.clone();
    h.with_app_mut(|app| {
        app.sidebar_visible = false;
        app.open_builtin_view_pane(
            Box::new(crate::file_browser::FileBrowserView::new(
                root_clone.clone(),
            )),
            PluginPermissions::builtin(),
            root_clone,
            None,
            None,
            None,
        );
    });
    h.run_steps(2);

    // Press Cmd+F to open the SearchBar and focus its TextInput, then type "search".
    h.harness().input_mut().events.push(egui::Event::Key {
        key: egui::Key::F,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers {
            command: true,
            ..Default::default()
        },
    });
    h.run_steps(2);
    h.harness()
        .input_mut()
        .events
        .push(egui::Event::Text("search".to_string()));
    h.run_steps(2);

    let deadline = std::time::Instant::now()
        + crate::testing::load_aware_timeout(std::time::Duration::from_secs(5));
    let mut found_all = false;
    while std::time::Instant::now() < deadline {
        h.step();
        let done = h.with_app_mut(|app| {
            app.windows[app.active_window]
                .panes
                .values_mut()
                .find_map(|p| {
                    let ViewRuntime::Builtin(b) = &mut p.as_view_mut()?.runtime else {
                        return None;
                    };
                    b.as_any_mut()
                        .downcast_mut::<crate::file_browser::FileBrowserView>()
                })
                .is_some_and(|fb| fb.search_result_count() >= 3)
        });
        if done {
            found_all = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(15));
    }
    assert!(
        found_all,
        "Cmd+F + typing 'search' must enter search mode and populate deep path + content hits"
    );

    h.run_steps(2);
    h.save_screenshot(&evidence_png!("narrow"))
        .expect("render narrow deep search");

    h.harness().set_size(egui::vec2(880.0, 480.0));
    h.run_steps(2);
    h.save_screenshot(&evidence_png!("wide"))
        .expect("render wide deep search");
}

#[test]
fn file_browser_search_discoverable_and_interactive_in_harness() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path().join("workspace-root");
    std::fs::create_dir_all(&root).expect("mkdir root");
    std::fs::write(root.join("alpha.rs"), b"fn alpha() {}\n").expect("write alpha");
    std::fs::write(root.join("beta.md"), b"# Beta\n").expect("write beta");

    let mut h = PlexiUiHarness::new_sized_ppp(640.0, 480.0, 2.0);
    let root_clone = root.clone();
    h.with_app_mut(|app| {
        app.sidebar_visible = false;
        app.open_builtin_view_pane(
            Box::new(crate::file_browser::FileBrowserView::new(
                root_clone.clone(),
            )),
            PluginPermissions::builtin(),
            root_clone,
            None,
            None,
            None,
        );
    });
    h.run_steps(2);

    let pane_id = h
        .with_app(|app| app.windows[app.active_window].panes.keys().copied().next())
        .expect("opened pane id");

    // 1. SearchBar affordance is visible on initial open
    assert!(
        h.pane_has_label(pane_id, "Search files and contents…"),
        "SearchBar affordance must be visible on initial File Explorer open"
    );

    // 2. Cmd+F enters search mode and requests text focus
    h.harness().input_mut().events.push(egui::Event::Key {
        key: egui::Key::F,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::COMMAND,
    });
    h.run_steps(2);

    let in_search_cmdf = h.with_app_mut(|app| {
        app.windows[app.active_window]
            .panes
            .values_mut()
            .find_map(|p| {
                let ViewRuntime::Builtin(b) = &mut p.as_view_mut()?.runtime else {
                    return None;
                };
                b.as_any_mut()
                    .downcast_mut::<crate::file_browser::FileBrowserView>()
            })
            .map(|fb| fb.is_in_search())
    });
    assert_eq!(in_search_cmdf, Some(true), "Cmd+F must enter search mode");

    // Escape exits search mode back to normal browsing
    h.harness().input_mut().events.push(egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::default(),
    });
    h.run_steps(2);

    let exited_search = h.with_app_mut(|app| {
        app.windows[app.active_window]
            .panes
            .values_mut()
            .find_map(|p| {
                let ViewRuntime::Builtin(b) = &mut p.as_view_mut()?.runtime else {
                    return None;
                };
                b.as_any_mut()
                    .downcast_mut::<crate::file_browser::FileBrowserView>()
            })
            .map(|fb| fb.is_in_search())
    });
    assert_eq!(exited_search, Some(false), "Escape must exit search mode");

    // 3. Pointer click on SearchBar enters search mode
    let search_pos = h
        .harness()
        .get_by_label("Search files and contents…")
        .rect()
        .center()
        / 2.0;
    h.harness().input_mut().events.extend([
        egui::Event::PointerMoved(search_pos),
        egui::Event::PointerButton {
            pos: search_pos,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        },
        egui::Event::PointerButton {
            pos: search_pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        },
    ]);
    h.run_steps(2);

    let in_search_click = h.with_app_mut(|app| {
        app.windows[app.active_window]
            .panes
            .values_mut()
            .find_map(|p| {
                let ViewRuntime::Builtin(b) = &mut p.as_view_mut()?.runtime else {
                    return None;
                };
                b.as_any_mut()
                    .downcast_mut::<crate::file_browser::FileBrowserView>()
            })
            .map(|fb| fb.is_in_search())
    });
    assert_eq!(
        in_search_click,
        Some(true),
        "clicking SearchBar must enter search mode"
    );

    // Escape exits search mode back to normal browsing
    h.harness().input_mut().events.push(egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::default(),
    });
    h.run_steps(2);

    // 4. Typing ordinary text ("alp") enters search and filters to alpha.rs
    h.harness()
        .input_mut()
        .events
        .push(egui::Event::Text("alp".to_string()));
    h.run_steps(2);

    let filtered_count = h.with_app_mut(|app| {
        app.windows[app.active_window]
            .panes
            .values_mut()
            .find_map(|p| {
                let ViewRuntime::Builtin(b) = &mut p.as_view_mut()?.runtime else {
                    return None;
                };
                b.as_any_mut()
                    .downcast_mut::<crate::file_browser::FileBrowserView>()
            })
            .map(|fb| (fb.is_in_search(), fb.search_result_count()))
    });
    assert_eq!(
        filtered_count,
        Some((true, 1)),
        "typing plain text must enter search mode and filter entries"
    );

    // 5. Activating result (Enter) opens alpha.rs and exits search mode
    h.harness().input_mut().events.push(egui::Event::Key {
        key: egui::Key::Enter,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::default(),
    });
    h.run_steps(2);

    let search_state = h.with_app_mut(|app| {
        app.windows[app.active_window]
            .panes
            .values_mut()
            .find_map(|p| {
                let ViewRuntime::Builtin(b) = &mut p.as_view_mut()?.runtime else {
                    return None;
                };
                b.as_any_mut()
                    .downcast_mut::<crate::file_browser::FileBrowserView>()
            })
            .map(|fb| (fb.is_in_search(), fb.opened_files.len()))
    });
    assert_eq!(
        search_state,
        Some((false, 1)),
        "activating search hit must open the file and exit search mode back to browsing"
    );
}

#[test]
fn screenshot_file_browser_status_bar_shortcuts_wide_and_narrow() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path().join("workspace-root");
    std::fs::create_dir_all(root.join("src")).expect("mkdir src");
    std::fs::create_dir_all(root.join("docs")).expect("mkdir docs");
    std::fs::write(root.join("Cargo.toml"), b"[package]\nname = \"plexi\"\n")
        .expect("write Cargo.toml");
    std::fs::write(root.join("README.md"), b"# Plexi\n").expect("write README.md");

    let mut h = PlexiUiHarness::new_sized_ppp(1100.0, 720.0, 2.0);
    let root_clone = root.clone();
    h.with_app_mut(|app| {
        app.sidebar_visible = false;
        app.open_builtin_view_pane(
            Box::new(crate::file_browser::FileBrowserView::new(
                root_clone.clone(),
            )),
            PluginPermissions::builtin(),
            root_clone,
            None,
            None,
            None,
        );
    });
    h.run_steps(3);
    h.with_app_mut(|app| {
        let fb = app.windows[app.active_window]
            .panes
            .values_mut()
            .find_map(|p| {
                let ViewRuntime::Builtin(b) = &mut p.as_view_mut()?.runtime else {
                    return None;
                };
                b.as_any_mut()
                    .downcast_mut::<crate::file_browser::FileBrowserView>()
            })
            .expect("FileBrowserView pane");
        let layout = fb
            .last_status_bar_layout
            .as_ref()
            .expect("wide status bar layout");
        assert_eq!(
            layout.label_rects.len(),
            7,
            "wide 1100x720 pane must fit all 7 status bar hint groups"
        );
        for label_rect in &layout.label_rects {
            assert!(
                (label_rect.max.y - layout.item_count_rect.max.y).abs() <= 0.5,
                "wide hint label baseline must match item count baseline"
            );
        }
        for chip_rect in &layout.chip_rects {
            assert!(
                (chip_rect.center().y - layout.item_count_rect.center().y).abs() <= 0.5,
                "wide chip vertical center must match item count vertical center"
            );
        }
        let last_label = layout.label_rects.last().unwrap();
        assert!(
            (layout.row_rect.max.x - last_label.max.x).abs() <= 2.0,
            "wide rightmost hint label must sit flush with status bar right edge"
        );
    });
    h.save_screenshot(&evidence_png!("_wide_1100x720"))
        .expect("render wide 1100x720 status bar");

    h.harness().set_size(egui::vec2(760.0, 520.0));
    h.run_steps(3);
    h.with_app_mut(|app| {
        let fb = app.windows[app.active_window]
            .panes
            .values_mut()
            .find_map(|p| {
                let ViewRuntime::Builtin(b) = &mut p.as_view_mut()?.runtime else {
                    return None;
                };
                b.as_any_mut()
                    .downcast_mut::<crate::file_browser::FileBrowserView>()
            })
            .expect("FileBrowserView pane");
        let layout = fb
            .last_status_bar_layout
            .as_ref()
            .expect("medium 760 status bar layout");
        assert!(
            !layout.label_rects.is_empty(),
            "medium 760x520 pane must fit status bar hints"
        );
        let last_label = layout.label_rects.last().unwrap();
        assert!(
            (layout.row_rect.max.x - last_label.max.x).abs() <= 2.0,
            "medium 760 rightmost hint label must sit flush with status bar right edge"
        );
    });
    h.save_screenshot(&evidence_png!("_medium_760x520"))
        .expect("render medium 760x520 status bar");

    h.harness().set_size(egui::vec2(640.0, 520.0));
    h.run_steps(3);
    h.with_app_mut(|app| {
        let fb = app.windows[app.active_window]
            .panes
            .values_mut()
            .find_map(|p| {
                let ViewRuntime::Builtin(b) = &mut p.as_view_mut()?.runtime else {
                    return None;
                };
                b.as_any_mut()
                    .downcast_mut::<crate::file_browser::FileBrowserView>()
            })
            .expect("FileBrowserView pane");
        let layout = fb
            .last_status_bar_layout
            .as_ref()
            .expect("narrow status bar layout");
        assert!(
            !layout.label_rects.is_empty() && layout.label_rects.len() < 7,
            "narrow 640x520 pane must drop leading low-priority hints to fit cleanly"
        );
        for label_rect in &layout.label_rects {
            assert!(
                (label_rect.max.y - layout.item_count_rect.max.y).abs() <= 0.5,
                "narrow hint label baseline must match item count baseline"
            );
        }
        for chip_rect in &layout.chip_rects {
            assert!(
                (chip_rect.center().y - layout.item_count_rect.center().y).abs() <= 0.5,
                "narrow chip vertical center must match item count vertical center"
            );
        }
        let first_chip = layout.chip_rects.first().unwrap();
        let last_label = layout.label_rects.last().unwrap();
        assert!(
            first_chip.min.x >= layout.item_count_rect.max.x + crate::ui::style::SPACE_MD,
            "narrow first chip must not collide with item count label"
        );
        assert!(
            (layout.row_rect.max.x - last_label.max.x).abs() <= 2.0,
            "narrow rightmost hint label must sit flush with status bar right edge"
        );
    });
    h.save_screenshot(&evidence_png!("_narrow_640x520"))
        .expect("render narrow 640x520 status bar");
}

#[test]
fn screenshot_file_browser_action_palette_and_intent_preview_chip() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path().join("workspace-root");
    std::fs::create_dir_all(root.join("src")).expect("mkdir src");
    std::fs::create_dir_all(root.join("docs")).expect("mkdir docs");
    std::fs::write(root.join("Cargo.toml"), b"[package]\nname = \"plexi\"\n")
        .expect("write Cargo.toml");
    std::fs::write(root.join("main.rs"), b"fn main() { // todo: wire up }\n")
        .expect("write main.rs");

    let mut h = PlexiUiHarness::new_sized_ppp(880.0, 560.0, 2.0);
    let root_clone = root.clone();
    h.with_app_mut(|app| {
        app.sidebar_visible = false;
        app.open_builtin_view_pane(
            Box::new(crate::file_browser::FileBrowserView::new(
                root_clone.clone(),
            )),
            PluginPermissions::builtin(),
            root_clone,
            None,
            None,
            None,
        );
    });
    h.run_steps(3);

    let tap_key = |h: &mut PlexiUiHarness, key: egui::Key, modifiers: egui::Modifiers| {
        h.harness().input_mut().events.push(egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        });
        h.harness().input_mut().events.push(egui::Event::Key {
            key,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers,
        });
    };

    // Press `/` to open the inline action palette pre-filled with `/`.
    tap_key(&mut h, egui::Key::Slash, egui::Modifiers::NONE);
    h.run_steps(2);
    assert!(
        h.host_has_label(
            "Type a command or use \u{2191}\u{2193} to choose an action \u{2014} Enter to run"
        ),
        "bare `/` palette open must show neutral command-mode hint chip"
    );
    h.save_screenshot(&evidence_png!("_slash_open"))
        .expect("render file_browser action palette on `/`");

    // Type `sort by size` so `/sort by size` candidates and preview chip render.
    h.harness()
        .input_mut()
        .events
        .push(egui::Event::Text("sort by size".to_string()));
    h.run_steps(2);
    h.save_screenshot(&evidence_png!("_sort_by_size"))
        .expect("render file_browser action palette on `/sort by size`");

    // Escape and re-open `/` to type `new file notes.md` so the structured INTENT preview chip resolves `CreateFile`.
    tap_key(&mut h, egui::Key::Escape, egui::Modifiers::NONE);
    h.run_steps(2);
    tap_key(&mut h, egui::Key::Slash, egui::Modifiers::NONE);
    h.run_steps(2);
    h.harness()
        .input_mut()
        .events
        .push(egui::Event::Text("new file notes.md".to_string()));
    h.run_steps(2);
    assert!(
        h.host_has_label("Create file: notes.md \u{2014} open in editor [Enter]"),
        "structured intent preview chip must render resolved CreateFile preview"
    );
    h.save_screenshot(&evidence_png!("_resolved_intent"))
        .expect("render file_browser action palette with resolved intent preview chip");

    // Escape palette, open `Cmd+F` search, and type `todo`.
    tap_key(&mut h, egui::Key::Escape, egui::Modifiers::NONE);
    h.run_steps(2);
    tap_key(
        &mut h,
        egui::Key::F,
        egui::Modifiers {
            command: true,
            ..Default::default()
        },
    );
    h.run_steps(2);
    h.harness()
        .input_mut()
        .events
        .push(egui::Event::Text("todo".to_string()));
    h.run_steps(2);
    h.save_screenshot(&evidence_png!("_search_todo"))
        .expect("render file_browser search with `todo`");
}

#[test]
fn screenshot_file_browser_new_file_prompt() {
    let mut h = PlexiUiHarness::new_sized(880.0, 560.0);
    let root = h.workspace_root().to_path_buf();
    std::fs::write(root.join("plan.md"), "# Plan\n").expect("seed plan.md");
    h.open_file_browser(root);
    h.run_steps(3);

    h.harness().key_press(egui::Key::Slash);
    h.run_steps(2);
    h.harness()
        .input_mut()
        .events
        .push(egui::Event::Text("new file".to_string()));
    h.run_steps(2);
    h.harness().key_press(egui::Key::Enter);
    h.run_steps(2);

    assert!(h.host_has_label("Create file: <name> — type filename and press Enter"));
    h.save_screenshot(&evidence_png!())
        .expect("render file_browser filename prompt");
}

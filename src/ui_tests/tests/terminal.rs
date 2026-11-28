use super::*;

/// The scene DSL cannot seed PTY output or drag inside a terminal widget.
/// Exercise real pointer events, selection commands and highlight painting.
#[test]
#[ignore = "requires-pty"]
fn terminal_selection_tracks_pointer_at_fractional_scale() {
    for scale in [1.0, 1.1, 1.3] {
        let mut h = PlexiUiHarness::new_sized_ppp(1100.0, 500.0, 2.0);
        h.with_app_mut(|app| app.ui_scale = scale);
        h.run_steps(3);
        let id = add_focused_pane(&mut h);
        let fixture = "0123456789".repeat(10);
        h.with_app_mut(|app| {
            app.sidebar_visible = false;
            let terminal = TerminalPane::fixture(
                id,
                app.ctx.clone(),
                app.pty_event_tx.clone(),
                egui_term::BackendSettings {
                    shell: "/bin/sh".into(),
                    args: vec![
                        "-c".into(),
                        format!(
                            "read -r line; printf '\\033[2J\\033[H{fixture}'; read -r line"
                        ),
                    ],
                    working_directory: Some(app.router.active().root.clone()),
                    ..Default::default()
                },
                16.0,
            )
            .expect("start selection fixture");
            let win = app.active_window;
            app.windows[win].replace_pane_for_test(id, Pane::Terminal(Box::new(terminal)));
        });
        h.run_steps(3);
        h.with_app_mut(|app| {
            app.windows[app.active_window]
                .panes
                .get_mut(&id)
                .unwrap()
                .as_terminal_mut()
                .unwrap()
                .backend
                .process_command(egui_term::BackendCommand::Write(b"ready\n".to_vec()));
        });
        let deadline = std::time::Instant::now()
            + crate::testing::load_aware_timeout(Duration::from_secs(10));
        loop {
            h.run_steps(2);
            let ready = h.with_app_mut(|app| {
                app.windows[app.active_window]
                    .panes
                    .get_mut(&id)
                    .unwrap()
                    .as_terminal_mut()
                    .unwrap()
                    .backend
                    .capture_lines(20)
                    .iter()
                    .any(|line| line.contains("01234567890123456789"))
            });
            if ready {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "selection fixture did not produce text"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        h.run_steps(3);
        let rect = h
            .harness()
            .ctx
            .read_response(egui_term::terminal_widget_id(id))
            .expect("rendered terminal widget")
            .rect;
        let (width, height) = h.with_app_mut(|app| {
            let size = app.windows[app.active_window]
                .panes
                .get_mut(&id)
                .unwrap()
                .as_terminal_mut()
                .unwrap()
                .backend
                .last_content()
                .terminal_size;
            (size.cell_width as f32, size.cell_height as f32)
        });
        let origin = rect.min
            + egui::vec2(
                crate::ui::style::PANE_CONTENT_INSET,
                crate::ui::style::PANE_CONTENT_INSET,
            );
        let at = |cell: f32| origin + egui::vec2(cell * width, height * 0.5);
        h.harness().input_mut().events.extend([
            egui::Event::PointerMoved(at(0.25)),
            egui::Event::PointerButton {
                pos: at(0.25),
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        h.step();
        let columns = ((rect.width() - 2.0) / width).floor().min(80.0) as usize;
        let mut previous = 0;
        for step in 3..columns * 4 {
            let cell = step as f32 / 4.0;
            h.harness()
                .input_mut()
                .events
                .push(egui::Event::PointerMoved(at(cell)));
            h.step();
            let selected = h.with_app_mut(|app| {
                app.windows[app.active_window]
                    .panes
                    .get_mut(&id)
                    .unwrap()
                    .as_terminal_mut()
                    .unwrap()
                    .backend
                    .selectable_content()
            });
            assert!(selected.len() >= previous,
                "rightward drag moved selection backward: scale={scale}, cell={cell}, {previous}->{}", selected.len());
            if step % 4 == 3 {
                let expected = step / 4 + 1;
                assert_eq!(selected, fixture[..expected], "scale={scale}, cell={cell}");
            }
            previous = selected.len();
        }
        h.save_screenshot(&evidence_png!(&format!("_scale_{scale}")))
            .expect("render selected terminal text");
        // Reverse the same drag across the whole line before releasing.
        for col in (1..columns).rev() {
            h.harness()
                .input_mut()
                .events
                .push(egui::Event::PointerMoved(at(col as f32 + 0.75)));
            h.step();
            let selected = h.with_app_mut(|app| {
                app.windows[app.active_window]
                    .panes
                    .get_mut(&id)
                    .unwrap()
                    .as_terminal_mut()
                    .unwrap()
                    .backend
                    .selectable_content()
            });
            assert_eq!(
                selected,
                fixture[..col + 1],
                "reverse scale={scale}, col={col}"
            );
        }
        h.harness()
            .input_mut()
            .events
            .push(egui::Event::PointerButton {
                pos: at(1.75),
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            });
        h.step();
    }
}

// Terminal minimum contrast (stint 1188): dark-theme 24-bit colors printed
// by a program, shown on Dawn and Day (stint 1186) with
// the default minimum and with the adjustment off.
#[test]
fn screenshot_terminal_minimum_contrast() {
    const FIXTURE: &str = "printf '\\033[38;2;153;153;153mgrey 153 comment text, dark-theme dim\\033[0m\\n'; \
printf '\\033[38;2;119;119;119mgrey 119 secondary text\\033[0m\\n'; \
printf '\\033[38;2;177;185;249minline_code() pale blue\\033[0m\\n'; \
printf '\\033[38;2;230;219;116mpale yellow string\\033[0m  \\033[38;2;166;226;46mlime green keyword\\033[0m\\n'; \
printf '\\033[38;2;255;121;198mpink function\\033[0m  \\033[38;2;139;233;253mcyan type\\033[0m\\n'; \
printf '\\033[2m\\033[38;2;200;200;200mfaint SGR2 grey\\033[0m  \\033[2m\\033[38;2;177;185;249mfaint pale blue\\033[0m  \\033[7mreverse video\\033[0m\\n'; \
printf 'hidden:[\\033[8msecret\\033[0m] must show blank\\n'; \
printf 'ANSI: \\033[31mred\\033[32m green\\033[33m yellow\\033[34m blue\\033[35m magenta\\033[36m cyan\\033[0m\\n'; \
printf 'FIXTURE-DONE\\n'; sleep 300";
    for (name, theme_toml) in [
        ("dawn", "preset = \"plexi-dawn\"\n"),
        ("day", "preset = \"plexi-day\"\n"),
    ] {
        for (label, terminal_toml) in [
            ("default", ""),
            ("off", "[terminal]\nminimum_contrast = 1.0\n"),
        ] {
            let mut h = PlexiUiHarness::new_sized_ppp(900.0, 520.0, 1.0);
            h.step();
            std::fs::write(
                crate::config::config_path(),
                format!("pane_gap = 0.0\n{terminal_toml}[theme]\n{theme_toml}"),
            )
            .unwrap();
            h.with_app_mut(|app| app.reload_config());
            let expected = if label == "off" { 1.0 } else { 3.0 };
            h.with_app(|app| {
                assert_eq!(
                    app.theme.minimum_contrast(),
                    expected,
                    "reload must apply the setting live"
                )
            });
            let pane = h.with_app_mut(|app| {
                app.spawn_agent_worker_pane(
                    "fixture", "pi", FIXTURE, "Contrast", None, false, None, None, None, None,
                    false, None,
                )
                .unwrap()
                .0
            });
            let deadline =
                Instant::now() + crate::testing::load_aware_timeout(Duration::from_secs(20));
            loop {
                h.run_steps(2);
                let ready = h.with_app_mut(|app| {
                    let (win, _) = app.find_pane_in_any_window(pane).unwrap();
                    app.windows[win]
                        .panes
                        .get_mut(&pane)
                        .unwrap()
                        .as_terminal_mut()
                        .unwrap()
                        .backend
                        .capture_lines(40)
                        .iter()
                        .any(|l| l.contains("FIXTURE-DONE"))
                });
                if ready {
                    break;
                }
                assert!(Instant::now() < deadline, "contrast fixture timed out");
                std::thread::sleep(Duration::from_millis(20));
            }
            h.focus_pane(pane).unwrap();
            h.run_steps(3);
            h.save_screenshot(&evidence_png!(&format!("_{name}_{label}")))
                .unwrap();
        }
    }
}

/// Scenes cannot seed terminal PTY output. Exercise the real terminal
/// widget with the same ANSI fixture used by installed-host validation.
#[test]
fn screenshot_terminal_bold_and_shades() {
    for ppp in [1.0, 2.0] {
        let mut h = PlexiUiHarness::new_sized_ppp(1000.0, 900.0, ppp);
        h.step();
        let id = add_focused_pane(&mut h);
        let workspace = h.workspace_root().to_path_buf();
        h.with_app_mut(|app| {
            let terminal = crate::host::pane::TerminalPane::fixture(
                id,
                app.ctx.clone(),
                app.pty_event_tx.clone(),
                egui_term::BackendSettings {
                    shell: "/bin/sh".into(),
                    args: vec![concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/tests/fixtures/terminal-rendering.sh"
                    )
                    .into()],
                    working_directory: Some(workspace),
                    ..Default::default()
                },
                18.0,
            )
            .expect("start terminal fixture");
            app.windows[app.active_window].take_pane(id);
            app.windows[app.active_window]
                .panes
                .insert(id, Pane::Terminal(Box::new(terminal)))
                .unwrap();
            app.sidebar_visible = false;
        });
        let deadline = std::time::Instant::now()
            + crate::testing::load_aware_timeout(Duration::from_secs(10));
        loop {
            h.run_steps(2);
            let ready = h.with_app_mut(|app| {
                app.windows[app.active_window]
                    .panes
                    .get_mut(&id)
                    .and_then(|pane| pane.as_terminal_mut())
                    .expect("fixture terminal")
                    .backend
                    .capture_lines(50)
                    .iter()
                    .any(|line| line.contains("RENDER_FIXTURE_READY"))
            });
            if ready {
                // `capture_lines` reads the live PTY grid directly, which
                // can advance between `run_steps(2)` and `capture_lines`.
                // Pump a settling pass so `TerminalView::sync()` and egui's
                // `FullOutput` incorporate the completed fixture before
                // `h.render()` rasterizes the frame (stint 0820).
                h.run_steps(2);
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "terminal fixture timed out"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        let path = evidence_png!(&format!("_ppp{ppp}"));
        let image = h.render().expect("render terminal fixture");
        image.save(&path).expect("save terminal screenshot");
        assert!(
            image
                .pixels()
                .filter(|p| p.0 == [117, 223, 255, 255])
                .count()
                > 100,
            "bold RGB slider must retain its requested foreground; screenshot: {path}"
        );
    }
}

/// Reproduce a Neovim-style clock icon against adjacent colored cells.
#[test]
fn screenshot_terminal_nvim_clock_narrow() {
    let mut h = PlexiUiHarness::new_sized_ppp(480.0, 240.0, 2.0);
    h.step();
    let id = add_focused_pane(&mut h);
    let workspace = h.workspace_root().to_path_buf();
    h.with_app_mut(|app| {
        let terminal = crate::host::pane::TerminalPane::fixture(
            id,
            app.ctx.clone(),
            app.pty_event_tx.clone(),
            egui_term::BackendSettings {
                shell: "/bin/sh".into(),
                args: vec![concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/tests/fixtures/terminal-clock.sh"
                )
                .into()],
                working_directory: Some(workspace),
                ..Default::default()
            },
            18.0,
        )
        .expect("start clock fixture");
        app.windows[app.active_window].take_pane(id);
        app.windows[app.active_window]
            .panes
            .insert(id, Pane::Terminal(Box::new(terminal)))
            .unwrap();
        app.sidebar_visible = false;
    });
    let deadline = Instant::now() + crate::testing::load_aware_timeout(Duration::from_secs(10));
    loop {
        h.run_steps(2);
        let ready = h.with_app_mut(|app| {
            app.windows[app.active_window]
                .panes
                .get_mut(&id)
                .and_then(|pane| pane.as_terminal_mut())
                .expect("fixture terminal")
                .backend
                .capture_lines(50)
                .iter()
                .any(|line| line.contains("CLOCK_READY"))
        });
        if ready {
            h.run_steps(2);
            break;
        }
        assert!(Instant::now() < deadline, "clock fixture timed out");
        std::thread::sleep(Duration::from_millis(20));
    }
    let image = h.render().expect("render clock fixture");
    image.save(&evidence_png!()).expect("save clock screenshot");
    // At this fixed viewport and scale, the clock's right half crosses into
    // the next colored cell. A later background used to erase it entirely.
    let right_half = (189..224)
        .flat_map(|y| (78..85).map(move |x| (x, y)))
        .filter(|&(x, y)| image.get_pixel(x, y).0[..3].iter().all(|&c| c < 90))
        .count();
    assert!(
        right_half > 30,
        "clock icon right half must remain visible; found {right_half} pixels"
    );
}

/// The inverted cursor glyph must not spill into a differently colored cell.
#[test]
fn screenshot_terminal_cursor_clock_stays_in_cell() {
    let mut h = PlexiUiHarness::new_sized_ppp(480.0, 240.0, 2.0);
    h.step();
    let id = add_focused_pane(&mut h);
    let workspace = h.workspace_root().to_path_buf();
    h.with_app_mut(|app| {
        let terminal = crate::host::pane::TerminalPane::fixture(
            id,
            app.ctx.clone(),
            app.pty_event_tx.clone(),
            egui_term::BackendSettings {
                shell: "/bin/sh".into(),
                args: vec![
                    concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/tests/fixtures/terminal-clock.sh"
                    )
                    .into(),
                    "cursor".into(),
                ],
                working_directory: Some(workspace),
                ..Default::default()
            },
            18.0,
        )
        .expect("start clock fixture");
        app.windows[app.active_window].take_pane(id);
        app.windows[app.active_window]
            .panes
            .insert(id, Pane::Terminal(Box::new(terminal)))
            .unwrap();
        app.sidebar_visible = false;
    });
    let deadline = Instant::now() + crate::testing::load_aware_timeout(Duration::from_secs(10));
    loop {
        h.run_steps(2);
        let ready = h.with_app_mut(|app| {
            let backend = &mut app.windows[app.active_window]
                .panes
                .get_mut(&id)
                .and_then(|pane| pane.as_terminal_mut())
                .expect("fixture terminal")
                .backend;
            let content = backend.last_content();
            content.cursor_visible
                && content.grid.cursor.point.line.0 == 0
                && content.grid.cursor.point.column.0 == 2
                && backend
                    .capture_lines(50)
                    .iter()
                    .any(|line| line.contains("CLOCK_READY"))
        });
        if ready {
            break;
        }
        assert!(Instant::now() < deadline, "clock fixture cursor not ready");
        std::thread::sleep(Duration::from_millis(20));
    }
    let widget_id = egui_term::terminal_widget_id(id);
    h.harness().ctx.data_mut(|data| {
        let mut state = data
            .get_temp::<egui_term::TerminalViewState>(widget_id)
            .unwrap_or_default();
        state.set_cursor_blink_enabled(false);
        data.insert_temp(widget_id, state);
    });
    h.step();
    let rect = h
        .harness()
        .ctx
        .read_response(widget_id)
        .expect("terminal widget")
        .rect;
    let size = h.with_app_mut(|app| {
        app.windows[app.active_window]
            .panes
            .get_mut(&id)
            .and_then(|pane| pane.as_terminal_mut())
            .expect("fixture terminal")
            .backend
            .last_content()
            .terminal_size
    });
    let scale = 2.0;
    let x = ((rect.min.x + crate::ui::style::PANE_CONTENT_INSET + 2.0 * size.cell_width as f32)
        * scale)
        .round() as u32;
    let y = ((rect.min.y + crate::ui::style::PANE_CONTENT_INSET) * scale).round() as u32;
    let width = (size.cell_width as f32 * scale).round() as u32;
    let height = (size.cell_height as f32 * scale).round() as u32;
    let image = h.render().expect("render clock fixture");
    image.save(&evidence_png!()).expect("save clock screenshot");
    // Check the cursor block and inverted glyph, then the neighboring blue cell.
    assert_eq!(
        image.get_pixel(x + 3, y + 2).0[..3],
        [26, 29, 40],
        "cursor must be visible"
    );
    let glyph = (y..y + height)
        .flat_map(|py| (x..x + width).map(move |px| (px, py)))
        .filter(|&(px, py)| {
            let [r, g, b, _] = image.get_pixel(px, py).0;
            r > 160 && g > 100 && b < 150
        })
        .count();
    assert!(
        glyph > 30,
        "visible cursor must contain the inverted clock glyph"
    );
    let spill = (y..y + height)
        .flat_map(|py| (x + width..x + 2 * width).map(move |px| (px, py)))
        .filter(|&(px, py)| {
            let [r, g, b, _] = image.get_pixel(px, py).0;
            r > 160 && g > 100 && b < 150
        })
        .count();
    assert_eq!(spill, 0, "inverted clock glyph spilled into the next cell");
}

/// Stint 1184: `egui::Event::Paste` into a pane whose program enabled
/// bracketed paste (mode 2004) must reach the program as one framed paste,
/// not raw bytes. 16 KB spans many 1022-byte PTY reads, the regime where
/// unframed pastes were lost.
#[test]
#[ignore = "requires-pty"]
#[cfg(target_os = "macos")]
fn terminal_paste_is_bracketed_when_program_enabled_2004() {
    const CHILD: &str = r#"
import os, sys, tty, time, select
out = sys.argv[1]
tty.setraw(0)
sys.stdout.write("\x1b[?2004hPASTE_READY\r\n"); sys.stdout.flush()
log = open(out, "wb")
last = time.time()
while time.time() - last < 1.5:
    r, _, _ = select.select([0], [], [], 0.2)
    if r:
        log.write(os.read(0, 65536)); log.flush()
        last = time.time()
log.close()
open(out + ".done", "w").close()
"#;
    let dir = std::env::temp_dir().join(format!("plexi_paste_2004_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let out = dir.join("reads.bin");
    let mut h = PlexiUiHarness::new_sized(900.0, 600.0);
    h.step();
    let id = add_focused_pane(&mut h);
    let out_arg = out.to_string_lossy().to_string();
    h.with_app_mut(|app| {
        let terminal = crate::host::pane::TerminalPane::fixture(
            id,
            app.ctx.clone(),
            app.pty_event_tx.clone(),
            egui_term::BackendSettings {
                shell: "/usr/bin/python3".into(),
                args: vec!["-c".into(), CHILD.into(), out_arg],
                working_directory: Some(std::env::temp_dir()),
                ..Default::default()
            },
            18.0,
        )
        .expect("start paste fixture");
        terminal.backend.enable_input_tap();
        // The fixture replaces the pane the helper installed in that slot.
        let win = app.active_window;
        app.windows[win]
            .take_pane(id)
            .expect("the helper installed a pane");
        app.windows[win]
            .panes
            .insert(id, Pane::Terminal(Box::new(terminal)))
            .unwrap();
    });
    let deadline = Instant::now() + crate::testing::load_aware_timeout(Duration::from_secs(10));
    loop {
        h.run_steps(2);
        let ready = h.with_app_mut(|app| {
            app.windows[app.active_window]
                .panes
                .get_mut(&id)
                .unwrap()
                .as_terminal_mut()
                .unwrap()
                .backend
                .capture_lines(50)
                .iter()
                .any(|line| line.contains("PASTE_READY"))
        });
        if ready {
            break;
        }
        assert!(Instant::now() < deadline, "paste fixture timed out");
        std::thread::sleep(Duration::from_millis(20));
    }
    let text: String = (0..16_000)
        .map(|i| {
            if i % 40 == 39 {
                '\n'
            } else {
                (b'a' + (i % 26) as u8) as char
            }
        })
        .collect();
    h.harness()
        .input_mut()
        .events
        .push(egui::Event::Paste(text.clone()));
    h.step();
    let tap = h.with_app_mut(|app| {
        app.windows[app.active_window]
            .panes
            .get_mut(&id)
            .unwrap()
            .as_terminal_mut()
            .unwrap()
            .backend
            .take_input_tap()
    });
    let mut expected = b"\x1b[200~".to_vec();
    expected.extend_from_slice(text.as_bytes());
    expected.extend_from_slice(b"\x1b[201~");
    assert_eq!(tap, expected, "input tap must record one framed paste");
    let done = out.with_extension("bin.done");
    let deadline = Instant::now() + crate::testing::load_aware_timeout(Duration::from_secs(15));
    while !done.exists() {
        h.run_steps(2);
        assert!(Instant::now() < deadline, "paste fixture never finished");
        std::thread::sleep(Duration::from_millis(20));
    }
    let received = std::fs::read(&out).expect("fixture read log");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        received, expected,
        "program must receive the framed paste intact"
    );
}

/// Exercise egui input ownership and the real terminal widget/PTY writer.
/// Scene keys cannot express separate press, repeat and release events.
#[test]
fn terminal_kitty_press_repeat_release_reaches_pty_once() {
    let mut h = PlexiUiHarness::new_sized(900.0, 600.0);
    h.step();
    let id = add_focused_pane(&mut h);
    h.with_app_mut(|app| {
        let terminal = crate::host::pane::TerminalPane::fixture(
            id,
            app.ctx.clone(),
            app.pty_event_tx.clone(),
            egui_term::BackendSettings {
                shell: "/bin/sh".into(),
                args: vec![concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/tests/fixtures/kitty-keyboard-probe.sh"
                )
                .into()],
                working_directory: Some(std::env::temp_dir()),
                ..Default::default()
            },
            18.0,
        )
        .expect("start keyboard fixture");
        terminal.backend.enable_input_tap();
        app.windows[app.active_window].take_pane(id);
        app.windows[app.active_window]
            .panes
            .insert(id, Pane::Terminal(Box::new(terminal)))
            .unwrap();
    });
    let deadline = Instant::now() + crate::testing::load_aware_timeout(Duration::from_secs(10));
    loop {
        h.run_steps(2);
        let ready = h.with_app_mut(|app| {
            app.windows[app.active_window]
                .panes
                .get_mut(&id)
                .unwrap()
                .as_terminal_mut()
                .unwrap()
                .backend
                .capture_lines(50)
                .iter()
                .any(|line| line.contains("KITTY_READY"))
        });
        if ready {
            break;
        }
        assert!(Instant::now() < deadline, "keyboard fixture timed out");
        std::thread::sleep(Duration::from_millis(20));
    }
    for (pressed, repeat) in [(true, false), (true, true), (false, false)] {
        h.harness().input_mut().events.push(egui::Event::Key {
            key: egui::Key::J,
            physical_key: Some(egui::Key::J),
            pressed,
            repeat,
            modifiers: egui::Modifiers::NONE,
        });
        if pressed {
            h.harness()
                .input_mut()
                .events
                .push(egui::Event::Text("j".into()));
        }
        h.step();
    }
    let bytes = h.with_app_mut(|app| {
        app.windows[app.active_window]
            .panes
            .get_mut(&id)
            .unwrap()
            .as_terminal_mut()
            .unwrap()
            .backend
            .take_input_tap()
    });
    assert_eq!(
        String::from_utf8_lossy(&bytes),
        "\x1b[106;1:1u\x1b[106;1:2u\x1b[106;1:3u"
    );
    let deadline = Instant::now() + crate::testing::load_aware_timeout(Duration::from_secs(10));
    loop {
        h.run_steps(2);
        let received = h.with_app_mut(|app| {
            app.windows[app.active_window]
                .panes
                .get_mut(&id)
                .unwrap()
                .as_terminal_mut()
                .unwrap()
                .backend
                .capture_lines(50)
                .iter()
                .any(|line| line.contains("KEY_EVENTS_OK"))
        });
        if received {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "child did not receive the key event sequence"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    h.save_screenshot(&evidence_png!())
        .expect("render keyboard fixture");
}

/// Window drag-resize workload (stint 1224): four real PTYs with 10,000
/// coloured scrollback lines each, 120 frames of a changing window size.
/// Run alone with PLEXI_UI_PROFILE=1 and --nocapture; prints ranked spans.
#[test]
#[ignore = "requires-pty: resize profiling workload"]
fn profile_four_terminal_window_resize() {
    assert!(
        crate::platform::ui_profile::enabled(),
        "run with PLEXI_UI_PROFILE=1"
    );
    egui_term::set_span_hook(crate::platform::ui_profile::note_nanos);
    let mut h = PlexiUiHarness::new_sized(1600.0, 1000.0);
    h.step();
    let ids: Vec<PaneId> = h.with_app_mut(|app| {
        app.sidebar_visible = false;
        let root = app.router.active().root.clone();
        let ids: Vec<PaneId> = (0..4)
            .map(|_| {
                let id = app.host.alloc_pane_id();
                let terminal = crate::host::pane::TerminalPane::fixture(
                    id,
                    app.ctx.clone(),
                    app.pty_event_tx.clone(),
                    egui_term::BackendSettings {
                        shell: "/bin/sh".into(),
                        args: vec![
                            "-c".into(),
                            r#"i=0; while [ $i -lt 10000 ]; do printf '\033[38;2;70;75;90mresize-profile-%05d The quick brown fox jumps over the lazy dog.\r\n' "$i"; i=$((i+1)); done; printf '\033[0mRESIZE_SEED_DONE\r\n'; exec /bin/cat"#.into(),
                        ],
                        working_directory: Some(root.clone()),
                        ..Default::default()
                    },
                    14.0,
                )
                .expect("spawn resize fixture terminal");
                app.windows[app.active_window]
                    .panes
                    .insert(id, Pane::Terminal(Box::new(terminal)))
                    .expect("insert resize fixture pane");
                id
            })
            .collect();
        let context = app.router.active().context_id;
        app.set_context_columns(context, vec![vec![ids[0], ids[1]], vec![ids[2], ids[3]]]);
        ids
    });
    let deadline = Instant::now() + crate::testing::load_aware_timeout(Duration::from_secs(60));
    loop {
        h.step();
        let seeded = h.with_app_mut(|app| {
            ids.iter().all(|id| {
                app.windows[app.active_window]
                    .panes
                    .get_mut(id)
                    .and_then(|p| p.as_terminal_mut())
                    .is_some_and(|t| {
                        t.backend
                            .capture_lines(4)
                            .iter()
                            .any(|line| line.contains("RESIZE_SEED_DONE"))
                    })
            })
        });
        if seeded {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "PTY fixture did not finish seeding"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    h.run_steps(10);
    let commits = |h: &PlexiUiHarness| -> Vec<u64> {
        h.with_app(|app| {
            ids.iter()
                .map(|id| {
                    app.windows[app.active_window].panes[id]
                        .as_terminal()
                        .unwrap()
                        .backend
                        .resize_commits()
                })
                .collect()
        })
    };
    let before = commits(&h);
    let mut saves = 0usize;
    crate::platform::ui_profile::reset_measurement();
    for frame in 0..120u32 {
        h.harness().set_size(egui::vec2(
            1600.0 - (frame % 60) as f32 * 8.0,
            1000.0 - (frame % 60) as f32 * 3.0,
        ));
        let save_before = h.with_app(|app| app.last_workspace_save);
        h.step();
        saves += usize::from(h.with_app(|app| app.last_workspace_save != save_before));
    }
    let per_pane: Vec<u64> = commits(&h)
        .iter()
        .zip(&before)
        .map(|(a, b)| a - b)
        .collect();
    println!(
        "RESIZE_MEASUREMENT frames=120 panes=4 resize_commits={per_pane:?} workspace_saves={saves}"
    );
    crate::platform::ui_profile::print_measurement();
}

/// Visual review for the terminal reflow debounce (stint 0719). The
/// debounce holds a resize while the layout is still moving and commits
/// once the drag settles, so the pixel risk is a *stranded* commit: a
/// terminal left wrapping to its pre-drag width inside a narrower pane.
/// Three real terminals are dragged narrower one frame at a time, then
/// held — the assertion proves the grid followed, the PNG proves the
/// glyphs did too.
#[test]
fn screenshot_terminals_settle_after_window_drag() {
    const START_W: f32 = 1200.0;
    const HEIGHT: f32 = 700.0;
    let mut h = PlexiUiHarness::new_sized(START_W, HEIGHT);
    h.step();
    add_focused_pane(&mut h);
    h.step();
    for _ in 0..3 {
        h.with_app_mut(|app| app.split_focused(false, None, false, false, None));
        h.run_steps(2);
    }

    let terminals: Vec<plexi_host_model::model::PaneId> = h.with_app(|app| {
        app.windows[app.active_window]
            .panes
            .iter()
            .filter(|(_, pane)| matches!(pane, Pane::Terminal(_)))
            .map(|(id, _)| *id)
            .collect()
    });
    assert!(
        terminals.len() >= 2,
        "need several real terminals for the drag to be meaningful"
    );
    let visible_terminals = terminals.clone();
    assert!(
        !visible_terminals.is_empty(),
        "need a visible terminal for the drag to be meaningful"
    );
    let widest_before = h.with_app_mut(|app| max_terminal_cols(app, &visible_terminals));
    seed_wrapping_text(&mut h, &terminals);

    // Drag the window edge in: one frame per intermediate width, exactly
    // as winit reports a live resize. Terminals settle on elapsed time, so
    // the drag runs on the resize clock, not this machine's frame rate.
    h.with_app_mut(|app| app.resize_clock.set(Instant::now()));
    for step in 1..=6 {
        h.with_app_mut(|app| app.resize_clock.advance(Duration::from_millis(16)));
        h.harness()
            .set_size(egui::vec2(START_W - step as f32 * 60.0, HEIGHT));
        h.step();
    }
    h.with_app_mut(|app| app.resize_clock.advance(egui_term::RESIZE_SETTLE));
    h.run_steps(4);

    let widest_after = h.with_app_mut(|app| max_terminal_cols(app, &visible_terminals));
    assert!(
        widest_after < widest_before,
        "the trailing-edge commit was stranded: terminals still {widest_before} cols \
             wide after dragging the window narrower (now {widest_after})"
    );

    h.save_screenshot(&evidence_png!()).expect("render failed");
    println!("Screenshot saved to {}", evidence_png!());
}

/// Fill each terminal with a line long enough to wrap, so the screenshot
/// shows real reflowed content instead of an empty grid. Bounded: an
/// unresponsive shell degrades the visual evidence, never the assertions,
/// so this returns rather than failing the test.
fn seed_wrapping_text(h: &mut PlexiUiHarness, terminals: &[plexi_host_model::model::PaneId]) {
    let line = "the quick brown fox jumps over the lazy dog. ".repeat(6);
    h.with_app_mut(|app| {
        for id in terminals {
            if let Some(terminal) = app.windows[app.active_window]
                .panes
                .get_mut(id)
                .and_then(|pane| pane.as_terminal_mut())
            {
                terminal
                    .backend
                    .process_command(egui_term::BackendCommand::Write(
                        line.as_bytes().to_vec(),
                    ));
            }
        }
    });
    let deadline =
        std::time::Instant::now() + crate::testing::load_aware_timeout(Duration::from_secs(5));
    while std::time::Instant::now() < deadline {
        h.run_steps(2);
        let painted = h.with_app_mut(|app| {
            terminals.iter().all(|id| {
                app.windows[app.active_window]
                    .panes
                    .get_mut(id)
                    .and_then(|pane| pane.as_terminal_mut())
                    .is_some_and(|terminal| {
                        terminal
                            .backend
                            .capture_lines(4)
                            .iter()
                            .any(|row| row.contains("quick brown fox"))
                    })
            })
        });
        if painted {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Widest committed PTY grid among `terminals`, in columns.
fn max_terminal_cols(app: &mut PlexiApp, terminals: &[plexi_host_model::model::PaneId]) -> u16 {
    terminals
        .iter()
        .filter_map(|id| {
            app.windows[app.active_window]
                .panes
                .get_mut(id)
                .and_then(|pane| pane.as_terminal_mut())
                .map(|terminal| terminal.backend.committed_grid().0)
        })
        .max()
        .expect("terminals must still be open")
}

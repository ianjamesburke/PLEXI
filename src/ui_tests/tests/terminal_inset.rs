use super::*;

/// Real PTY terminal that fills its whole grid with wide glyphs and carries an
/// agent state, so both the focus ring and the state ring are drawn.
fn install_inset_fixture_terminal(h: &mut PlexiUiHarness, pane: u64) {
    let root = h.workspace_root().to_path_buf();
    h.with_app_mut(|app| {
        let script = "i=0; while [ $i -lt 120 ]; do printf '%0300d\\n' 0 | tr 0 W; i=$((i+1)); done; while IFS= read -r line; do eval \"$line\"; done";
        let mut term = crate::host::pane::TerminalPane::fixture(pane, app.ctx.clone(), app.pty_event_tx.clone(),
            egui_term::BackendSettings { shell: "/bin/sh".into(), args: vec!["-c".into(), script.into()], working_directory: Some(root), ..Default::default() }, 14.0).expect("spawn test terminal");
        term.outside_workspace_cached = false;
        term.outside_workspace_checked_at = Some(Instant::now());
        term.outside_workspace_root = Some(app.router.active().root.clone());
        term.spawned_by = Some("human".into());
        term.name = Some("fixture".into());
        term.agent = Some(plexi_protocol::PaneAgentState { pane_id: pane, state: plexi_protocol::AgentState::Blocked, agent: "fixture".into(), detail: None, session_id: None });
        app.ctx.data_mut(|d| d.insert_temp(egui::Id::new(("provenance_icon_rect", pane)), egui::Rect::NOTHING));
        let win = app.active_window;
        app.windows[win].replace_pane_for_test(pane, Pane::Terminal(Box::new(term)));
    });
    h.focus_pane(pane).unwrap();
    let deadline = Instant::now() + crate::testing::load_aware_timeout(Duration::from_secs(10));
    loop {
        h.run_steps(3);
        let ready = h.with_app_mut(|app| {
            app.windows[app.active_window]
                .panes
                .get_mut(&pane)
                .unwrap()
                .as_terminal_mut()
                .unwrap()
                .backend
                .capture_lines(50)
                .iter()
                .any(|line| line.contains("WWWWWWWW"))
        });
        if ready {
            h.run_steps(3);
            break;
        }
        assert!(Instant::now() < deadline, "terminal fixture timed out");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Ask the shell itself for its tty size (rows, cols). The marker is short and on its own
/// line so a narrow wrapped terminal cannot split it.
fn pty_reported_size(h: &mut PlexiUiHarness, pane: u64) -> (usize, usize) {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let marker = format!(
        "Q{}",
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    h.with_app_mut(|app| {
        app.windows[app.active_window]
            .panes
            .get_mut(&pane)
            .unwrap()
            .as_terminal_mut()
            .unwrap()
            .backend
            .process_command(egui_term::BackendCommand::Write(
                format!("printf '{marker}\\n'; stty size\n").into_bytes(),
            ))
    });
    let deadline = Instant::now() + crate::testing::load_aware_timeout(Duration::from_secs(10));
    loop {
        h.run_steps(3);
        let lines = h.with_app_mut(|app| {
            app.windows[app.active_window]
                .panes
                .get_mut(&pane)
                .unwrap()
                .as_terminal_mut()
                .unwrap()
                .backend
                .capture_lines(200)
        });
        if let Some(index) = lines.iter().position(|line| line.trim() == marker) {
            let actual: Vec<usize> = lines[index + 1..]
                .iter()
                .flat_map(|line| line.split_whitespace())
                .filter_map(|word| word.parse().ok())
                .take(2)
                .collect();
            if actual.len() == 2 {
                return (actual[0], actual[1]);
            }
        }
        assert!(
            Instant::now() < deadline,
            "PTY size fixture timed out: {lines:?}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Terminal widget rect, its expected inset grid, and the ink of the drawn glyphs.
struct TerminalInsetMetrics {
    rect: egui::Rect,
    rows: usize,
    cols: usize,
    ink: egui::Rect,
    glyph_left: f32,
}

fn terminal_inset_metrics(h: &mut PlexiUiHarness, pane: u64) -> TerminalInsetMetrics {
    let rect = h
        .harness()
        .ctx
        .read_response(egui_term::terminal_widget_id(pane))
        .unwrap()
        .rect;
    let (cell_width, cell_height) = h.with_app_mut(|app| {
        let size = app.windows[app.active_window]
            .panes
            .get_mut(&pane)
            .unwrap()
            .as_terminal_mut()
            .unwrap()
            .backend
            .last_content()
            .terminal_size;
        (size.cell_width as f32, size.cell_height as f32)
    });
    let pad = crate::render::terminal_pane::terminal_content_padding();
    let mut ink = egui::Rect::NOTHING;
    let mut glyph_left = f32::INFINITY;
    for shape in &h.harness().output().shapes {
        if let egui::Shape::Text(t) = &shape.shape {
            if rect.contains(t.pos) {
                ink = ink.union(shape.shape.visual_bounding_rect());
                glyph_left = glyph_left.min(t.pos.x);
            }
        }
    }
    assert!(
        glyph_left.is_finite(),
        "terminal glyphs must be rendered inside {rect:?}"
    );
    TerminalInsetMetrics {
        rect,
        rows: ((rect.height() - 2.0 * pad.y) / cell_height) as usize,
        cols: ((rect.width() - 2.0 * pad.x) / cell_width) as usize,
        ink,
        glyph_left,
    }
}

/// The drawn grid, the shell's tty size and the ink all agree with the inset rect.
/// Returns the left glyph inset measured from the widget rect.
fn assert_terminal_inset_geometry(h: &mut PlexiUiHarness, pane: u64, frame: egui::Rect) -> f32 {
    use crate::ui::style::{PANE_FOCUS_WIDTH, PANE_STATE_WIDTH};
    let rings = PANE_FOCUS_WIDTH + PANE_STATE_WIDTH;
    let pad = crate::render::terminal_pane::terminal_content_padding();
    // The resize is debounced; settle until the shell reports the expected size.
    let deadline = Instant::now() + crate::testing::load_aware_timeout(Duration::from_secs(10));
    let (m, reported) = loop {
        let m = terminal_inset_metrics(h, pane);
        let reported = pty_reported_size(h, pane);
        if reported == (m.rows, m.cols) || Instant::now() > deadline {
            break (m, reported);
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    assert_eq!(
        reported,
        (m.rows, m.cols),
        "PTY (rows, cols) must match the inset grid of {:?}",
        m.rect
    );
    let (drawn_cols, drawn_rows) = h.with_app_mut(|app| {
        let content = app.windows[app.active_window]
            .panes
            .get_mut(&pane)
            .unwrap()
            .as_terminal_mut()
            .unwrap()
            .backend
            .last_content();
        (
            content
                .grid
                .display_iter()
                .map(|cell| cell.point.column.0 + 1)
                .max()
                .unwrap(),
            content
                .grid
                .display_iter()
                .map(|cell| cell.point.line.0 + 1)
                .max()
                .unwrap() as usize,
        )
    });
    assert_eq!(
        (drawn_rows, drawn_cols),
        (m.rows, m.cols),
        "drawn grid must match the PTY grid"
    );
    // Glyph origin sits at the shared inset (galley offsets are sub-pixel), ink stays off the rings.
    let inset = m.glyph_left - m.rect.left();
    assert!(
        (inset - pad.x).abs() < 0.5,
        "glyph inset {inset} must equal the shared padding {}",
        pad.x
    );
    assert!(
        m.ink.left() >= frame.left() + rings,
        "ink {:?} overlaps left rings of {frame:?}",
        m.ink
    );
    assert!(
        m.ink.right() <= frame.right() - rings,
        "ink {:?} overlaps right rings of {frame:?}",
        m.ink
    );
    assert!(
        m.ink.bottom() <= frame.bottom() - rings,
        "ink {:?} overlaps bottom rings of {frame:?}",
        m.ink
    );
    inset
}

#[test]
#[ignore = "requires-pty"]
fn pane_content_clears_focused_state_rings() {
    for scale in [1.0, 2.0] {
        for width in [480.0, 1200.0] {
            let (mut h, pane, _) = pane_chrome_fixture_sized(scale, width);
            std::fs::write(
                crate::config::config_path(),
                format!("ui_scale = {scale}\n"),
            )
            .unwrap();
            h.with_app_mut(|app| {
                app.reload_config();
                app.ui_scale = scale;
            });
            h.run_steps(3);
            h.with_app(|app| {
                assert!(
                    (app.ctx.zoom_factor() - scale).abs() < 0.001,
                    "fixture must retain requested scale"
                )
            });
            install_inset_fixture_terminal(&mut h, pane);
            let frame = h.with_app(|app| app.visible_pane_rect(pane).unwrap());
            assert_terminal_inset_geometry(&mut h, pane, frame);
        }
    }
}

#[test]
#[ignore = "requires-pty"]
fn zoomed_terminal_grid_matches_inset_and_normal_view() {
    let (mut h, pane, _) = pane_chrome_fixture_sized(1.0, 1200.0);
    install_inset_fixture_terminal(&mut h, pane);
    let frame = h.with_app(|app| app.visible_pane_rect(pane).unwrap());
    let normal_inset = assert_terminal_inset_geometry(&mut h, pane, frame);
    let normal_rect = terminal_inset_metrics(&mut h, pane).rect;
    h.with_app_mut(|app| {
        let win = &mut app.windows[app.active_window];
        let tile = win.tree.tiles.find_pane(&pane).unwrap();
        win.zoom_to(tile);
    });
    h.run_steps(5);
    let zoomed_rect = terminal_inset_metrics(&mut h, pane).rect;
    assert!(
        zoomed_rect.width() > normal_rect.width(),
        "zoom must enlarge the terminal: {normal_rect:?} -> {zoomed_rect:?}"
    );
    let zoomed_inset = assert_terminal_inset_geometry(&mut h, pane, zoomed_rect);
    assert!(
        (zoomed_inset - normal_inset).abs() < 0.01,
        "zoomed glyph inset {zoomed_inset} must equal normal {normal_inset}"
    );
}

/// PTY (rows, cols) from the shell must equal the inset grid of the pane's current widget rect.
fn assert_pty_follows_layout(h: &mut PlexiUiHarness, pane: u64, step: &str) {
    let deadline = Instant::now() + crate::testing::load_aware_timeout(Duration::from_secs(10));
    loop {
        h.run_steps(3);
        let rect = h
            .harness()
            .ctx
            .read_response(egui_term::terminal_widget_id(pane))
            .unwrap()
            .rect;
        let (cw, ch) = h.with_app_mut(|app| {
            let size = app.windows[app.active_window]
                .panes
                .get_mut(&pane)
                .unwrap()
                .as_terminal_mut()
                .unwrap()
                .backend
                .last_content()
                .terminal_size;
            (size.cell_width as f32, size.cell_height as f32)
        });
        let pad = crate::render::terminal_pane::terminal_content_padding();
        let expected = (
            ((rect.height() - 2.0 * pad.y) / ch) as usize,
            ((rect.width() - 2.0 * pad.x) / cw) as usize,
        );
        let reported = pty_reported_size(h, pane);
        if reported == expected {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{step}: pane {pane} PTY {reported:?} != inset grid {expected:?} of {rect:?}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Stint 1213 D1: after a restore, splitting a third pane and zooming must keep every PTY
/// size equal to its drawn grid. Reproduces the installed tester's restart sequence.
#[test]
#[ignore = "requires-pty"]
fn pty_follows_layout_after_restore_split_and_zoom() {
    let (mut h1, first, second) = pane_chrome_fixture_sized(1.0, 1200.0);
    let root = h1.workspace_root().to_path_buf();
    h1.with_app_mut(|app| {
        let term = crate::host::pane::TerminalPane::fixture(
            second,
            app.ctx.clone(),
            app.pty_event_tx.clone(),
            egui_term::BackendSettings {
                shell: "/bin/sh".into(),
                working_directory: Some(root),
                ..Default::default()
            },
            14.0,
        )
        .expect("spawn test terminal");
        // The fixture replaces the placeholder the chrome fixture put in
        // that slot.
        let win = app.active_window;
        app.windows[win]
            .take_pane(second)
            .expect("the chrome fixture's placeholder");
        app.windows[win]
            .panes
            .insert(second, Pane::Terminal(Box::new(term)))
            .unwrap();
    });
    h1.run_steps(5);
    for pane in [first, second] {
        assert_pty_follows_layout(&mut h1, pane, "before restart");
    }
    assert!(
        h1.with_app_mut(|app| app.save_workspace_now()),
        "workspace must save"
    );

    // Restart: a fresh app restored from the saved workspace, like a host relaunch.
    let mut h = PlexiUiHarness::new_sized(1200.0, 700.0);
    h.with_app_mut(|app| {
        let profile = crate::config::config_dir();
        let ws = crate::workspace::WorkspaceFile::load_from(&profile).expect("saved workspace");
        let cfg = app.config.clone();
        let colors = app.colors.clone();
        let pty_event_tx = app.pty_event_tx.clone();
        let ctx = app.ctx.clone();
        let restored = crate::app::restore_workspace(
            ws,
            &cfg,
            crate::app::WorkspaceRestoreCollaborators {
                profile_dir: &profile,
                colors: &colors,
                hot_reload: &mut app.hot_reload,
                state_watch: &mut app.state_watch,
                terminal_spawner: Some((&ctx, &pty_event_tx)),
            },
        )
        .expect("restore workspace");
        app.install_restored_workspace(restored);
        app.sidebar_visible = false;
    });
    h.run_steps(5);
    let mut panes: Vec<u64> = h.with_app(|app| {
        app.windows[app.active_window]
            .panes
            .keys()
            .copied()
            .collect()
    });
    panes.sort_unstable();
    assert_eq!(panes.len(), 2, "restore must bring both terminals back");
    for &pane in &panes {
        assert_pty_follows_layout(&mut h, pane, "after restore");
    }

    h.with_app_mut(|app| {
        app.split_focused(true, None, false, false, None);
    });
    h.run_steps(5);
    let mut panes: Vec<u64> = h.with_app(|app| {
        app.windows[app.active_window]
            .panes
            .keys()
            .copied()
            .collect()
    });
    panes.sort_unstable();
    assert_eq!(panes.len(), 3, "split must add a third terminal");
    for &pane in &panes {
        assert_pty_follows_layout(&mut h, pane, "after third split");
    }

    let zoomed = panes[0];
    h.with_app_mut(|app| {
        let win = &mut app.windows[app.active_window];
        let tile = win.tree.tiles.find_pane(&zoomed).unwrap();
        win.zoom_to(tile);
    });
    h.run_steps(5);
    assert_pty_follows_layout(&mut h, zoomed, "zoomed after restore + split");
    h.with_app_mut(|app| app.windows[app.active_window].clear_zoom());
    h.run_steps(5);
    for &pane in &panes {
        assert_pty_follows_layout(&mut h, pane, "after unzoom");
    }
}

#[test]
#[ignore = "requires-pty"]
fn pane_provenance_clears_focused_state_rings() {
    for scale in [1.0, 2.0] {
        for width in [480.0, 1200.0] {
            let (mut h, pane, _) = pane_chrome_fixture_sized(scale, width);
            std::fs::write(
                crate::config::config_path(),
                format!("ui_scale = {scale}\n"),
            )
            .unwrap();
            h.with_app_mut(|app| {
                app.reload_config();
                app.ui_scale = scale;
            });
            h.run_steps(3);
            h.with_app(|app| {
                assert!(
                    (app.ctx.zoom_factor() - scale).abs() < 0.001,
                    "fixture must retain requested scale"
                )
            });
            install_inset_fixture_terminal(&mut h, pane);
            h.with_app(|app| {
                assert!(
                    app.windows[app.active_window]
                        .panes
                        .get(&pane)
                        .and_then(|p| p.as_terminal())
                        .unwrap()
                        .agent
                        .is_some(),
                    "state ring needs an agent state"
                )
            });
            let (frame, icon) = h.with_app(|app| {
                (
                    app.visible_pane_rect(pane).unwrap(),
                    app.ctx.data(|d| {
                        d.get_temp::<egui::Rect>(egui::Id::new(("provenance_icon_rect", pane)))
                            .unwrap()
                    }),
                )
            });
            // Optional accessories collapse at narrow widths by design; wide headers must paint it.
            if width >= 1200.0 {
                assert_ne!(
                    icon,
                    egui::Rect::NOTHING,
                    "wide header must paint provenance"
                );
            }
            if icon == egui::Rect::NOTHING {
                continue;
            }
            assert!(
                icon.right() <= frame.right() - crate::ui::style::PANE_CONTENT_INSET,
                "provenance icon {icon:?} overlaps pane edge {frame:?}"
            );
        }
    }
}

/// Dusk/Dawn at normal and narrow widths, a real PTY running `script`, shot to `evidence_png!`.
fn terminal_inset_screenshots(script: &str, ready: &str, name: impl Fn(&str, f32) -> String) {
    for theme in ["plexi-dusk", "plexi-dawn"] {
        for width in [480.0, 1200.0] {
            let mut h = PlexiUiHarness::new_sized(width, 500.0);
            h.step();
            let pane = add_focused_pane(&mut h);
            let root = h.workspace_root().to_path_buf();
            std::fs::write(
                crate::config::config_path(),
                format!("[theme]\npreset = \"{theme}\"\n"),
            )
            .unwrap();
            h.with_app_mut(|app| {
                app.reload_config();
                let cfg = crate::ui::theme::preset_colors(theme).unwrap();
                app.colors = crate::ui::theme::Colors::from_config(&cfg);
                app.theme = crate::ui::theme::terminal_theme(
                    &cfg,
                    crate::config::DEFAULT_TERMINAL_MINIMUM_CONTRAST,
                );
                crate::ui::theme::setup_style(&app.ctx, &app.colors);
                app.sidebar_visible = false;
                let mut term = crate::host::pane::TerminalPane::fixture(
                    pane,
                    app.ctx.clone(),
                    app.pty_event_tx.clone(),
                    egui_term::BackendSettings {
                        shell: "/bin/sh".into(),
                        args: vec!["-c".into(), script.into()],
                        working_directory: Some(root),
                        ..Default::default()
                    },
                    14.0,
                )
                .expect("spawn test terminal");
                term.name = Some("Claude Code".into());
                term.spawned_by = Some("human".into());
                term.outside_workspace_root = Some(app.router.active().root.clone());
                term.outside_workspace_checked_at = Some(Instant::now());
                term.agent = Some(plexi_protocol::PaneAgentState {
                    pane_id: pane,
                    state: plexi_protocol::AgentState::Working,
                    agent: "fixture".into(),
                    detail: None,
                    session_id: None,
                });
                let win = app.active_window;
                app.windows[win].replace_pane_for_test(pane, Pane::Terminal(Box::new(term)));
            });
            h.focus_pane(pane).unwrap();
            let deadline =
                Instant::now() + crate::testing::load_aware_timeout(Duration::from_secs(10));
            loop {
                h.run_steps(3);
                if h.with_app_mut(|app| {
                    app.windows[app.active_window]
                        .panes
                        .get_mut(&pane)
                        .unwrap()
                        .as_terminal_mut()
                        .unwrap()
                        .backend
                        .capture_lines(50)
                        .iter()
                        .any(|line| line.contains(ready))
                }) {
                    h.run_steps(3);
                    break;
                }
                assert!(Instant::now() < deadline, "terminal fixture timed out");
                std::thread::sleep(Duration::from_millis(20));
            }
            h.with_app(|app| {
                assert_eq!(
                    app.colors.bg_darkest,
                    crate::ui::theme::Colors::from_config(
                        &crate::ui::theme::preset_colors(theme).unwrap()
                    )
                    .bg_darkest,
                    "fixture must retain requested theme"
                )
            });
            // Registered pane-header fixture: the provenance, agent title
            // and outside-workspace badge fit beside each other at both
            // widths and clear contrast in both themes.
            assert_layout_surface(&h, "pane_header");
            let path = name(theme, width);
            h.save_screenshot(&path).unwrap();
            println!("screenshot: {path}");
        }
    }
}

#[test]
#[ignore = "requires-pty"]
fn screenshot_terminal_inset_and_provenance_matrix() {
    terminal_inset_screenshots(
        "printf '* Working on your request\\nWIDEGLYPH\\n'; sleep 300",
        "WIDEGLYPH",
        |theme, width| evidence_png!(&format!("_{theme}_{width}")),
    );
}

/// Full-screen TUI with non-default cell backgrounds: the theme-colored gutter around the
/// grid is intentional (matches iTerm/Ghostty); edge-cell bleed is a possible follow-up.
#[test]
#[ignore = "requires-pty"]
fn screenshot_terminal_inset_colored_background_tui() {
    let script = "i=0; while [ $i -lt 40 ]; do printf '\\033[4%dm%-300s\\033[0m\\n' $((i%6+1)) \"TUIROW $i colored background\"; i=$((i+1)); done; sleep 300";
    terminal_inset_screenshots(script, "TUIROW", |theme, width| {
        evidence_png!(&format!("_{theme}_{width}"))
    });
}

#[test]
fn pane_chrome_state_ring_is_visible_with_and_without_focus() {
    use plexi_protocol::AgentState;
    let (mut h, pane, other) = pane_chrome_fixture(1.0);
    for state in [
        AgentState::Working,
        AgentState::Blocked,
        AgentState::NeedsYou,
        AgentState::Interrupted,
        AgentState::Failed,
    ] {
        for focused in [false, true] {
            h.with_app_mut(|app| {
                app.windows[app.active_window]
                    .panes
                    .get_mut(&pane)
                    .unwrap()
                    .set_agent(Some(plexi_protocol::PaneAgentState {
                        pane_id: pane,
                        state: state.clone(),
                        agent: "fixture".into(),
                        detail: None,
                        session_id: None,
                    }));
            });
            h.focus_pane(if focused { pane } else { other }).unwrap();
            h.run_steps(3);
            let (rect, color, accent) = h.with_app(|app| {
                (
                    app.visible_pane_rect(pane).unwrap(),
                    crate::ui::activity::dot_color_from_time(&state, &app.colors, 0.0, 0),
                    app.colors.accent,
                )
            });
            let shapes = &h.harness().output().shapes;
            let ring = shapes.iter().filter_map(|s| match &s.shape { egui::Shape::Path(r) if r.stroke.color == egui::epaint::ColorMode::Solid(color) && r.points.iter().all(|p| rect.contains(*p)) => Some(r), _ => None }).next().unwrap_or_else(|| panic!("state-color ring must be painted: {state:?}, focused={focused}, paths={:?}", shapes.iter().filter_map(|s| match &s.shape { egui::Shape::Path(p) if p.closed && p.points.len() == 36 => Some((&p.stroke.color, egui::Rect::from_points(&p.points))), _ => None }).collect::<Vec<_>>()));
            assert!(ring.stroke.width > 0.0);
            if focused {
                let focus = shapes
                    .iter()
                    .filter_map(|s| match &s.shape {
                        egui::Shape::Rect(r) if r.stroke.color == accent && r.rect == rect => {
                            Some(r)
                        }
                        _ => None,
                    })
                    .next()
                    .expect("focus ring");
                let left = ring
                    .points
                    .iter()
                    .map(|p| p.x)
                    .fold(f32::INFINITY, f32::min)
                    - ring.stroke.width / 2.0;
                assert!(
                    left + 0.001 >= focus.rect.left() + focus.stroke.width,
                    "focus and state strokes overlap"
                );
            }
        }
    }
}

/// Stint 1213 round 3: an unfocused pane in a quiet state (Idle, Completed) paints no ring, so
/// the edge band is the pane background. The gray `state_waiting` ring was the stray line.
#[test]
fn unfocused_quiet_pane_edge_is_pane_background() {
    use plexi_protocol::AgentState;
    let (mut h, pane, other) = pane_chrome_fixture(1.0);
    for state in [AgentState::Idle, AgentState::Completed] {
        h.with_app_mut(|app| {
            app.windows[app.active_window]
                .panes
                .get_mut(&pane)
                .unwrap()
                .set_agent(Some(plexi_protocol::PaneAgentState {
                    pane_id: pane,
                    state: state.clone(),
                    agent: "fixture".into(),
                    detail: None,
                    session_id: None,
                }));
        });
        h.focus_pane(other).unwrap();
        h.run_steps(4);
        let rect = h.with_app(|app| app.visible_pane_rect(pane).unwrap());
        let img = h.render().unwrap();
        let ppp = img.width() as f32 / h.with_app(|app| app.ctx.content_rect().width());
        let y = ((rect.center().y) * ppp) as u32;
        let at = |dx: f32| *img.get_pixel(((rect.left() + dx) * ppp) as u32, y);
        let band = at(crate::ui::style::PANE_STATE_WIDTH / 2.0);
        let inside = at(crate::ui::style::PANE_RING_WIDTH + 0.5);
        assert_eq!(
            band, inside,
            "{state:?}: ring band pixel {band:?} must equal pane background {inside:?}"
        );
        assert_ne!(
            at(-3.0),
            inside,
            "probe must sit on the pane edge, not in the gap"
        );
    }
}

/// Stint 1216 rule: focus changes paint, never layout. Header height and terminal top
/// offset from the pane rect are identical focused and unfocused.
#[test]
fn header_and_body_layout_is_identical_focused_and_unfocused() {
    let (mut h, pane, other) = pane_chrome_fixture(1.0);
    h.with_app_mut(|app| {
        let t = app.windows[app.active_window].panes.get_mut(&pane).unwrap();
        if let Some(term) = t.as_terminal_mut() {
            term.name = Some("fixture".into());
            term.spawned_by = Some("researcher".into());
        }
        t.set_agent(Some(plexi_protocol::PaneAgentState {
            pane_id: pane,
            state: plexi_protocol::AgentState::Idle,
            agent: "fixture".into(),
            detail: None,
            session_id: None,
        }));
    });
    let mut tops = Vec::new();
    for focus in [pane, other] {
        h.focus_pane(focus).unwrap();
        h.run_steps(4);
        let rect = h.with_app(|app| app.visible_pane_rect(pane).unwrap());
        let body = h
            .harness()
            .ctx
            .read_response(egui_term::terminal_widget_id(pane))
            .unwrap()
            .rect;
        tops.push(body.top() - rect.top());
        let icon = h.with_app(|app| {
            app.ctx.data(|d| {
                d.get_temp::<egui::Rect>(egui::Id::new(("provenance_icon_rect", pane)))
            })
        });
        {
            let icon = icon
                .filter(|i| *i != egui::Rect::NOTHING)
                .expect("header paints the provenance icon");
            let (above, below) = (
                icon.top() - rect.top(),
                rect.top() + crate::ui::style::PANE_HEADER_HEIGHT - icon.bottom(),
            );
            assert!(
                (above - below).abs() <= 0.5,
                "icon gap above {above} vs below {below} (focus {focus})"
            );
        }
    }
    assert!(
        (tops[0] - tops[1]).abs() < 0.01,
        "terminal top offset focused {} vs unfocused {}",
        tops[0],
        tops[1]
    );
    assert!(
        (tops[0] - crate::ui::style::PANE_HEADER_HEIGHT).abs() < 0.01,
        "body starts right under the {}pt header, got {}",
        crate::ui::style::PANE_HEADER_HEIGHT,
        tops[0]
    );
}

/// An unfocused idle pane paints no outline, seam or veil: below the header, edge band and
/// the strip under it are the pane background, exactly as in the focused render except the ring.
#[test]
fn unfocused_idle_pane_has_no_seam_or_outline_pixels() {
    let (mut h, pane, other) = pane_chrome_fixture(1.0);
    h.with_app_mut(|app| {
        let t = app.windows[app.active_window].panes.get_mut(&pane).unwrap();
        t.set_agent(Some(plexi_protocol::PaneAgentState {
            pane_id: pane,
            state: plexi_protocol::AgentState::Idle,
            agent: "fixture".into(),
            detail: None,
            session_id: None,
        }));
    });
    let mut shots = Vec::new();
    for focus in [other, pane] {
        h.focus_pane(focus).unwrap();
        h.run_steps(4);
        let rect = h.with_app(|app| app.visible_pane_rect(pane).unwrap());
        let img = h.render().unwrap();
        let ppp = img.width() as f32 / h.with_app(|app| app.ctx.content_rect().width());
        shots.push((rect, img, ppp));
    }
    let (rect, img, ppp) = &shots[0];
    let px = |x: f32, y: f32| *img.get_pixel((x * ppp) as u32, (y * ppp) as u32);
    let body = px(
        rect.right() - 20.0,
        rect.top() + crate::ui::style::PANE_HEADER_HEIGHT + 30.0,
    );
    let header = px(
        rect.right() - 3.0,
        rect.top() + crate::ui::style::PANE_HEADER_HEIGHT - 2.0,
    );
    assert_ne!(
        header, body,
        "header fill differs from body, so a seam would be visible"
    );
    // Column under the header must switch from header fill to body fill with no intermediate colour.
    let mut colours = std::collections::BTreeSet::new();
    let mut y = rect.top() + crate::ui::style::PANE_HEADER_HEIGHT - 3.0;
    while y < rect.top() + crate::ui::style::PANE_HEADER_HEIGHT + 20.0 {
        colours.insert(px(rect.right() - 3.0, y).0);
        y += 0.5;
    }
    assert!(
        colours.len() <= 2,
        "unfocused header/body column has seam colours {colours:?}"
    );
    // Edge band down the left side below the header is the body fill.
    let mut y = rect.top() + crate::ui::style::PANE_HEADER_HEIGHT + 20.0;
    while y < rect.bottom() - 20.0 {
        for dx in [0.5, 1.5, 3.0, 5.0] {
            assert_eq!(px(rect.left() + dx, y), body, "edge pixel {dx} at y {y}");
        }
        y += 7.0;
    }
    // Focused render: same layout; only the ring colours the edge.
    let (_, focused, _) = &shots[1];
    let fpx = |x: f32, y: f32| *focused.get_pixel((x * ppp) as u32, (y * ppp) as u32);
    let f = fpx(
        rect.right() - 20.0,
        rect.top() + crate::ui::style::PANE_HEADER_HEIGHT + 30.0,
    );
    // Ghost opacity may round one channel by 1/255; anything more would be a veil.
    assert!(
        f.0.iter().zip(body.0).all(|(a, b)| a.abs_diff(b) <= 1),
        "focused body {f:?} vs unfocused {body:?}"
    );
}

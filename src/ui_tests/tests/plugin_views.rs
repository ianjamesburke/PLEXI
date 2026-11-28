use super::*;

/// A plugin view of rows that hand out their width (stints 1358, 1363),
/// decoded from the wire as a Python plugin's view arrives.
struct RowLayoutFixture;

impl RowLayoutFixture {
    fn tree() -> crate::host::wasm_view::UiTree {
        let button = |label: &str| serde_json::json!({"type": "button", "label": label, "on_click": label});
        let nodes = [
            (
                0,
                serde_json::json!({"type": "column", "children": [1, 2, 3, 4]}),
            ),
            // `Row([Back, Spacer(grow=True), Next])`
            (
                1,
                serde_json::json!({"type": "row", "children": [10, 11, 12]}),
            ),
            (10, button("Back step")),
            (11, serde_json::json!({"type": "spacer", "grow": true})),
            (12, button("Next step")),
            // `Row([..], justify="center")`
            (
                2,
                serde_json::json!({"type": "row", "children": [20, 21], "justify": "center"}),
            ),
            (20, button("Pause timer")),
            (21, button("Reset timer")),
            // Gauntlet card 07: two growing columns side by side.
            (
                3,
                serde_json::json!({"type": "row", "children": [30, 31], "align": "start"}),
            ),
            (
                30,
                serde_json::json!({"type": "column", "children": [32, 33], "grow": true}),
            ),
            (
                31,
                serde_json::json!({"type": "column", "children": [34, 35], "grow": true}),
            ),
            (32, button("Convert units")),
            (33, serde_json::json!({"type": "text", "text": "12 in"})),
            (34, button("Copy result")),
            (35, serde_json::json!({"type": "text", "text": "30.48 cm"})),
            // Gauntlet card 10: centered columns as cells of a row.
            (
                4,
                serde_json::json!({"type": "row", "children": [40, 41, 42], "gap": 12.0}),
            ),
            (
                40,
                serde_json::json!({"type": "column", "children": [43, 44], "align": "center"}),
            ),
            (
                41,
                serde_json::json!({"type": "column", "children": [45, 46], "align": "center"}),
            ),
            (
                42,
                serde_json::json!({"type": "column", "children": [47, 48], "align": "center"}),
            ),
            (
                43,
                serde_json::json!({"type": "badge", "text": "good", "color": "success"}),
            ),
            (44, button("Happy mood")),
            (
                45,
                serde_json::json!({"type": "badge", "text": "fine", "color": "warning"}),
            ),
            (46, button("Meh mood")),
            (
                47,
                serde_json::json!({"type": "badge", "text": "low", "color": "danger"}),
            ),
            (48, button("Sad mood")),
        ];
        let nodes: Vec<serde_json::Value> = nodes
            .iter()
            .map(
                |(id, data)| serde_json::json!({"id": id, "key": id.to_string(), "data": data}),
            )
            .collect();
        crate::host::wasm_python::decode_ui_tree(
            &serde_json::json!({"root": 0, "nodes": nodes}).to_string(),
        )
        .expect("row layout tree decodes")
    }
}

impl crate::app::view_trait::View for RowLayoutFixture {
    fn type_id(&self) -> &'static str {
        "row_layout_fixture"
    }
    fn display_name(&self) -> String {
        "Row layout".into()
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
    fn ui(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &crate::app::view_trait::ViewRenderContext<'_>,
    ) {
        crate::host::wasm_render::render_ui_tree_with_surface(
            ui,
            &Self::tree(),
            ctx.colors,
            None,
            None,
            0,
        );
    }
}

/// Stints 1358 and 1363 in a real plugin pane: a grow spacer pins a
/// button to the row's right edge, `justify="center"` centers a pair of
/// buttons, two growing columns split the row, and centered column cells
/// sit side by side instead of each taking the row's width.
#[test]
fn screenshot_rows_hand_out_width_in_a_plugin_pane() {
    for (width, height) in [(800.0, 600.0), (420.0, 360.0)] {
        let mut h = PlexiUiHarness::new_sized_ppp(width, height, 1.0);
        let pane_id = add_focused_pane(&mut h);
        h.with_app_mut(|app| {
            if let Some(Pane::View(pane)) =
                app.windows[app.active_window].panes.get_mut(&pane_id)
            {
                pane.runtime = ViewRuntime::Builtin(Box::new(RowLayoutFixture));
            }
        });
        h.run_steps(3);
        // A row's measuring pass lays each child out once, invisible and
        // disabled, before painting it, and egui gives those widgets
        // accessibility nodes too: the painted button is the enabled one.
        let mut rect = |label: &str| {
            use egui_kittest::kittest::NodeT;
            let painted: Vec<egui::Rect> = h
                .harness()
                .get_all_by_role_and_label(egui::accesskit::Role::Button, label)
                .filter(|node| !node.accesskit_node().is_disabled())
                .map(|node| node.rect())
                .collect();
            assert_eq!(painted.len(), 1, "{label}: painted once, got {painted:?}");
            painted[0]
        };
        let (back, next) = (rect("Back step"), rect("Next step"));
        let (pause, reset) = (rect("Pause timer"), rect("Reset timer"));
        let (convert, copy) = (rect("Convert units"), rect("Copy result"));
        let moods = [rect("Happy mood"), rect("Meh mood"), rect("Sad mood")];
        let size = format!("{width}x{height}");

        // The rows share left and right edges: Back starts them, the
        // spacer pins Next to the end.
        let (left, right) = (back.min.x, next.max.x);
        assert!(
            right - left > width / 2.0,
            "{size}: Next is pinned right: {back:?} {next:?}"
        );
        let pair_middle = (pause.min.x + reset.max.x) / 2.0;
        assert!(
            (pair_middle - (left + right) / 2.0).abs() <= 2.0,
            "{size}: justify center: {pause:?} {reset:?} in {left}..{right}"
        );
        let half = (right - left - crate::ui::style::SPACE_SM) / 2.0;
        assert!(
            (copy.min.x - (left + half + crate::ui::style::SPACE_SM)).abs() <= 2.0,
            "{size}: the second grow column starts halfway: {convert:?} {copy:?}"
        );
        assert!((convert.min.x - left).abs() <= 2.0, "{size}: {convert:?}");
        for pair in moods.windows(2) {
            assert!(
                pair[1].min.x - pair[0].max.x <= 12.0 + 2.0,
                "{size}: mood cells sit side by side: {moods:?}"
            );
        }
        assert!(moods[2].max.x <= right + 0.5, "{size}: {moods:?}");
        h.save_screenshot(&evidence_png!(&format!("_{size}")))
            .expect("row layout screenshot");
    }
}

#[test]
fn screenshot_agents_pane_states_and_keys() {
    use plexi_protocol::AgentState;
    let mut h = PlexiUiHarness::new_sized(1200.0, 850.0);
    let root = h.workspace_root().to_path_buf();
    let mut ids = Vec::new();
    for (role, state, detail) in [
        ("Planner", AgentState::Working, None),
        (
            "Implementer",
            AgentState::Blocked,
            Some("Which branch should receive the fix?"),
        ),
        (
            "Reviewer",
            AgentState::NeedsYou,
            Some("Approve the deployment plan"),
        ),
        ("Tester", AgentState::Completed, None),
        (
            "Researcher",
            AgentState::Failed,
            Some("harness exited with code 1 without agent done"),
        ),
        ("Writer", AgentState::Idle, None),
        ("Runner", AgentState::Working, None),
    ] {
        let id = add_focused_pane(&mut h);
        ids.push(id);
        h.with_app_mut(|app| {
            let mut term = crate::host::pane::TerminalPane::fixture(
                id,
                app.ctx.clone(),
                app.pty_event_tx.clone(),
                egui_term::BackendSettings {
                    shell: "/bin/sh".into(),
                    args: vec!["-c".into(), "sleep 300".into()],
                    working_directory: Some(root.clone()),
                    ..Default::default()
                },
                14.0,
            )
            .expect("spawn test terminal");
            term.agent_role = Some(role.into());
            term.agent_harness = Some("pi".into());
            term.agent_completed = state == AgentState::Completed;
            term.tail_preview =
                Some("Inspected the affected files. Ready for the next step.".into());
            term.parent_pane_id = (role == "Implementer").then_some(ids[0]);
            term.agent = Some(plexi_protocol::PaneAgentState {
                pane_id: id,
                state,
                agent: "pi".into(),
                detail: detail.map(str::to_string),
                session_id: None,
            });
            app.windows[app.active_window].take_pane(id);
            app.windows[app.active_window]
                .panes
                .insert(id, Pane::Terminal(Box::new(term)))
                .unwrap();
            app.sidebar_visible = false;
        });
    }
    h.with_app_mut(|app| {
        let win = &mut app.windows[app.active_window];
        let tiles = ids
            .iter()
            .map(|id| win.tree.tiles.find_pane(id).unwrap())
            .collect();
        win.tree.root = Some(win.tree.tiles.insert_tab_tile(tiles));
    });
    h.with_app_mut(|app| {
        app.open_command_palette("agents");
    });
    h.run_steps(2);
    h.save_screenshot(&evidence_png!("_before")).unwrap();
    h.with_app_mut(|app| app.close_command_palette());
    h.step();
    let agents_id = h.open_builtin_by_id("agents", &root, &[]).unwrap();
    h.run_steps(3);
    h.with_app_mut(|app| {
        let win = &mut app.windows[app.active_window];
        win.zoomed_pane = win.tree.tiles.find_pane(&agents_id);
    });
    h.run_steps(3);
    h.harness().get_by_label("Agents");
    h.save_screenshot(&evidence_png!("_after")).unwrap();
    h.harness().set_size(egui::vec2(520.0, 760.0));
    h.run_steps(2);
    h.save_screenshot(&evidence_png!("_narrow")).unwrap();
    h.harness().set_size(egui::vec2(1200.0, 850.0));
    h.run_steps(2);
    // The selection starts at human attention, then the parent-routed question.
    h.harness().key_press(egui::Key::J);
    h.step();
    h.harness().key_press(egui::Key::A);
    h.run_steps(2);
    h.harness().get_by_label("Reply to worker");
    h.harness()
        .input_mut()
        .events
        .push(egui::Event::Text("Use feature/0846".into()));
    h.step();
    h.harness().get_by_label("Send reply").click();
    h.run_steps(2);
    h.with_app(|app| {
        let view = app.agents_view(plexi_protocol::FleetFilter::All);
        let row = view
            .tasks
            .iter()
            .flat_map(|t| &t.rows)
            .find(|r| r.pane_id == ids[1])
            .unwrap();
        assert_eq!(
            row.display_state,
            crate::agent::fleet::DisplayState::Working
        );
        assert!(app.pane_mailboxes[&ids[1]]
            .front()
            .unwrap()
            .payload
            .contains("feature/0846"));
    });
    h.harness().key_press(egui::Key::Enter);
    h.run_steps(2);
    h.with_app(|app| assert_eq!(app.fleet_focused_pane(), Some(ids[1])));
}

/// Stint 1404 visual review: two Logs panes side by side, the left one
/// focused. The INFO chip (an accent Primary tab) must read the same in
/// both; the unfocused pane is only dimmed. The colour rule is pinned by
/// `primary_label_colour_does_not_flip_with_pane_opacity`; this PNG is the
/// screenshot pair for the PR.
#[test]
fn screenshot_logs_info_chip_focused_and_unfocused() {
    let plugin_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("plugins/logs");
    let mut h = PlexiUiHarness::new_sized(1100.0, 420.0);
    let workspace = h.workspace_root().to_path_buf();
    let logs = h.open_view_at(&plugin_dir, &[]).expect("launch logs");
    h.wait_for_view_frame(logs, Duration::from_secs(30))
        .expect("logs renders");
    let other = h
        .open_target(HarnessOpenTarget::Builtin {
            id: "file_browser",
            cwd: &workspace,
            args: &[],
        })
        .expect("open a second pane");
    // Select the INFO level tab: it becomes the accent (Primary) chip.
    h.with_app_mut(|app| {
        app.handle_pane_ipc_request(crate::broker::caller::RequestOrigin::HostSystem, plexi_protocol::HostRequest::SendPaneAction {
            pane_id: logs,
            action: "logs-level:3".into(),
            args: Vec::new(),
            response_file: None,
        })
    });
    h.run_steps(6);
    assert!(h.pane_has_label(logs, "INFO"));
    for (focus, suffix) in [(logs, "-focused"), (other, "-unfocused")] {
        h.focus_pane(focus).expect("focus pane");
        h.run_steps(6);
        let path = evidence_png_path(enclosing_fn_name!(), suffix);
        h.save_screenshot(&path).expect("save screenshot");
    }
}

/// Stint 0720 visual review: the todo plugin's add form, autofocused and
/// mid-draft. Drives the real plugin process through the production paths —
/// `a` opens the form on the guest's raw KeyEvent path, `autofocus` hands
/// the cursor to the host TextEdit with no click, and the typed draft is
/// painted from the host-owned buffer while it round-trips to the guest.
/// The assertion is the gate; the PNG is for a human/agent to look at.
#[test]
fn screenshot_todo_add_form_autofocused_draft() {
    let plugin_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("plugins/todo");
    let mut h = PlexiUiHarness::new_sized(900.0, 620.0);
    let pane_id = h.open_view_at(&plugin_dir, &[]).expect("launch todo");
    h.wait_for_view_frame(pane_id, Duration::from_secs(30))
        .expect("todo renders its first frame");
    h.focus_pane(pane_id).expect("focus todo pane");
    h.run_steps(2);

    // `a` opens the add form (the guest's raw KeyEvent path — no text
    // surface is focused yet).
    h.harness().input_mut().events.push(egui::Event::Key {
        key: egui::Key::A,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    });
    h.step();

    let pane_text = |h: &PlexiUiHarness| -> String {
        h.with_app(|app| {
            app.windows[app.active_window]
                .panes
                .get(&pane_id)
                .and_then(Pane::as_view)
                .map(|pane| {
                    pane.semantic_state()
                        .nodes
                        .iter()
                        .flat_map(|n| {
                            n.label
                                .clone()
                                .into_iter()
                                .chain(n.value.clone().map(|v| format!("value:{v}")))
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default()
        })
    };
    let start = Instant::now();
    while !pane_text(&h).contains("What needs doing?") {
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "todo add form never appeared:\n{}",
            pane_text(&h)
        );
        h.step();
        std::thread::sleep(Duration::from_millis(25));
    }

    // Type without clicking: `autofocus` must already own the cursor. One
    // character per frame, faster than the guest can echo any of them.
    for ch in "Buy oat milk".chars() {
        h.harness()
            .input_mut()
            .events
            .push(egui::Event::Text(ch.to_string()));
        h.step();
    }
    let start = Instant::now();
    while !pane_text(&h).contains("value:Buy oat milk") {
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "autofocused draft never round-tripped to the guest:\n{}",
            pane_text(&h)
        );
        h.step();
        std::thread::sleep(Duration::from_millis(25));
    }

    h.save_screenshot(&evidence_png!())
        .expect("render todo add form");
}

#[test]
fn screenshot_cli_renderer_shared_chrome() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let desc_path = tmp.path().join("descriptor.json");
    std::fs::write(
        &desc_path,
        r#"{
                "plexi_version": "0.1",
                "name": "cargo",
                "version": "1.84.0",
                "description": "Rust's package manager and build tool",
                "default_view": "list",
                "commands": [
                    {
                        "name": "build",
                        "description": "Compile a local package and all of its dependencies",
                        "args": [
                            {"name": "package", "type": "string", "required": false, "placeholder": "plexi"}
                        ],
                        "flags": [
                            {"name": "--release", "type": "bool", "description": "Build artifacts in release mode"}
                        ]
                    },
                    {
                        "name": "test",
                        "description": "Execute all unit and integration tests"
                    }
                ]
            }"#,
    )
    .expect("write descriptor.json");

    let mut h = PlexiUiHarness::new_sized_ppp(720.0, 680.0, 2.0);
    let desc_str = desc_path.to_string_lossy().into_owned();
    let cwd = tmp.path().to_path_buf();
    h.with_app_mut(|app| {
        app.sidebar_visible = false;
        app.open_builtin_view_pane(
            Box::new(crate::render::cli_renderer_view::CliRendererView::new(
                desc_str,
            )),
            PluginPermissions::builtin(),
            cwd,
            None,
            None,
            None,
        );
    });
    h.run_steps(3);
    h.with_app_mut(|app| {
        let cli = app.windows[app.active_window]
            .panes
            .values_mut()
            .find_map(|p| {
                let ViewRuntime::Builtin(b) = &mut p.as_view_mut()?.runtime else {
                    return None;
                };
                b.as_any_mut()
                    .downcast_mut::<crate::render::cli_renderer_view::CliRendererView>()
            })
            .expect("CliRendererView pane");
        assert!(cli.last_header_rect.is_some());
        assert!(cli.last_footer_chip_rects.len() >= 3);
    });
    h.save_screenshot(&evidence_png!("_list"))
        .expect("render cli_renderer list chrome");

    // Press Enter to open the `build` command form.
    h.harness().input_mut().events.push(egui::Event::Key {
        key: egui::Key::Enter,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    });
    h.run_steps(3);
    h.with_app_mut(|app| {
        let cli = app.windows[app.active_window]
            .panes
            .values_mut()
            .find_map(|p| {
                let ViewRuntime::Builtin(b) = &mut p.as_view_mut()?.runtime else {
                    return None;
                };
                b.as_any_mut()
                    .downcast_mut::<crate::render::cli_renderer_view::CliRendererView>()
            })
            .expect("CliRendererView pane");
        assert!(cli.last_header_rect.is_some());
        assert_eq!(cli.last_footer_chip_rects.len(), 2);
    });
    h.save_screenshot(&evidence_png!("_form"))
        .expect("render cli_renderer form chrome");
}

#[test]
fn screenshot_edit_commands_header_scope_banner() {
    let mut h = PlexiUiHarness::new_sized(1168.0, 820.0);
    add_focused_pane(&mut h);
    let ws_dir = tempfile::Builder::new()
        .prefix("plexi-ws-")
        .tempdir_in("/tmp")
        .unwrap_or_else(|_| tempfile::tempdir().unwrap());
    let ws_channel_dir = ws_dir.path().join(crate::config::workspace_channel_dir());
    std::fs::create_dir_all(&ws_channel_dir).unwrap();
    let ws_cmds = ws_channel_dir.join("commands.toml");
    std::fs::write(
        &ws_cmds,
        "[commands]\ngs = \"git status -sb\"\n\n[commands.play]\ndescription = \"Open mixer + editor\"\nsteps = [\n  \"pane new --right --name mixer -- echo mix\",\n  \"view open text-editor --split down\",\n]\n",
    )
    .unwrap();

    h.with_app_mut(|app| {
        app.palette_workspace_root = Some(ws_dir.path().to_path_buf());
        app.open_config_in_editor_with_cwd(ws_cmds.clone(), Some(ws_dir.path().to_path_buf()));
    });
    h.run_steps(3);
    assert!(
        h.host_has_label("WORKSPACE"),
        "Edit commands header must render WORKSPACE scope badge"
    );
    assert!(
        h.host_has_label("Switch to global"),
        "Edit commands header must render Switch to global action"
    );
    let out = evidence_png!();
    h.save_screenshot(&out)
        .expect("render edit_commands header scope banner");
    let _ = std::fs::remove_file(&out);
}

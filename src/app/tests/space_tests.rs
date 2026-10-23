//! Nested spaces: one dive/surface model for team runs, agent calls, and
//! pane-created sub-contexts. Cmd+Enter is `toggle_pane_zoom`. Cmd+Esc and
//! Cmd+Backspace are `zoom_out_of_context`. Attention is `pane_navigate`.

use super::super::*;
use crate::host::context::Window;
use crate::host::pane::{AppPane, AppRuntime, Pane, PortalPane};
use crate::host::space_coord::{self, is_machine_id};
use crate::protocol::{AgentState, PaneAgentState};

fn host() -> PlexiApp {
    let ctx = egui::Context::default();
    let frame_tick = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (mut app, _tx) = PlexiApp::new_for_test(ctx, frame_tick);
    app.router.get_mut(0).name = "PLEXI".into();
    app
}

fn named_pane(pane_id: u64, name: &str, state: Option<AgentState>) -> Pane {
    let root = crate::testing::scratch_context_root("space");
    Pane::App(Box::new(AppPane {
        pip_status: None,
        id: pane_id,
        runtime: AppRuntime::Builtin(Box::new(crate::file_browser::FileBrowserApp::new(
            root.clone(),
        ))),
        workspace_root: root,
        permissions: crate::app::permissions::AppPermissions::builtin(),
        manifest_id: format!("space-{pane_id}"),
        name: name.to_string(),
        pane_group: None,
        linked_pane_id: None,
        overlay_replaced: None,
        hidden: false,
        agent: state.map(|state| PaneAgentState {
            pane_id,
            state,
            agent: "pi".to_string(),
            detail: None,
            session_id: None,
        }),
        slots: std::collections::HashMap::new(),
        semantic_state: Default::default(),
    }))
}

fn portal(pane_id: u64, target: u64) -> Pane {
    Pane::Portal(Box::new(PortalPane {
        pane_id,
        target_context_id: target,
        context_state: None,
        hidden: false,
    }))
}

fn push_context(app: &mut PlexiApp, id: u64, parent: u64, name: &str, depth: u32) {
    let root = app.router.get(0).root.clone();
    app.router.push(crate::host::context::Context {
        name: name.to_string().into(),
        root,
        description: None,
        context_id: id,
        parent_id: Some(parent),
        depth,
        parked: false,
    });
}

struct SpaceWindow {
    index: usize,
    tiles: Vec<egui_tiles::TileId>,
}

fn push_window(
    app: &mut PlexiApp,
    ctx_id: u64,
    window_id: u64,
    panes: Vec<(u64, Pane)>,
) -> SpaceWindow {
    let mut tiles = egui_tiles::Tiles::default();
    let mut pane_map = std::collections::HashMap::new();
    let mut tile_ids = Vec::new();
    for (id, pane) in panes {
        tile_ids.push(tiles.insert_pane(id));
        pane_map.insert(id, pane);
    }
    let root = if tile_ids.len() == 1 {
        tile_ids[0]
    } else {
        tiles.insert_horizontal_tile(tile_ids.clone())
    };
    app.windows.push(Window {
        name: format!("space-{ctx_id}"),
        path: app.windows[0].path.clone(),
        tree: egui_tiles::Tree::new(format!("space-{ctx_id}"), root, tiles),
        panes: pane_map,
        focused_pane: tile_ids.first().copied(),
        zoomed_pane: None,
        grid_x: 0,
        grid_y: 0,
        window_id,
        context_id: ctx_id,
    });
    app.context_active_window.insert(ctx_id, window_id);
    SpaceWindow {
        index: app.windows.len() - 1,
        tiles: tile_ids,
    }
}

fn path_of(app: &PlexiApp) -> String {
    space_coord::join_coordinate(&app.space_coordinate())
}

#[test]
fn dive_surface_and_attention_follow_human_coordinates() {
    let mut app = host();
    let root_id = app.router.active().context_id;
    let root_win = app.windows[0].window_id;

    let design_id = 4100;
    let grand_id = 4200;
    let raw_team = "team-017f341796ab4976bee42030d6f5a6c0";
    push_context(&mut app, design_id, root_id, raw_team, 1);
    push_context(&mut app, grand_id, design_id, "agent-tools-42", 2);
    app.seed_team_title_for_test(design_id, "design");

    let writer = 4101;
    let reviewer = 4102;
    let nested_portal = 4103;
    let helper = 4201;
    let design = push_window(
        &mut app,
        design_id,
        4104,
        vec![
            (writer, named_pane(writer, "writer", None)),
            (
                reviewer,
                named_pane(reviewer, "reviewer", Some(AgentState::Blocked)),
            ),
            (nested_portal, portal(nested_portal, grand_id)),
        ],
    );
    let _grand = push_window(
        &mut app,
        grand_id,
        4204,
        vec![(
            helper,
            named_pane(helper, "helper-2", Some(AgentState::NeedsYou)),
        )],
    );

    let root_portal = 4001;
    let mut root_tiles = egui_tiles::Tiles::default();
    let root_portal_tile = root_tiles.insert_pane(root_portal);
    app.windows[0].tree = egui_tiles::Tree::new("root", root_portal_tile, root_tiles);
    app.windows[0]
        .panes
        .insert(root_portal, portal(root_portal, design_id));
    app.windows[0].focused_pane = Some(root_portal_tile);

    assert_eq!(app.space_title(design_id), "design");
    assert_eq!(app.space_title(grand_id), "helper-2");
    assert!(is_machine_id(raw_team));
    assert!(!app.space_title(design_id).contains("team-"));

    // Cmd+Enter on the pane that contains the team space.
    app.toggle_pane_zoom();
    assert_eq!(app.router.active().context_id, design_id);
    assert_eq!(app.fleet_focused_pane(), Some(reviewer));
    assert_eq!(app.router.current_depth(), 1);
    assert_eq!(path_of(&app), "PLEXI \u{203a} design \u{203a} reviewer");
    assert!(app.windows[app.active_window].panes.contains_key(&reviewer));
    assert!(!app.windows[app.active_window]
        .panes
        .contains_key(&root_portal));

    // Cmd+Enter again, on the pane that contains the next space.
    app.windows[design.index].focused_pane = Some(design.tiles[2]);
    app.toggle_pane_zoom();
    assert_eq!(app.router.active().context_id, grand_id);
    assert_eq!(app.fleet_focused_pane(), Some(helper));
    assert_eq!(app.router.current_depth(), 2);
    assert_eq!(path_of(&app), "PLEXI \u{203a} design \u{203a} helper-2");
    for segment in app.space_coordinate() {
        assert!(!is_machine_id(&segment), "{segment}");
    }

    // Cmd+Esc / Cmd+Backspace surface one level at a time.
    assert!(app.zoom_out_of_context());
    assert_eq!(app.router.active().context_id, design_id);
    assert_eq!(app.router.current_depth(), 1);
    assert_eq!(app.windows[app.active_window].window_id, 4104);
    assert_eq!(path_of(&app), "PLEXI \u{203a} design");

    assert!(app.zoom_out_of_context());
    assert_eq!(app.router.active().context_id, root_id);
    assert_eq!(app.windows[app.active_window].window_id, root_win);
    assert_eq!(app.router.current_depth(), 0);
    assert_eq!(path_of(&app), "PLEXI");
    assert!(!app.zoom_out_of_context());

    // Attention (notification click and the next-attention key) is pane_navigate.
    assert!(app.pane_navigate(helper));
    assert_eq!(app.router.active().context_id, grand_id);
    assert_eq!(app.fleet_focused_pane(), Some(helper));
    assert_eq!(app.router.current_depth(), 2);
    assert_eq!(path_of(&app), "PLEXI \u{203a} design \u{203a} helper-2");

    assert!(app.zoom_out_of_context());
    assert_eq!(app.router.active().context_id, design_id);
    assert!(app.zoom_out_of_context());
    assert_eq!(app.router.active().context_id, root_id);
    assert_eq!(path_of(&app), "PLEXI");
}

fn spawn_app(
    app: &mut PlexiApp,
    type_id: &str,
    name: Option<&str>,
    from_pane_id: Option<u64>,
    layout: Option<&str>,
    args: Vec<String>,
) {
    app.handle_pane_ipc_request(crate::protocol::AppRequest::SpawnPane {
        type_id: type_id.to_string(),
        layout: layout.map(str::to_string),
        args,
        from_pane_id,
        request_id: None,
        response_file: None,
        ephemeral: false,
        cwd: None,
        no_focus: false,
        path: None,
        workspace_root: None,
        target_context: None,
        context_name: None,
        name: name.map(str::to_string),
        agent_cmd: None,
        boot_timeout_secs: None,
        worktree: None,
        result_schema: None,
        peer_ancestry: None,
    });
}

fn focus_pane(app: &mut PlexiApp, pane_id: u64) {
    let (win, _) = app.find_pane_in_any_window(pane_id).expect("pane");
    let ctx = app.windows[win].context_id;
    if app.router.active().context_id != ctx {
        let idx = app
            .router
            .position(|c| c.context_id == ctx)
            .expect("context");
        app.switch_workspace(idx);
    }
    let (win, tile) = app.find_pane_in_any_window(pane_id).expect("pane");
    app.active_window = win;
    app.windows[win].focused_pane = Some(tile);
}

fn pane_named(app: &PlexiApp, name: &str) -> u64 {
    app.windows
        .iter()
        .find_map(|win| {
            win.panes.iter().find_map(|(&id, pane)| {
                pane.as_app()
                    .filter(|app_pane| app_pane.name == name)
                    .map(|_| id)
            })
        })
        .unwrap_or_else(|| panic!("no app pane named {name}"))
}

fn mark_blocked(app: &mut PlexiApp, pane_id: u64) {
    let (win, _) = app.find_pane_in_any_window(pane_id).expect("pane");
    let app_pane = app.windows[win]
        .panes
        .get_mut(&pane_id)
        .and_then(|pane| pane.as_app_mut())
        .expect("app pane");
    app_pane.agent = Some(PaneAgentState {
        pane_id,
        state: AgentState::Blocked,
        agent: "git".to_string(),
        detail: Some("need a decision".to_string()),
        session_id: None,
    });
}

/// `app open` from a pane uses the same anchored space as an agent call.
/// A spawn with no launching pane stays a sibling in the current context.
#[test]
fn app_open_from_a_pane_anchors_beside_the_launcher() {
    let mut app = host();
    let root_id = app.router.active().context_id;
    let (launcher_tile, launcher) = app.add_test_pane();
    app.windows[0].focused_pane = Some(launcher_tile);
    let contexts = app.router.len();

    spawn_app(&mut app, "agents", None, None, None, vec![]);
    assert_eq!(
        app.router.len(),
        contexts,
        "an open with no launching pane stays put"
    );
    assert_eq!(app.router.active().context_id, root_id);
    assert!(app.conjoined_contexts.is_empty());
    assert!(
        app.windows.iter().any(|win| {
            win.context_id == root_id
                && win.panes.values().any(|pane| {
                    pane.as_app()
                        .is_some_and(|app_pane| app_pane.manifest_id == "agents")
                })
        }),
        "the detached open is a sibling of the launcher"
    );

    focus_pane(&mut app, launcher);
    spawn_app(
        &mut app,
        "cli-renderer",
        Some("git"),
        Some(launcher),
        None,
        vec!["/tmp/plexi-missing-git-descriptor.json".into()],
    );
    let git = pane_named(&app, "git");
    let git_ctx = app
        .windows
        .iter()
        .find(|win| win.panes.contains_key(&git))
        .map(|win| win.context_id)
        .unwrap();
    assert_ne!(git_ctx, root_id);
    assert_eq!(app.conjoined_contexts.get(&git_ctx), Some(&launcher));
    assert_eq!(app.router.active().context_id, git_ctx);
    assert_eq!(app.space_title(git_ctx), "git");
    assert_eq!(path_of(&app), "PLEXI \u{203a} git");
    for segment in app.space_coordinate() {
        assert!(!is_machine_id(&segment), "{segment}");
    }
    assert!(!app.windows[app.active_window].panes.contains_key(&launcher));
    let git_pane = app.windows[app.active_window]
        .panes
        .get(&git)
        .and_then(|pane| pane.as_app())
        .unwrap();
    assert_eq!(git_pane.manifest_id, "cli-renderer");
    assert_eq!(git_pane.name, "git");

    assert!(app.zoom_out_of_context());
    assert_eq!(app.router.active().context_id, root_id);
    assert_eq!(app.router.current_depth(), 0);
    assert_eq!(app.fleet_focused_pane(), Some(launcher));
    app.toggle_pane_zoom();
    assert_eq!(
        app.router.active().context_id,
        git_ctx,
        "Cmd+Enter dives back into git"
    );
    assert_eq!(path_of(&app), "PLEXI \u{203a} git");

    assert!(app.zoom_out_of_context());
    mark_blocked(&mut app, git);
    let face = app
        .space_faces_for(&std::collections::HashMap::new())
        .get(&launcher)
        .cloned()
        .expect("launcher summary");
    assert_eq!(face.title, "git");
    assert_eq!(face.urgent, crate::host::space_coord::UrgentKind::Blocked);
    assert!(app.pane_navigate(git));
    assert_eq!(app.fleet_focused_pane(), Some(git));
    assert_eq!(app.router.active().context_id, git_ctx);
    assert_eq!(path_of(&app), "PLEXI \u{203a} git");
    assert_eq!(app.router.current_depth(), 1);

    {
        let _gate =
            crate::release::set_test_experiments(&[crate::release::ReleaseFeature::McpClient]);
        spawn_app(
            &mut app,
            "mcp-renderer",
            Some("mcp: github"),
            Some(launcher),
            None,
            vec!["github".into()],
        );
    }
    let mcp = pane_named(&app, "mcp: github");
    let mcp_ctx = app
        .windows
        .iter()
        .find(|win| win.panes.contains_key(&mcp))
        .map(|win| win.context_id)
        .unwrap();
    assert_ne!(mcp_ctx, git_ctx);
    assert_eq!(app.conjoined_contexts.get(&mcp_ctx), Some(&launcher));
    assert_eq!(app.space_title(mcp_ctx), "mcp: github");
    assert!(path_of(&app).contains("mcp: github"), "{}", path_of(&app));
    assert!(!path_of(&app).contains("team-"), "{}", path_of(&app));

    spawn_app(&mut app, "file_browser", None, Some(launcher), None, vec![]);
    let browser_ctx = app.router.active().context_id;
    assert_ne!(browser_ctx, root_id);
    assert_ne!(browser_ctx, git_ctx);
    assert_eq!(app.conjoined_contexts.get(&browser_ctx), Some(&launcher));
    assert_eq!(app.space_title(browser_ctx), "file_browser");
    assert!(path_of(&app).contains("file_browser"), "{}", path_of(&app));

    let before = app.router.len();
    spawn_app(
        &mut app,
        "cli-renderer",
        Some("git"),
        Some(launcher),
        None,
        vec!["/tmp/plexi-missing-git-descriptor.json".into()],
    );
    assert_eq!(
        app.router.len(),
        before,
        "the same app name reuses the anchored space"
    );
    assert_eq!(app.router.active().context_id, git_ctx);
    assert_eq!(
        app.anchored_spaces(launcher).first().copied(),
        Some(git_ctx)
    );

    focus_pane(&mut app, launcher);
    let rooted = app.router.len();
    spawn_app(
        &mut app,
        "text-editor",
        Some("note"),
        Some(launcher),
        Some("split_h"),
        vec![],
    );
    assert_eq!(
        app.router.len(),
        rooted,
        "an explicit split stays in the launching context"
    );
    assert_eq!(app.router.active().context_id, root_id);
    assert!(app.windows.iter().any(|win| {
        win.context_id == root_id
            && win.panes.values().any(|pane| {
                pane.as_app()
                    .is_some_and(|app_pane| app_pane.manifest_id == "text-editor")
            })
    }));
}

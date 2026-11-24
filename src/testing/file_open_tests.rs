//! `file.open` from a pane: caller authentication, path resolution, handler
//! choice, placement, focus and the receipt (`src/app/file_open.rs`).

use std::path::{Path, PathBuf};

use egui_tiles::{Container, LinearDir, Tile};

use crate::host::pane::ViewRuntime;
use crate::broker::caller::{AncestryEnd, PeerHop, PeerWitness};
use crate::protocol::{HostRequest, OpenPlacement};
use crate::spatial::tiling::PaneId;
use crate::testing::HostHarness;

/// A harness with one terminal (the caller) and its pid.
fn caller() -> (HostHarness, PaneId, u32) {
    let mut h = HostHarness::new();
    let pane = h.add_focused_terminal();
    let pid = h.shell_pid(pane);
    (h, pane, pid)
}

struct Open<'a> {
    path: &'a str,
    cwd: Option<PathBuf>,
    from: Option<u64>,
    placement: Option<OpenPlacement>,
    no_focus: bool,
}

impl<'a> Open<'a> {
    /// `_caller` is who sends it; nothing on the wire names it (stint 1285).
    fn new(path: &'a str, cwd: &Path, _caller: PaneId) -> Self {
        Self {
            path,
            cwd: Some(cwd.to_path_buf()),
            from: None,
            placement: None,
            no_focus: false,
        }
    }

    /// Send over a socket whose peer walk reached the OS root.
    fn send(self, h: &mut HostHarness, ancestry: &[u32]) -> serde_json::Value {
        self.send_walk(h, walk(ancestry, AncestryEnd::ReachedRoot))
    }

    /// Send with an explicit host-stamped peer walk (`None`: no credential).
    fn send_walk(self, h: &mut HostHarness, peer: Option<PeerWitness>) -> serde_json::Value {
        let token = h
            .workspace_root()
            .join(format!("open-{}.json", uuid::Uuid::new_v4()));
        let request = HostRequest::FileOpen {
            path: self.path.to_string(),
            cwd: self.cwd.map(|cwd| cwd.display().to_string()),
            from_pane_id: self.from,
            placement: self.placement,
            no_focus: self.no_focus,
            response_file: token.to_string_lossy().into_owned(),
        };
        let line = serde_json::to_string(&request).unwrap();
        let (mut server, client) = std::os::unix::net::UnixStream::pair().unwrap();
        let token = token.to_string_lossy().into_owned();
        crate::rpc::register_client_stream(token.clone(), client);
        crate::app::handle_socket_line_in(
            &crate::config::config_dir(),
            &line,
            &h.ipc_tx,
            peer.as_ref(),
            &mut server,
        );
        drop(server);
        h.run_frames(1);
        let body =
            crate::rpc::poll_string(&token, Some(std::time::Duration::from_secs(2))).unwrap();
        serde_json::from_str(&body).unwrap()
    }
}

fn pane_count(h: &HostHarness) -> usize {
    h.app.windows.iter().map(|w| w.panes.len()).sum()
}

fn window_of(h: &HostHarness, pane: PaneId) -> usize {
    h.app
        .find_pane_in_any_window(pane)
        .map(|(window, _)| window)
        .expect("pane is live")
}

/// The opened editor's path and the text it loaded.
fn editor_buffer(h: &mut HostHarness, pane: PaneId) -> (PathBuf, String) {
    let window = window_of(h, pane);
    let view = h.app.windows[window]
        .panes
        .get_mut(&pane)
        .and_then(|p| p.as_view_mut())
        .expect("view pane");
    let ViewRuntime::Builtin(app) = &mut view.runtime else {
        panic!("editor is a builtin view");
    };
    let editor = app
        .as_any_mut()
        .downcast_mut::<crate::app::text_editor_view::TextEditorView>()
        .expect("text editor");
    let buffer = editor.open_buffer();
    (PathBuf::from(buffer.path), buffer.text)
}

/// Whether `right` is the next tile after `left` in a side-by-side
/// container. Each pane sits in its own tab group, so both are lifted to the
/// child of their shared linear container first.
fn is_right_sibling(h: &HostHarness, left: PaneId, right: PaneId) -> bool {
    let tiles = &h.app.windows[window_of(h, left)].tree.tiles;
    let lift = |pane| {
        let mut tile = tiles.find_pane(&pane)?;
        loop {
            let parent = tiles.parent_of(tile)?;
            if let Some(Tile::Container(Container::Linear(linear))) = tiles.get(parent) {
                return Some((parent, tile, linear));
            }
            tile = parent;
        }
    };
    let (Some((lp, l, linear)), Some((rp, r, _))) = (lift(left), lift(right)) else {
        return false;
    };
    let at = |tile| linear.children.iter().position(|c| *c == tile);
    lp == rp
        && linear.dir == LinearDir::Horizontal
        && matches!((at(l), at(r)), (Some(a), Some(b)) if a + 1 == b)
}

fn copy_repo_file(name: &str, into: &Path) -> String {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join(name);
    std::fs::copy(&source, into.join(name)).expect("copy fixture");
    std::fs::read_to_string(source).expect("fixture is text")
}

#[test]
fn plexi_open_justfile_and_readme_open_the_editor_right_of_the_caller() {
    let (mut h, caller, pid) = caller();
    let root = h.workspace_root();
    let justfile = copy_repo_file("justfile", &root);
    let readme = copy_repo_file("README.md", &root);

    for (name, contents, matched_by) in [
        ("justfile", &justfile, "content"),
        ("README.md", &readme, "extension"),
    ] {
        let reply = Open::new(name, &root, caller).send(&mut h, &[pid]);
        assert_eq!(reply["status"], "opened", "{reply}");
        assert_eq!(reply["handler"], "text-editor", "{reply}");
        assert_eq!(reply["matched_by"], matched_by, "{reply}");
        assert_eq!(reply["placement"], "right", "{reply}");
        assert_eq!(reply["focused"], true, "{reply}");
        let expected = root.join(name).canonicalize().unwrap();
        assert_eq!(reply["path"], expected.display().to_string(), "{reply}");
        assert_eq!(
            reply["context_id"],
            h.app.windows[window_of(&h, caller)].context_id,
            "{reply}"
        );
        let pane = reply["pane_id"].as_u64().expect("pane id");
        let (path, text) = editor_buffer(&mut h, pane);
        assert_eq!(path, expected);
        assert_eq!(&text, contents, "{name} loaded its own contents");
        assert!(
            is_right_sibling(&h, caller, pane),
            "{name} opens right of the caller"
        );
    }
}

#[test]
fn a_relative_path_resolves_in_the_callers_cwd() {
    let (mut h, caller, pid) = caller();
    let root = h.workspace_root();
    std::fs::create_dir(root.join("docs")).unwrap();
    std::fs::write(root.join("README.md"), "root readme").unwrap();
    std::fs::write(root.join("docs/README.md"), "docs readme").unwrap();

    let reply = Open::new("README.md", &root.join("docs"), caller).send(&mut h, &[pid]);
    let pane = reply["pane_id"].as_u64().expect("pane id");
    assert_eq!(editor_buffer(&mut h, pane).1, "docs readme", "{reply}");
}

#[test]
fn a_file_named_like_a_view_opens_the_file_never_the_view() {
    let (mut h, caller, pid) = caller();
    let root = h.workspace_root();
    std::fs::write(root.join("file_browser"), "just a file\n").unwrap();

    let reply = Open::new("file_browser", &root, caller).send(&mut h, &[pid]);
    assert_eq!(reply["handler"], "text-editor", "{reply}");
    let pane = reply["pane_id"].as_u64().expect("pane id");
    assert_eq!(editor_buffer(&mut h, pane).1, "just a file\n");

    let before = pane_count(&h);
    let reply = Open::new("agents", &root, caller).send(&mut h, &[pid]);
    assert_eq!(reply["error_code"], "not_found", "{reply}");
    assert_eq!(pane_count(&h), before, "a missing file launches nothing");
}

#[test]
fn a_leading_dash_name_opens_as_a_file() {
    let (mut h, caller, pid) = caller();
    let root = h.workspace_root();
    std::fs::write(root.join("-notes.txt"), "dash").unwrap();
    let reply = Open::new("-notes.txt", &root, caller).send(&mut h, &[pid]);
    let pane = reply["pane_id"].as_u64().expect("pane id");
    assert_eq!(editor_buffer(&mut h, pane).1, "dash");
}

#[test]
fn missing_binary_and_unresolvable_targets_are_typed_and_open_nothing() {
    let (mut h, caller, pid) = caller();
    let root = h.workspace_root();
    std::fs::write(root.join("blob"), [0u8, 159, 146, 150]).unwrap();
    let before = pane_count(&h);

    let cases = [
        (Open::new("missing.md", &root, caller), "not_found"),
        (Open::new("blob", &root, caller), "no_handler"),
        (
            Open {
                cwd: None,
                ..Open::new("README.md", &root, caller)
            },
            "cwd_unavailable",
        ),
    ];
    for (open, code) in cases {
        let reply = open.send(&mut h, &[pid]);
        assert_eq!(reply["error_code"], code, "{reply}");
        assert!(reply["error"].is_string(), "{reply}");
        assert!(reply.get("status").is_none(), "{reply}");
    }
    assert_eq!(pane_count(&h), before);
}

#[test]
fn a_caller_outside_every_pane_without_an_anchor_places_nothing() {
    let (mut h, caller, _pid) = caller();
    let root = h.workspace_root();
    std::fs::write(root.join("a.txt"), "a").unwrap();
    let before = pane_count(&h);
    // No pane is named on the wire; the peer runs in none, so there is
    // nothing to place beside (stint 1285).
    let reply = Open::new("a.txt", &root, caller).send(&mut h, &[999_999]);
    assert_eq!(reply["error_code"], "no_anchor", "{reply}");
    assert_eq!(pane_count(&h), before);
}

#[test]
fn reopening_an_open_file_reuses_its_editor_and_refuses_a_second_copy() {
    let (mut h, caller, pid) = caller();
    let root = h.workspace_root();
    std::fs::write(root.join("a.txt"), "a").unwrap();
    let first = Open::new("a.txt", &root, caller).send(&mut h, &[pid]);
    let pane = first["pane_id"].as_u64().unwrap();

    let again = Open::new("a.txt", &root, caller).send(&mut h, &[pid]);
    assert_eq!(again["status"], "reused", "{again}");
    assert_eq!(again["pane_id"], pane, "{again}");

    let before = pane_count(&h);
    let placed = Open {
        placement: Some(OpenPlacement::Down),
        ..Open::new("a.txt", &root, caller)
    }
    .send(&mut h, &[pid]);
    assert_eq!(placed["error_code"], "placement_conflict", "{placed}");
    assert_eq!(placed["pane_id"], pane, "{placed}");
    assert_eq!(pane_count(&h), before);
}

#[test]
fn no_focus_keeps_the_callers_focus() {
    let (mut h, caller, pid) = caller();
    let root = h.workspace_root();
    std::fs::write(root.join("a.txt"), "a").unwrap();
    let reply = Open {
        no_focus: true,
        ..Open::new("a.txt", &root, caller)
    }
    .send(&mut h, &[pid]);
    assert_eq!(reply["status"], "opened", "{reply}");
    assert_eq!(reply["focused"], false, "{reply}");
    let window = &h.app.windows[h.app.active_window];
    let focused = window
        .focused_pane
        .and_then(|tile| window.tree.tiles.get(tile));
    assert!(
        matches!(focused, Some(Tile::Pane(id)) if *id == caller),
        "focus stays on the caller"
    );
}

#[test]
fn tab_and_window_placements_are_honored() {
    let (mut h, caller, pid) = caller();
    let root = h.workspace_root();
    std::fs::write(root.join("tab.txt"), "t").unwrap();
    std::fs::write(root.join("window.txt"), "w").unwrap();

    let reply = Open {
        placement: Some(OpenPlacement::Tab),
        ..Open::new("tab.txt", &root, caller)
    }
    .send(&mut h, &[pid]);
    assert_eq!(reply["placement"], "tab", "{reply}");
    let pane = reply["pane_id"].as_u64().unwrap();
    let tiles = &h.app.windows[window_of(&h, caller)].tree.tiles;
    let parent_of = |id| tiles.find_pane(&id).and_then(|tile| tiles.parent_of(tile));
    let parent = parent_of(pane).expect("tab parent");
    assert_eq!(
        parent_of(caller),
        Some(parent),
        "tab joins the caller's group"
    );
    assert!(matches!(
        tiles.get(parent),
        Some(Tile::Container(Container::Tabs(_)))
    ));

    let windows = h.app.windows.len();
    let reply = Open {
        placement: Some(OpenPlacement::Window),
        ..Open::new("window.txt", &root, caller)
    }
    .send(&mut h, &[pid]);
    assert_eq!(reply["placement"], "window", "{reply}");
    let pane = reply["pane_id"].as_u64().unwrap();
    assert_eq!(h.app.windows.len(), windows + 1);
    let window = window_of(&h, pane);
    assert_ne!(window, window_of(&h, caller));
    assert_eq!(
        h.app.windows[window].context_id,
        h.app.windows[window_of(&h, caller)].context_id
    );
    assert_eq!(reply["focused"], true, "{reply}");
}

#[test]
fn a_directory_opens_explorer_rooted_there() {
    let (mut h, caller, pid) = caller();
    let root = h.workspace_root();
    std::fs::create_dir(root.join("src")).unwrap();
    let reply = Open::new("src", &root, caller).send(&mut h, &[pid]);
    assert_eq!(reply["handler"], "file_browser", "{reply}");
    assert_eq!(reply["matched_by"], "directory", "{reply}");
    let pane = reply["pane_id"].as_u64().unwrap();
    let state = h.app.windows[window_of(&h, pane)].panes[&pane]
        .as_view()
        .and_then(|view| view.runtime.serialize_state())
        .expect("explorer state");
    assert_eq!(
        Path::new(state["cwd"].as_str().unwrap()),
        root.join("src").canonicalize().unwrap()
    );
}

/// The Explorer's `OpenArtifact` seams: the sender's workspace root is found
/// in whichever window holds the sender, and the validated root-joined path
/// is the one that opens.
#[test]
fn open_artifact_checks_and_opens_the_root_joined_path_from_a_background_window() {
    let mut h = HostHarness::new();
    let sender = h.add_test_pane();
    let root = h.workspace_root();
    std::fs::write(root.join("inside.md"), "inside").unwrap();
    let context_id = h.app.windows[0].context_id;
    let launch = h.app.test_terminal_launch(context_id, None, None);
    h.app.create_page_at(1, 0, context_id, launch);
    assert_ne!(
        h.app.active_window, 0,
        "the sender's window is in the background"
    );

    let before = pane_count(&h);
    h.app.dispatch_open_artifact(
        sender,
        "../escape.md".into(),
        crate::protocol::ArtifactOpenMode::OpenInPane,
    );
    assert_eq!(pane_count(&h), before, "a path outside the root is refused");

    h.app.dispatch_open_artifact(
        sender,
        "inside.md".into(),
        crate::protocol::ArtifactOpenMode::OpenInPane,
    );
    let editor = h
        .app
        .windows
        .iter()
        .flat_map(|w| w.panes.iter())
        .find_map(|(id, pane)| {
            pane.as_view()
                .is_some_and(|v| v.runtime.type_id() == "text-editor")
                .then_some(*id)
        })
        .expect("inside.md opened");
    let (path, text) = editor_buffer(&mut h, editor);
    assert_eq!(
        path,
        root.join("inside.md").canonicalize().unwrap(),
        "the checked canonical path opens"
    );
    assert_eq!(text, "inside");
}

// ── Round-1 review regressions ───────────────────────────────────────────────

/// A second context with one view pane, rooted at its own temp dir.
fn second_context(h: &mut HostHarness) -> (tempfile::TempDir, PaneId) {
    let root = tempfile::tempdir().unwrap();
    let context_id = h.app.windows[0].context_id + 1_000;
    let window_id = h.app.windows.iter().map(|w| w.window_id).max().unwrap() + 1;
    h.app.windows.push(crate::host::context::Window {
        name: "B".into(),
        path: root.path().to_path_buf(),
        tree: egui_tiles::Tree::empty("file_open_ctx_b"),
        panes: Default::default(),
        focused_pane: None,
        zoomed_pane: None,
        grid_x: 0,
        grid_y: 1,
        window_id,
        context_id,
        tree_history: Default::default(),
    });
    h.app.router.push(crate::host::context::Context {
        name: "Context B".into(),
        root: root.path().to_path_buf(),
        description: None,
        context_id,
        parent_id: None,
        depth: 0,
        parked: false,
    });
    let window = h.app.windows.len() - 1;
    let (_, pane) = h.app.add_view_pane_in_window(window, "test");
    (root, pane)
}

/// Register a plugin in the caller context's registry from a real manifest.
fn install_plugin(h: &mut HostHarness, id: &str, extra: &str) -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    let global = tmp.path().join("global-apps");
    let dir = global.join(id);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("manifest.toml"),
        format!(
            "schema_version = 2\n\n{extra}\n[plugin]\nid = \"{id}\"\nname = \"{id}\"\nversion = \"0.0.1\"\n\n[runtime]\nkind = \"python\"\nentry = \"main.py\"\n\n[[contributes.views]]\nid = \"{id}.main\"\ndefault = true\n"
        ),
    )
    .unwrap();
    std::fs::write(dir.join("main.py"), "#!/usr/bin/env python3\n").unwrap();
    let registry = crate::app::registry::PluginRegistry::load_with_global(tmp.path(), &global);
    assert!(registry.get(id).is_some(), "{id} loads");
    h.trust_plugin_folder(&global);
    let root = h.app.router.get(0).root.clone();
    h.app.registries.set_view_for_test(&root, registry);
    tmp
}

/// Finding 1: OpenArtifact authorizes the canonical path and opens exactly
/// that file. `sub/../x.md` with `sub -> a/b` names `a/x.md` on disk, not the
/// lexical `x.md`.
#[test]
fn open_artifact_opens_the_canonical_file_it_checked() {
    let mut h = HostHarness::new();
    let sender = h.add_test_pane();
    let root = h.workspace_root();
    std::fs::create_dir_all(root.join("a/b")).unwrap();
    std::fs::write(root.join("a/x.md"), "canonical").unwrap();
    std::fs::write(root.join("x.md"), "lexical").unwrap();
    std::os::unix::fs::symlink(root.join("a/b"), root.join("sub")).unwrap();

    h.app.dispatch_open_artifact(
        sender,
        "sub/../x.md".into(),
        crate::protocol::ArtifactOpenMode::OpenInPane,
    );
    let editor = h
        .app
        .windows
        .iter()
        .flat_map(|w| w.panes.iter())
        .find_map(|(id, pane)| {
            pane.as_view()
                .is_some_and(|v| v.runtime.type_id() == "text-editor")
                .then_some(*id)
        })
        .expect("opened");
    let (path, text) = editor_buffer(&mut h, editor);
    assert_eq!(text, "canonical");
    assert_eq!(path, root.join("a/x.md").canonicalize().unwrap());
}

/// Finding 2: the canonical target must be inside the calling pane's
/// authority before anything reads it.
#[test]
fn a_target_outside_the_callers_authority_is_refused_before_it_is_read() {
    let (mut h, caller, pid) = caller();
    let root = h.workspace_root();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("secret.txt"), "s").unwrap();
    std::os::unix::fs::symlink(outside.path().join("secret.txt"), root.join("link.txt")).unwrap();
    std::fs::write(root.join("ok.txt"), "ok").unwrap();
    let before = pane_count(&h);
    for path in [outside.path().join("secret.txt"), root.join("link.txt")] {
        let reply = Open::new(path.to_str().unwrap(), &root, caller).send(&mut h, &[pid]);
        assert_eq!(reply["error_code"], "outside_authority", "{reply}");
    }
    assert_eq!(pane_count(&h), before);

    // An agent pane loses a nested context even under its own root.
    h.app.windows[0]
        .panes
        .get_mut(&caller)
        .and_then(|p| p.as_terminal_mut())
        .unwrap()
        .agent_harness = Some("pi".to_string());
    let nested = root.join("nested");
    std::fs::create_dir(&nested).unwrap();
    std::fs::write(nested.join("n.txt"), "n").unwrap();
    h.app.router.push(crate::host::context::Context {
        name: "Nested".into(),
        root: nested.clone(),
        description: None,
        context_id: h.app.windows[0].context_id + 2_000,
        parent_id: None,
        depth: 1,
        parked: false,
    });
    let reply = Open::new("nested/n.txt", &root, caller).send(&mut h, &[pid]);
    assert_eq!(reply["error_code"], "outside_authority", "{reply}");
    let reply = Open::new("ok.txt", &root, caller).send(&mut h, &[pid]);
    assert_eq!(
        reply["status"], "opened",
        "the agent's own root still opens: {reply}"
    );
}

/// Finding 3: an unidentified peer, or a terminal hidden under an overlay,
/// gets no outside-pane placement privilege. A verified external caller is
/// the one explicit outside case, bounded by the anchor context.
#[test]
fn only_a_verified_external_caller_places_into_another_context() {
    let (mut h, caller, pid) = caller();
    let (root_b, anchor_b) = second_context(&mut h);
    std::fs::write(root_b.path().join("b.txt"), "b").unwrap();
    let in_b = |_: Option<u64>| Open {
        from: Some(anchor_b),
        ..Open::new("b.txt", root_b.path(), caller)
    };
    let before = pane_count(&h);

    let reply = in_b(None).send(&mut h, &[]);
    assert_eq!(reply["error_code"], "caller_unidentified", "{reply}");

    // Cover the caller's terminal with a view overlay; its process still
    // belongs to that pane.
    h.focus_pane(caller);
    h.app
        .launch_view_by_id_with_layout("file_browser", Some("overlay".into()), &[], None)
        .unwrap();
    assert!(
        h.app.windows[0].panes[&caller].as_terminal().is_none(),
        "covered"
    );
    let reply = in_b(None).send(&mut h, &[pid]);
    assert_eq!(reply["error_code"], "anchor_outside_context", "{reply}");
    assert_eq!(pane_count(&h), before);

    // A verified process outside every pane may place into B, inside B's root.
    let reply = in_b(None).send(&mut h, &[999_999]);
    assert_eq!(reply["status"], "opened", "{reply}");
    let a_txt = h.workspace_root().join("a.txt");
    std::fs::write(&a_txt, "a").unwrap();
    let outside = Open {
        from: Some(anchor_b),
        ..Open::new(a_txt.to_str().unwrap(), root_b.path(), caller)
    };
    let reply = outside.send(&mut h, &[999_999]);
    assert_eq!(reply["error_code"], "outside_authority", "{reply}");
}

/// Finding 4 at the operation boundary: a FIFO with a text extension never
/// reaches the editor.
#[test]
fn a_fifo_named_like_text_is_refused() {
    let (mut h, caller, pid) = caller();
    let root = h.workspace_root();
    let status = std::process::Command::new("/usr/bin/mkfifo")
        .arg(root.join("pipe.txt"))
        .status()
        .unwrap();
    assert!(status.success());
    let reply = Open::new("pipe.txt", &root, caller).send(&mut h, &[pid]);
    assert_eq!(reply["error_code"], "special_file", "{reply}");
}

/// Finding 5: a plugin whose `on_launch` policy would focus an existing
/// instance still gets a fresh instance carrying the requested file.
#[test]
fn a_focus_existing_handler_still_receives_the_file() {
    let (mut h, caller, pid) = caller();
    let root = h.workspace_root();
    let _plugin = install_plugin(
        &mut h,
        "image-viewer",
        "[launch]\non_launch = \"focus_existing\"\n",
    );
    std::fs::write(root.join("one.png"), b"png").unwrap();
    std::fs::write(root.join("two.png"), b"png").unwrap();
    let first = Open::new("one.png", &root, caller).send(&mut h, &[pid]);
    assert_eq!(first["status"], "opened", "{first}");
    let second = Open::new("two.png", &root, caller).send(&mut h, &[pid]);
    assert_eq!(second["status"], "opened", "{second}");
    let pane = second["pane_id"].as_u64().unwrap();
    assert_ne!(Some(pane), first["pane_id"].as_u64());
    let state = h.app.windows[window_of(&h, pane)].panes[&pane]
        .as_view()
        .and_then(|v| v.runtime.serialize_state())
        .expect("viewer state");
    assert_eq!(
        Path::new(state["path"].as_str().unwrap()),
        root.join("two.png").canonicalize().unwrap()
    );
}

/// Finding 5: a failed capability preflight is a refusal, leaves no tile and,
/// with `--window`, no empty window.
#[test]
fn a_failed_preflight_is_reported_and_leaves_nothing_behind() {
    let (mut h, caller, pid) = caller();
    let root = h.workspace_root();
    h.app.config.ai = None;
    let _plugin = install_plugin(
        &mut h,
        "aiview",
        "[plugin.capabilities]\ncapabilities = [\"ai.query\"]\nfile_types = [\"zz\"]\n",
    );
    std::fs::write(root.join("doc.zz"), "zz").unwrap();
    let (panes, windows) = (pane_count(&h), h.app.windows.len());
    for placement in [None, Some(OpenPlacement::Window)] {
        let reply = Open {
            placement,
            ..Open::new("doc.zz", &root, caller)
        }
        .send(&mut h, &[pid]);
        assert_eq!(reply["error_code"], "handler_unavailable", "{reply}");
        assert_eq!(pane_count(&h), panes, "{placement:?}");
        assert_eq!(h.app.windows.len(), windows, "{placement:?}");
    }
}

/// Finding 5: an OS opener that fails to start is a refusal.
#[test]
fn an_os_opener_failure_is_reported() {
    let (mut h, caller, pid) = caller();
    let root = h.workspace_root();
    std::fs::write(root.join("a.txt"), "a").unwrap();
    let mut handlers = std::collections::HashMap::new();
    handlers.insert("txt".to_string(), "os".to_string());
    h.app.config.file_handlers = Some(handlers);

    crate::app::canvas_bindings::set_test_opener(Some("/nonexistent/plexi-opener"));
    let reply = Open::new("a.txt", &root, caller).send(&mut h, &[pid]);
    assert_eq!(reply["error_code"], "external_open_failed", "{reply}");

    crate::app::canvas_bindings::set_test_opener(Some("/usr/bin/true"));
    let reply = Open::new("a.txt", &root, caller).send(&mut h, &[pid]);
    assert_eq!(reply["status"], "external_dispatched", "{reply}");
    assert!(reply["pane_id"].is_null(), "{reply}");
}

/// Finding 6: `--tab --no-focus` leaves the caller's tab showing.
#[test]
fn a_background_tab_stays_hidden() {
    let (mut h, caller, pid) = caller();
    let root = h.workspace_root();
    std::fs::write(root.join("a.txt"), "a").unwrap();
    let reply = Open {
        placement: Some(OpenPlacement::Tab),
        no_focus: true,
        ..Open::new("a.txt", &root, caller)
    }
    .send(&mut h, &[pid]);
    assert_eq!(reply["focused"], false, "{reply}");
    let tiles = &h.app.windows[0].tree.tiles;
    let caller_tile = tiles.find_pane(&caller).unwrap();
    let group = tiles.parent_of(caller_tile).unwrap();
    let Some(Tile::Container(Container::Tabs(tabs))) = tiles.get(group) else {
        panic!("caller sits in a tab group");
    };
    assert_eq!(
        tabs.active,
        Some(caller_tile),
        "the caller's tab stays shown"
    );
}

/// Finding 6: a view opened as a tab records focus history, so Back returns
/// to the pane it was opened from.
#[test]
fn back_after_a_view_tab_returns_to_the_origin() {
    let (mut h, caller, _pid) = caller();
    h.focus_pane(caller);
    let origin = h.app.windows[0].focused_pane;
    h.app
        .launch_view_by_id_with_layout("file_browser", Some("tab".into()), &[], None)
        .unwrap();
    assert_ne!(h.app.windows[0].focused_pane, origin);
    h.app.step_focus_history_back();
    assert_eq!(h.app.windows[0].focused_pane, origin);
}

/// Finding 7: when a manifest handler fails to launch, the Explorer keeps
/// going down the cascade to the built-in editor instead of leaving Plexi.
#[test]
fn explorer_falls_through_a_failed_manifest_handler_to_the_editor() {
    let mut h = HostHarness::new();
    let sender = h.add_test_pane();
    let root = h.workspace_root();
    crate::app::canvas_bindings::set_test_opener(Some("/usr/bin/true"));
    // `video-player` is release-gated off, so launching it fails.
    let _plugin = install_plugin(
        &mut h,
        "video-player",
        "[plugin.capabilities]\nfile_types = [\"md\"]\n",
    );
    std::fs::write(root.join("notes.md"), "# notes").unwrap();
    h.app.dispatch_open_artifact(
        sender,
        "notes.md".into(),
        crate::protocol::ArtifactOpenMode::OpenInPane,
    );
    let editor = h
        .app
        .windows
        .iter()
        .flat_map(|w| w.panes.iter())
        .find_map(|(id, pane)| {
            pane.as_view()
                .is_some_and(|v| v.runtime.type_id() == "text-editor")
                .then_some(*id)
        })
        .expect("the editor opened notes.md");
    assert_eq!(editor_buffer(&mut h, editor).1, "# notes");
}

// ── Round-2 review regressions ───────────────────────────────────────────────

/// The editor opens on an empty buffer when its file does not load (here,
/// invalid UTF-8 behind a text extension). That is a refusal, not `opened`,
/// and the empty editor is closed.
#[test]
fn an_editor_that_cannot_load_the_file_is_a_refusal() {
    let (mut h, caller, pid) = caller();
    let root = h.workspace_root();
    std::fs::write(root.join("latin1.txt"), [b'c', b'a', b'f', 0xe9]).unwrap();
    let before = pane_count(&h);
    let reply = Open::new("latin1.txt", &root, caller).send(&mut h, &[pid]);
    assert_eq!(reply["error_code"], "load_failed", "{reply}");
    assert_eq!(pane_count(&h), before, "the empty editor is closed");
}

/// `--tab --no-focus` with keyboard focus in another split: the anchor's tab
/// group keeps showing the anchor, and focus stays where it was.
#[test]
fn a_background_tab_stays_hidden_when_focus_is_in_another_split() {
    let (mut h, caller, pid) = caller();
    let root = h.workspace_root();
    std::fs::write(root.join("a.txt"), "a").unwrap();
    let other = *h.app.windows[0]
        .panes
        .keys()
        .find(|id| **id != caller)
        .expect("the harness's first pane");
    h.focus_pane(other);
    let focused_before = h.app.windows[0].focused_pane;

    let reply = Open {
        placement: Some(OpenPlacement::Tab),
        no_focus: true,
        ..Open::new("a.txt", &root, caller)
    }
    .send(&mut h, &[pid]);
    assert_eq!(reply["status"], "opened", "{reply}");
    assert_eq!(reply["focused"], false, "{reply}");
    assert_eq!(h.app.windows[0].focused_pane, focused_before);
    let tiles = &h.app.windows[0].tree.tiles;
    let caller_tile = tiles.find_pane(&caller).unwrap();
    let group = tiles.parent_of(caller_tile).unwrap();
    let Some(Tile::Container(Container::Tabs(tabs))) = tiles.get(group) else {
        panic!("caller sits in a tab group");
    };
    assert_eq!(
        tabs.active,
        Some(caller_tile),
        "the anchor's tab stays shown"
    );
}

// ── Approved caller-identity design: slice 1, caller evidence ────────────────

/// A kernel-shaped walk over `pids`: a live pid carries its real start, a
/// made-up one a start no live process has.
fn walk(pids: &[u32], end: AncestryEnd) -> Option<PeerWitness> {
    Some(PeerWitness {
        pid: pids.first().copied().unwrap_or_default(),
        hops: pids
            .iter()
            .map(|&pid| PeerHop {
                pid,
                start: crate::platform::process::process_info(pid)
                    .map_or(crate::platform::process::ProcStart::for_test(0), |info| {
                        info.start
                    }),
            })
            .collect(),
        end,
    })
}

/// The same unmatched pid prefix is an external caller only when the walk
/// reached the OS root; any interrupted ending, or no credential at all,
/// identifies nobody. A match before an interruption still names the pane.
#[test]
fn caller_evidence_decides_pane_external_or_unidentified() {
    let (mut h, caller, pid) = caller();
    let root = h.workspace_root();
    std::fs::write(root.join("ok.txt"), "ok").unwrap();
    let external = || Open {
        from: Some(caller),
        placement: Some(OpenPlacement::Down),
        ..Open::new("ok.txt", &root, caller)
    };
    let prefix = [999_998, 999_997];

    let before = pane_count(&h);
    for end in [
        AncestryEnd::LookupFailed,
        AncestryEnd::Cycle,
        AncestryEnd::HopLimit,
    ] {
        let reply = external().send_walk(&mut h, walk(&prefix, end));
        assert_eq!(
            reply["error_code"], "caller_unidentified",
            "{end:?}: {reply}"
        );
    }
    let reply = external().send_walk(&mut h, None);
    assert_eq!(reply["error_code"], "caller_unidentified", "{reply}");
    let reply = external().send_walk(&mut h, walk(&[], AncestryEnd::ReachedRoot));
    assert_eq!(reply["error_code"], "caller_unidentified", "{reply}");
    assert_eq!(pane_count(&h), before, "nothing opens without evidence");

    let reply = external().send_walk(&mut h, walk(&prefix, AncestryEnd::ReachedRoot));
    assert_eq!(
        reply["status"], "opened",
        "a complete walk is external: {reply}"
    );

    // The terminal's shell matched before the lookup failed: a pane caller,
    // so no --from is needed.
    std::fs::write(root.join("pane.txt"), "p").unwrap();
    let reply = Open {
        ..Open::new("pane.txt", &root, caller)
    }
    .send_walk(&mut h, walk(&[pid, 999_996], AncestryEnd::LookupFailed));
    assert_eq!(reply["status"], "opened", "{reply}");
    assert!(is_right_sibling(
        &h,
        caller,
        reply["pane_id"].as_u64().unwrap()
    ));
}

/// A terminal under two stacked view overlays is still its pane's caller.
#[test]
fn a_terminal_under_nested_overlays_is_its_panes_caller() {
    let (mut h, caller, pid) = caller();
    let root = h.workspace_root();
    std::fs::write(root.join("a.txt"), "a").unwrap();
    for _ in 0..2 {
        h.focus_pane(caller);
        h.app
            .launch_view_by_id_with_layout("file_browser", Some("overlay".into()), &[], None)
            .unwrap();
    }
    assert!(h.app.windows[0].panes[&caller].as_terminal().is_none());
    let reply = Open::new("a.txt", &root, caller).send(&mut h, &[pid]);
    assert_eq!(reply["status"], "opened", "{reply}");
}

// ── Slice 2, authority ───────────────────────────────────────────────────────

/// One parent context R with a human terminal, a worker terminal and a view,
/// and a separately owned nested context R/nested with its own view.
struct Scopes {
    h: HostHarness,
    root: PathBuf,
    human: PaneId,
    human_pid: u32,
    worker: PaneId,
    worker_pid: u32,
    view: PaneId,
    nested_view: PaneId,
}

fn scopes() -> Scopes {
    let (mut h, human, human_pid) = caller();
    let root = h.workspace_root();
    let view = *h.app.windows[0]
        .panes
        .keys()
        .find(|id| **id != human)
        .unwrap();
    let worker = h.add_focused_terminal();
    let worker_pid = h.terminal_backend(worker).child_pid();
    h.app.windows[0]
        .panes
        .get_mut(&worker)
        .and_then(|p| p.as_terminal_mut())
        .unwrap()
        .agent_harness = Some("pi".to_string());
    std::fs::create_dir(root.join("nested")).unwrap();
    std::fs::write(root.join("ok.txt"), "ok").unwrap();
    std::fs::write(root.join("nested/n.txt"), "n").unwrap();
    let nested_id = h.app.windows[0].context_id + 3_000;
    let window_id = h.app.windows.iter().map(|w| w.window_id).max().unwrap() + 1;
    h.app.windows.push(crate::host::context::Window {
        name: "Nested".into(),
        path: root.join("nested"),
        tree: egui_tiles::Tree::empty("file_open_nested"),
        panes: Default::default(),
        focused_pane: None,
        zoomed_pane: None,
        grid_x: 0,
        grid_y: 2,
        window_id,
        context_id: nested_id,
        tree_history: Default::default(),
    });
    h.app.router.push(crate::host::context::Context {
        name: "Nested".into(),
        root: root.join("nested"),
        description: None,
        context_id: nested_id,
        parent_id: Some(h.app.windows[0].context_id),
        depth: 1,
        parked: false,
    });
    let window = h.app.windows.len() - 1;
    let (_, nested_view) = h.app.add_view_pane_in_window(window, "test");
    Scopes {
        h,
        root,
        human,
        human_pid,
        worker,
        worker_pid,
        view,
        nested_view,
    }
}

fn external_from<'a>(path: &'a str, cwd: &Path, from: PaneId) -> Open<'a> {
    Open {
        from: Some(from),
        placement: Some(OpenPlacement::Down),
        cwd: Some(cwd.to_path_buf()),
        path,
        no_focus: false,
    }
}

const OUTSIDE: [u32; 1] = [999_995];

/// An external caller's authority is the selected context's, whoever owns
/// the anchor: human, worker and view anchors in one context answer alike.
#[test]
fn an_external_caller_gets_the_selected_contexts_authority_from_any_anchor() {
    let mut s = scopes();
    let root = s.root.clone();
    for anchor in [s.human, s.worker, s.view] {
        let reply = external_from("ok.txt", &root, anchor).send(&mut s.h, &OUTSIDE);
        assert!(
            reply["status"] == "opened" || reply["error_code"] == "placement_conflict",
            "{anchor}: {reply}"
        );
        let reply = external_from("nested/n.txt", &root, anchor).send(&mut s.h, &OUTSIDE);
        assert_eq!(
            reply["error_code"], "outside_authority",
            "{anchor}: {reply}"
        );
    }
    // Selecting the nested context evaluates its own root.
    let reply =
        external_from("n.txt", &root.join("nested"), s.nested_view).send(&mut s.h, &OUTSIDE);
    assert_eq!(reply["status"], "opened", "{reply}");
}

/// A pane caller's authority follows its matched terminal, covered or not,
/// and never the anchor it names.
#[test]
fn a_pane_callers_authority_follows_its_own_terminal() {
    let mut s = scopes();
    let root = s.root.clone();
    for pane in [s.human, s.worker] {
        s.h.focus_pane(pane);
        s.h.app
            .launch_view_by_id_with_layout("file_browser", Some("overlay".into()), &[], None)
            .unwrap();
    }
    // A pane the person opened has no wider reach than any pane (stint
    // 1285): a nested context is another scope, covered or not.
    let reply = Open {
        placement: Some(OpenPlacement::Down),
        ..Open::new("nested/n.txt", &root, s.human)
    }
    .send(&mut s.h, &[s.human_pid]);
    assert_eq!(reply["error_code"], "outside_authority", "{reply}");
    // A covered worker keeps its restriction, even anchored on the human.
    let before = pane_count(&s.h);
    let reply = Open {
        from: Some(s.human),
        placement: Some(OpenPlacement::Down),
        ..Open::new("nested/n.txt", &root, s.worker)
    }
    .send(&mut s.h, &[s.worker_pid]);
    assert_eq!(reply["error_code"], "outside_authority", "{reply}");
    // A cross-context anchor is refused for a pane caller.
    let reply = Open {
        from: Some(s.nested_view),
        ..Open::new("ok.txt", &root, s.worker)
    }
    .send(&mut s.h, &[s.worker_pid]);
    assert_eq!(reply["error_code"], "anchor_outside_context", "{reply}");
    assert_eq!(pane_count(&s.h), before);
}

/// Authority is decided on the canonical target before classification: a
/// symlink escaping the root to a FIFO is `outside_authority`, never
/// `special_file`, and nothing launches.
#[test]
fn denial_happens_before_classification_or_launch() {
    let mut s = scopes();
    let root = s.root.clone();
    let outside = tempfile::tempdir().unwrap();
    let fifo = outside.path().join("pipe.txt");
    assert!(std::process::Command::new("/usr/bin/mkfifo")
        .arg(&fifo)
        .status()
        .unwrap()
        .success());
    std::os::unix::fs::symlink(&fifo, root.join("escape.txt")).unwrap();
    let before = pane_count(&s.h);
    let reply = external_from("escape.txt", &root, s.view).send(&mut s.h, &OUTSIDE);
    assert_eq!(reply["error_code"], "outside_authority", "{reply}");
    let reply = Open::new("escape.txt", &root, s.human).send(&mut s.h, &[s.human_pid]);
    assert_eq!(reply["error_code"], "outside_authority", "{reply}");
    assert_eq!(pane_count(&s.h), before);
}

/// A focused tab open records history: Back returns to the caller.
#[test]
fn back_after_a_focused_file_open_tab_returns_to_the_caller() {
    let (mut h, caller, pid) = caller();
    let root = h.workspace_root();
    std::fs::write(root.join("a.txt"), "a").unwrap();
    let origin = h.app.windows[0].focused_pane;
    let reply = Open {
        placement: Some(OpenPlacement::Tab),
        ..Open::new("a.txt", &root, caller)
    }
    .send(&mut h, &[pid]);
    assert_eq!(reply["focused"], true, "{reply}");
    h.app.step_focus_history_back();
    assert_eq!(h.app.windows[0].focused_pane, origin);
}

/// A replacement-character filename is an ordinary UTF-8 name and opens
/// as itself.
#[test]
fn a_replacement_character_filename_opens_as_itself() {
    let (mut h, caller, pid) = caller();
    let root = h.workspace_root();
    std::fs::write(root.join("caf\u{FFFD}.txt"), "replacement").unwrap();
    let reply = Open::new("caf\u{FFFD}.txt", &root, caller).send(&mut h, &[pid]);
    let pane = reply["pane_id"].as_u64().expect("opened");
    assert_eq!(editor_buffer(&mut h, pane).1, "replacement");
}

// ── Round-3 review regressions ───────────────────────────────────────────────

/// An editor already open on the file but holding a failed load is not the
/// file: `file.open` names it and does not report `reused`.
#[test]
fn a_failed_editor_buffer_is_not_reused() {
    let (mut h, caller, pid) = caller();
    let root = h.workspace_root();
    let path = root.join("latin1.txt");
    std::fs::write(&path, [b'c', b'a', b'f', 0xe9]).unwrap();
    h.focus_pane(caller);
    h.app
        .launch_view_by_id_with_layout(
            "text-editor",
            Some("split_h".into()),
            &[path.canonicalize().unwrap().display().to_string()],
            None,
        )
        .unwrap();
    let stale = h
        .app
        .windows
        .iter()
        .flat_map(|w| w.panes.iter())
        .find_map(|(id, pane)| {
            pane.as_view()
                .is_some_and(|v| v.runtime.type_id() == "text-editor")
                .then_some(*id)
        })
        .expect("editor with a failed load");
    let before = pane_count(&h);
    let reply = Open::new("latin1.txt", &root, caller).send(&mut h, &[pid]);
    assert_eq!(reply["error_code"], "load_failed", "{reply}");
    assert_eq!(reply["pane_id"], stale, "{reply}");
    assert_eq!(pane_count(&h), before);
}

/// The editor reports a file that was missing at read time as not loaded;
/// a scratch editor on a new path is still created normally.
#[test]
fn an_editor_on_a_missing_file_reports_it_did_not_load() {
    use crate::app::view_trait::View;
    let dir = tempfile::tempdir().unwrap();
    let editor = crate::app::text_editor_view::TextEditorView::new(dir.path().join("vanished.txt"));
    assert!(matches!(editor.document_load(), Some(Err(_))));
    std::fs::write(dir.path().join("here.txt"), "here").unwrap();
    let editor = crate::app::text_editor_view::TextEditorView::new(dir.path().join("here.txt"));
    assert_eq!(editor.document_load(), Some(Ok(())));
}

/// A handler that is not a file viewer is refused before it starts: no
/// terminal runs the path as a command, no Assistant opens.
#[test]
fn a_non_view_handler_is_refused_before_launch() {
    let (mut h, caller, pid) = caller();
    let root = h.workspace_root();
    std::fs::write(root.join("a.txt"), "echo hi").unwrap();
    for handler in ["app:terminal", "app:assistant"] {
        let mut handlers = std::collections::HashMap::new();
        handlers.insert("txt".to_string(), handler.to_string());
        h.app.config.file_handlers = Some(handlers);
        let (panes, windows) = (pane_count(&h), h.app.windows.len());
        let reply = Open::new("a.txt", &root, caller).send(&mut h, &[pid]);
        assert_eq!(
            reply["error_code"], "handler_unavailable",
            "{handler}: {reply}"
        );
        assert_eq!(pane_count(&h), panes, "{handler}");
        assert_eq!(h.app.windows.len(), windows, "{handler}");
    }
}

// ── Round-4 review regressions ───────────────────────────────────────────────

/// The load result comes from the editor's one read: a read that found no
/// file is not a load, even if the file exists by the time anyone looks.
#[test]
fn the_editor_load_result_is_its_read_result() {
    use crate::app::view_trait::View;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("reappeared.txt");
    std::fs::write(&path, "now here").unwrap();
    let missed = crate::app::text_editor_view::TextEditorView::from_read(
        path.clone(),
        None,
        Err(std::io::ErrorKind::NotFound.into()),
    );
    assert!(matches!(missed.document_load(), Some(Err(_))));
    let read = crate::app::text_editor_view::TextEditorView::from_read(
        path,
        None,
        Ok("now here".to_string()),
    );
    assert_eq!(read.document_load(), Some(Ok(())));
}

/// The handler guard decides on the id the launcher dispatches on: a view
/// contributed by an installed plugin named `terminal` is refused before
/// anything starts.
#[test]
fn a_view_owned_by_a_plugin_named_terminal_is_refused() {
    let (mut h, caller, pid) = caller();
    let root = h.workspace_root();
    let _plugin = install_plugin(&mut h, "terminal", "");
    std::fs::write(root.join("a.txt"), "echo hi").unwrap();
    let mut handlers = std::collections::HashMap::new();
    handlers.insert("txt".to_string(), "app:terminal.main".to_string());
    h.app.config.file_handlers = Some(handlers);
    let (panes, windows) = (pane_count(&h), h.app.windows.len());
    let reply = Open::new("a.txt", &root, caller).send(&mut h, &[pid]);
    assert_eq!(reply["error_code"], "handler_unavailable", "{reply}");
    assert_eq!(pane_count(&h), panes);
    assert_eq!(h.app.windows.len(), windows);
}

// ── Round-5 review regression ────────────────────────────────────────────────

/// An explicit view of a multi-view plugin with no default view opens: the
/// guard checks the owning plugin, and the launch still names the view.
#[test]
fn an_explicit_view_of_a_plugin_without_a_default_view_opens() {
    let (mut h, caller, pid) = caller();
    let root = h.workspace_root();
    let tmp = tempfile::tempdir().unwrap();
    let global = tmp.path().join("global-apps");
    let dir = global.join("notes");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("manifest.toml"),
        "schema_version = 2\n\n[plugin]\nid = \"notes\"\nname = \"notes\"\nversion = \"0.0.1\"\n\n\
         [runtime]\nkind = \"python\"\nentry = \"main.py\"\n\n\
         [[contributes.views]]\nid = \"notes.outline\"\n\n\
         [[contributes.views]]\nid = \"notes.board\"\n",
    )
    .unwrap();
    std::fs::write(dir.join("main.py"), "#!/usr/bin/env python3\n").unwrap();
    let registry = crate::app::registry::PluginRegistry::load_with_global(tmp.path(), &global);
    assert!(registry.get("notes").is_some(), "notes loads");
    h.trust_plugin_folder(&global);
    let context_root = h.app.router.get(0).root.clone();
    h.app.registries.set_view_for_test(&context_root, registry);
    std::fs::write(root.join("a.txt"), "outline").unwrap();
    let mut handlers = std::collections::HashMap::new();
    handlers.insert("txt".to_string(), "app:notes.outline".to_string());
    h.app.config.file_handlers = Some(handlers);

    let reply = Open::new("a.txt", &root, caller).send(&mut h, &[pid]);
    assert_eq!(reply["status"], "opened", "{reply}");
    assert_eq!(reply["handler"], "notes.outline", "{reply}");
}

/// Stint 1673: a builtin view that cannot be placed is the launcher's error,
/// carried to `file.open` with its reason, never an `Ok` read as opened.
#[test]
fn a_builtin_placement_failure_reaches_file_open_with_its_reason() {
    let (mut h, caller, pid) = caller();
    let root = h.workspace_root();
    std::fs::write(root.join("note.txt"), "n").unwrap();
    h.app.config.core_only = Some(true);
    let (panes, windows) = (pane_count(&h), h.app.windows.len());
    for placement in [None, Some(OpenPlacement::Tab), Some(OpenPlacement::Window)] {
        let reply = Open {
            placement,
            ..Open::new("note.txt", &root, caller)
        }
        .send(&mut h, &[pid]);
        assert_eq!(reply["error_code"], "handler_unavailable", "{placement:?}: {reply}");
        assert!(
            reply["error"].as_str().unwrap_or("").contains("core-only"),
            "{placement:?}: the reason is the launcher's: {reply}"
        );
        assert_eq!(pane_count(&h), panes, "{placement:?}");
        assert_eq!(h.app.windows.len(), windows, "{placement:?}");
    }
}

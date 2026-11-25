use crate::app::agent_tools::UseSubject;
use crate::broker::gate::{ApprovalChoice, PermissionMonitor};
use crate::host::keys::Action;
use crate::testing::HostHarness;
use std::sync::Arc;

pub(super) fn approve_root(actor: &str, context: u64) -> Arc<PermissionMonitor> {
    let monitor = PermissionMonitor::for_profile(&crate::config::config_dir()).unwrap();
    let ask = monitor
        .host_records()
        .pending()
        .into_iter()
        .find(|p| {
            p.actor_id == actor && p.tool == "context.set_root" && p.context_id == Some(context)
        })
        .expect("pending root change for the supplying principal and target context");
    monitor
        .approve_pending(&ask.pending_request_id, ApprovalChoice::Once)
        .unwrap();
    monitor
}

#[test]
fn root_change_from_a_foreign_pane_is_refused_with_no_effect() {
    let mut h = HostHarness::new();
    let pane = h.add_focused_terminal();
    let origin = h.origin_from_pane(pane);
    let before = h.app.router.active().clone();
    let id = before.context_id;
    let revision = h.app.context_scope_revision(id);
    let target = tempfile::tempdir().unwrap();
    let request = crate::protocol::HostRequest::SetContextRoot {
        root: target.path().into(),
        context_id: Some(id),
        response_file: None,
    };
    h.app
        .handle_pane_ipc_request(origin.clone(), request.clone());
    approve_root(&format!("pane:{pane}"), id);
    // The queued request has an exact allow in the context its pane just left.
    let live = h.app.pane_authority(pane).unwrap();
    live.publish_move(id + 1, target.path().into());
    let moved = live.scope();
    h.app.handle_pane_ipc_request(origin, request);
    assert_eq!(h.app.router.active().root, before.root);
    assert_eq!(h.app.router.active().name, before.name);
    assert_eq!(h.app.context_scope_revision(id), revision);
    assert_eq!(live.scope(), moved);
}

#[test]
fn root_change_after_revoke_is_refused_with_no_effect() {
    let mut h = HostHarness::new();
    let pane = h.add_focused_terminal();
    let origin = h.origin_from_pane(pane);
    let before = h.app.router.active().clone();
    let id = before.context_id;
    let live = h.app.pane_authority(pane).unwrap();
    let scope = live.scope();
    let revision = h.app.context_scope_revision(id);
    let target = tempfile::tempdir().unwrap();
    let root = target.path();
    let subject = UseSubject::from_origin(&origin).unwrap();
    assert!(subject.admit_root_change(&before.root, id, root).is_err());
    let monitor = approve_root(&format!("pane:{pane}"), id);
    let (permit, record_outcome) = subject.admit_root_change(&before.root, id, root).unwrap();
    let grant = monitor.store().records()[0].grant_id.clone();
    assert!(monitor.revoke_grant_id(&grant).unwrap());
    let proof = crate::broker::permit::ContextRootProof::Permitted(permit);
    let result = h.app.commit_context_root(root.into(), id, proof);
    record_outcome(result.is_ok());
    assert!(result.unwrap_err().contains("no longer allowed"));
    assert_eq!(h.app.router.active().root, before.root);
    assert_eq!(h.app.router.active().name, before.name);
    assert_eq!(h.app.context_scope_revision(id), revision);
    assert_eq!(live.scope(), scope);
    assert!(!root.join(".plexi").exists());
}

#[test]
fn focused_cwd_action_requires_the_panes_permit() {
    let mut h = HostHarness::new();
    let pane = h.add_focused_terminal();
    let id = h.app.router.active().context_id;
    let cwd = h.workspace_root().canonicalize().unwrap();
    let old = tempfile::tempdir().unwrap();
    h.app.set_context_root(old.path().into(), Some(id));
    let act = |h: &mut HostHarness| {
        h.app
            .dispatch_host_action(Action::SetContextRootFromCwd, &h.app.ctx.clone());
    };
    // Socket-injected input is the pane's own request: it needs the permit.
    h.app.synthetic_input_frame = true;
    act(&mut h);
    assert_eq!(h.app.router.active().root, old.path());
    approve_root(&format!("pane:{pane}"), id);
    act(&mut h);
    assert_eq!(h.app.router.active().root, cwd);
}

#[test]
fn a_persons_own_keypress_sets_the_root_in_one_gesture() {
    let mut h = HostHarness::new();
    h.add_focused_terminal();
    let id = h.app.router.active().context_id;
    let cwd = h.workspace_root().canonicalize().unwrap();
    let old = tempfile::tempdir().unwrap();
    h.app.set_context_root(old.path().into(), Some(id));
    assert!(!h.app.synthetic_input_frame);
    h.app
        .dispatch_host_action(Action::SetContextRootFromCwd, &h.app.ctx.clone());
    assert_eq!(h.app.router.active().root, cwd);
    let monitor = PermissionMonitor::for_profile(&crate::config::config_dir()).unwrap();
    assert!(
        monitor.host_records().pending().is_empty(),
        "no approval is filed"
    );
}

#[test]
fn focused_app_view_offers_its_workspace_root_to_a_person_only() {
    let mut h = HostHarness::new();
    let pane = h.add_test_pane();
    let tile = h.app.windows[0].tree.tiles.find_pane(&pane).unwrap();
    h.app.windows[0].focused_pane = Some(tile);
    let id = h.app.router.active().context_id;
    let view_root = h.workspace_root();
    let old = tempfile::tempdir().unwrap();
    h.app.set_context_root(old.path().into(), Some(id));
    let act = |h: &mut HostHarness| {
        h.app
            .dispatch_host_action(Action::SetContextRootFromCwd, &h.app.ctx.clone());
    };
    h.app.synthetic_input_frame = true;
    act(&mut h);
    assert_eq!(
        h.app.router.active().root,
        old.path(),
        "a view has no permit"
    );
    h.app.synthetic_input_frame = false;
    act(&mut h);
    assert_eq!(h.app.router.active().root, view_root);
}

#[test]
fn synthetic_input_cannot_commit_the_root_editor() {
    use crate::app::{FocusKind, OverlayTarget, TextInputOverlay};
    let mut h = HostHarness::new();
    let pane = h.add_focused_terminal();
    let id = h.app.router.active().context_id;
    let old = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    h.app.set_context_root(old.path().into(), Some(id));
    let open = |h: &mut HostHarness| {
        h.app.text_overlay = Some((
            TextInputOverlay {
                label: "Root directory".to_string(),
                hint: String::new(),
                buffer: target.path().display().to_string(),
                session: crate::ui::text_field::OpenSession::new(),
            },
            OverlayTarget::ContextRoot(id),
        ));
        h.app.push_focus_layer(FocusKind::TextInput);
        h.run_frames(1);
    };
    open(&mut h);
    // IPC sends a complete keystroke so egui sees Enter released before the
    // person's next press.
    let injected_enter = crate::app::key_str_to_egui_raw_input("Enter").unwrap();
    input_frame(&mut h, Some(pane), injected_enter.events);
    assert_eq!(h.app.router.active().root, old.path());
    let (overlay, _) = h.app.text_overlay.as_ref().expect("the editor stays open");
    assert_eq!(overlay.buffer, target.path().display().to_string());
    assert!(
        overlay.label.ends_with("press Enter again"),
        "{}",
        overlay.label
    );
    // The person's own Enter commits the original draft, without retyping.
    h.press_key(egui::Key::Enter, egui::Modifiers::NONE);
    assert!(!h.app.synthetic_input_frame);
    assert_eq!(h.app.router.active().root, target.path());
}

/// One input frame. With `socket_pane`, the events are queued for that pane
/// exactly as `plexi pane` injection queues them, so the frame is marked
/// synthetic; otherwise they arrive as a person's own input.
fn input_frame(h: &mut HostHarness, socket_pane: Option<u64>, events: Vec<egui::Event>) {
    let raw = egui::RawInput {
        events,
        ..Default::default()
    };
    match socket_pane {
        Some(pane) => {
            h.app
                .pending_pane_inputs
                .entry(pane)
                .or_default()
                .push(raw.into());
            h.frame(egui::RawInput::default());
            assert!(
                h.app.synthetic_input_frame,
                "queued pane input is synthetic"
            );
        }
        None => {
            h.frame(raw);
            assert!(!h.app.synthetic_input_frame);
        }
    }
}

fn click(
    h: &mut HostHarness,
    socket_pane: Option<u64>,
    pos: egui::Pos2,
    button: egui::PointerButton,
) {
    for pressed in [true, false] {
        let events = vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button,
                pressed,
                modifiers: egui::Modifiers::NONE,
            },
        ];
        input_frame(h, socket_pane, events);
    }
    h.run_frames(2);
}

#[test]
fn sidebar_set_root_from_cwd_requires_the_supplying_panes_permit() {
    let mut h = HostHarness::new();
    let pane = h.add_focused_terminal();
    let cwd = h.workspace_root().canonicalize().unwrap();
    let old = tempfile::tempdir().unwrap();
    let mut clicked = h.app.router.active().clone();
    let id = crate::testing::reserve_test_context_id();
    clicked.context_id = id;
    clicked.root = old.path().into();
    h.app.router.push(clicked);
    h.app.sidebar_visible = true;
    h.run_frames(2);
    let act = |h: &mut HostHarness, socket: Option<u64>| {
        let row = h
            .app
            .ctx
            .read_response(egui::Id::new(("ctx", 1usize)))
            .unwrap();
        click(h, socket, row.rect.center(), egui::PointerButton::Secondary);
        let pos = h
            .last_full_output
            .as_ref()
            .unwrap()
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(t) if t.galley.text() == "Set root to current path" => {
                    Some(t.visual_bounding_rect().center())
                }
                _ => None,
            })
            .expect("sidebar root action");
        click(h, socket, pos, egui::PointerButton::Primary);
    };
    act(&mut h, Some(pane));
    assert_eq!(h.app.context_root_for(id).unwrap(), old.path());
    approve_root(&format!("pane:{pane}"), id);
    act(&mut h, Some(pane));
    assert_eq!(h.app.context_root_for(id).unwrap(), cwd);
    h.app.set_context_root(old.path().into(), Some(id));
    act(&mut h, None);
    assert_eq!(
        h.app.context_root_for(id).unwrap(),
        cwd,
        "a person's own click applies in one gesture"
    );
}

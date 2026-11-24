use crate::app::agent_tools::UseSubject;
use crate::broker::gate::{ApprovalChoice, PermissionMonitor};
use crate::host::keys::Action;
use crate::testing::HostHarness;
use std::sync::Arc;

pub(super) fn approve_root(actor: &str, context: u64) -> Arc<PermissionMonitor> {
    let monitor = PermissionMonitor::for_profile(&crate::config::config_dir()).unwrap();
    let ask = monitor
        .list_pending()
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
    act(&mut h);
    assert_eq!(h.app.router.active().root, old.path());
    approve_root(&format!("pane:{pane}"), id);
    act(&mut h);
    assert_eq!(h.app.router.active().root, cwd);
}

fn click(h: &mut HostHarness, pos: egui::Pos2, button: egui::PointerButton) {
    for pressed in [true, false] {
        h.frame(egui::RawInput {
            events: vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            ..Default::default()
        });
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
    let act = |h: &mut HostHarness| {
        let row = h
            .app
            .ctx
            .read_response(egui::Id::new(("ctx", 1usize)))
            .unwrap();
        click(h, row.rect.center(), egui::PointerButton::Secondary);
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
        click(h, pos, egui::PointerButton::Primary);
    };
    act(&mut h);
    assert_eq!(h.app.context_root_for(id).unwrap(), old.path());
    approve_root(&format!("pane:{pane}"), id);
    act(&mut h);
    assert_eq!(h.app.context_root_for(id).unwrap(), cwd);
}

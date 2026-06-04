use super::super::*;
use crate::app::permissions::AppPermissions;
use crate::app_protocol::{
    ArtifactOpenMode, AppRequest, PathTokenMode, PlexiEvent,
};
use std::collections::HashSet;

fn make_app(capabilities: HashSet<Capability>) -> Option<ProcessApp> {
    let (app, _tx) = ProcessApp::new_for_test(
        7,
        AppPermissions {
            capabilities,
            blocked: HashSet::new(),
            is_builtin: false,
            allowed_hosts: vec![],
        },
    );
    Some(app)
}

// ── Capability denial paths ─────────────────────────────────────────

#[test]
fn denied_app_request_linked_terminal_emits_sentinel_event() {
    let Some(mut app) = make_app(HashSet::new()) else {
        eprintln!("skipping: no /bin/sh available");
        return;
    };
    app.route_command(AppRequest::RequestLinkedTerminal {
        request_id: "req-0".to_string(),
        cwd: None,
        label: None,
    });
    // Sentinel event so the SDK's blocking helper unblocks.
    let event = app
        .outbound_events
        .iter()
        .find(|e| matches!(e, PlexiEvent::LinkedTerminalReady { .. }))
        .expect("denied path must emit LinkedTerminalReady sentinel");
    match event {
        PlexiEvent::LinkedTerminalReady {
            request_id,
            terminal_pane_id,
        } => {
            assert_eq!(request_id, "req-0");
            assert_eq!(
                *terminal_pane_id, 0,
                "sentinel pane id (0) signals capability denied"
            );
        }
        other => panic!("expected LinkedTerminalReady, got {other:?}"),
    }
    // Must NOT have queued an AppCommand for the host to act on.
    assert!(
        !app.pending_commands.iter().any(|c| matches!(
            c,
            AppCommand::RequestLinkedTerminal { .. }
        )),
        "denied path must not enqueue RequestLinkedTerminal"
    );
}

#[test]
fn denied_app_run_in_linked_terminal_drops_silently() {
    let Some(mut app) = make_app(HashSet::new()) else {
        eprintln!("skipping: no /bin/sh available");
        return;
    };
    app.route_command(AppRequest::RunInLinkedTerminal {
        terminal_pane_id: 42,
        command: "ls".to_string(),
        echo: true,
    });
    // No event — fire-and-forget verb has no response shape.
    // Must NOT enqueue the AppCommand.
    assert!(
        !app.pending_commands.iter().any(|c| matches!(
            c,
            AppCommand::RunInLinkedTerminal { .. }
        )),
        "denied path must drop RunInLinkedTerminal without dispatch"
    );
}

#[test]
fn denied_app_request_command_preview_emits_empty_cwd_sentinel() {
    let Some(mut app) = make_app(HashSet::new()) else {
        eprintln!("skipping: no /bin/sh available");
        return;
    };
    app.route_command(AppRequest::RequestCommandPreview {
        request_id: "req-9".to_string(),
        terminal_pane_id: 42,
        command: "rm -rf .git".to_string(),
    });
    let event = app
        .outbound_events
        .iter()
        .find(|e| matches!(e, PlexiEvent::CommandPreview { .. }))
        .expect("denied path must emit CommandPreview sentinel");
    match event {
        PlexiEvent::CommandPreview {
            request_id,
            command,
            would_run_in_cwd,
        } => {
            assert_eq!(request_id, "req-9");
            assert_eq!(command, "rm -rf .git");
            assert!(
                would_run_in_cwd.is_empty(),
                "denied path must return empty cwd: got {would_run_in_cwd:?}"
            );
        }
        other => panic!("expected CommandPreview, got {other:?}"),
    }
}

// ── Granted-path AppCommand enqueue ─────────────────────────────────

#[test]
fn granted_app_dispatches_request_linked_terminal() {
    let mut caps = HashSet::new();
    caps.insert(Capability::TerminalBindings);
    let Some(mut app) = make_app(caps) else {
        eprintln!("skipping: no /bin/sh available");
        return;
    };
    app.route_command(AppRequest::RequestLinkedTerminal {
        request_id: "req-ok".to_string(),
        cwd: Some("/tmp/foo".to_string()),
        label: Some("bindings demo".to_string()),
    });
    // No synchronous event — granted path defers to the host.
    assert!(app
        .outbound_events
        .iter()
        .find(|e| matches!(e, PlexiEvent::LinkedTerminalReady { .. }))
        .is_none());
    // The AppCommand lands on pending_commands with sender_pane_id
    // stamped (set_pane_id(7)).
    let cmd = app
        .pending_commands
        .iter()
        .find_map(|c| {
            if let AppCommand::RequestLinkedTerminal {
                sender_pane_id,
                request_id,
                cwd,
                label,
            } = c
            {
                Some((*sender_pane_id, request_id.clone(), cwd.clone(), label.clone()))
            } else {
                None
            }
        })
        .expect("granted path must enqueue AppCommand::RequestLinkedTerminal");
    assert_eq!(cmd.0, 7, "sender_pane_id must come from app's pane_id");
    assert_eq!(cmd.1, "req-ok");
    assert_eq!(cmd.2.as_deref(), Some("/tmp/foo"));
    assert_eq!(cmd.3.as_deref(), Some("bindings demo"));
}

#[test]
fn granted_app_dispatches_run_in_linked_terminal() {
    let mut caps = HashSet::new();
    caps.insert(Capability::TerminalBindings);
    let Some(mut app) = make_app(caps) else {
        eprintln!("skipping: no /bin/sh available");
        return;
    };
    app.route_command(AppRequest::RunInLinkedTerminal {
        terminal_pane_id: 42,
        command: "ls -la".to_string(),
        echo: true,
    });
    let cmd = app
        .pending_commands
        .iter()
        .find_map(|c| {
            if let AppCommand::RunInLinkedTerminal {
                terminal_pane_id,
                command,
                echo,
                ..
            } = c
            {
                Some((*terminal_pane_id, command.clone(), *echo))
            } else {
                None
            }
        })
        .expect("granted path must enqueue RunInLinkedTerminal");
    assert_eq!(cmd.0, 42);
    assert_eq!(cmd.1, "ls -la");
    assert!(cmd.2);
}

#[test]
fn granted_app_dispatches_insert_path_token() {
    let mut caps = HashSet::new();
    caps.insert(Capability::TerminalBindings);
    let Some(mut app) = make_app(caps) else {
        eprintln!("skipping: no /bin/sh available");
        return;
    };
    app.route_command(AppRequest::InsertPathToken {
        terminal_pane_id: 42,
        path: "/tmp/x".to_string(),
        mode: PathTokenMode::Replace,
    });
    let mode = app
        .pending_commands
        .iter()
        .find_map(|c| {
            if let AppCommand::InsertPathToken { mode, .. } = c {
                Some(*mode)
            } else {
                None
            }
        })
        .expect("granted path must enqueue InsertPathToken");
    assert_eq!(mode, PathTokenMode::Replace);
}

#[test]
fn granted_app_dispatches_open_artifact() {
    let mut caps = HashSet::new();
    caps.insert(Capability::TerminalBindings);
    let Some(mut app) = make_app(caps) else {
        eprintln!("skipping: no /bin/sh available");
        return;
    };
    app.route_command(AppRequest::OpenArtifact {
        path: "/tmp/x".to_string(),
        mode: ArtifactOpenMode::RevealInFinder,
    });
    let mode = app
        .pending_commands
        .iter()
        .find_map(|c| {
            if let AppCommand::OpenArtifact { mode, .. } = c {
                Some(*mode)
            } else {
                None
            }
        })
        .expect("granted path must enqueue OpenArtifact");
    assert_eq!(mode, ArtifactOpenMode::RevealInFinder);
}

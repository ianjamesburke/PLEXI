#[test]
fn turn_lifecycle_conversation_switch_cancels_worker() {
    let ws = tempfile::tempdir().unwrap();
    let mut app = test_app(ws.path());
    app.model.streaming.in_flight = true;
    let cancel = app.turn_cancel.clone();
    let dispatcher = app.gated_dispatcher();
    let effects = app.model.switch_conversation("next".into(), Vec::new());
    app.execute_effects(effects);
    assert!(!app.model.streaming.in_flight);
    assert!(
        cancel.is_cancelled(),
        "departing dispatcher must be cancelled"
    );
    assert_eq!(
        dispatcher
            .dispatch_call("departed".into(), HOST_TOOL_PANES_LIST, "{}".into())
            .error_code
            .as_deref(),
        Some("cancelled")
    );
}

#[test]
fn turn_lifecycle_cancel_clears_external_sheet() {
    let ws = tempfile::tempdir().unwrap();
    let mut app = test_app(ws.path());
    app.side_in_flight = Some("cli-turn".into());
    app.side_conversation = Some("cli-conversation".into());
    app.open_external_permission_sheet("tool", "{}", "actor", "resource", "pending");
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    app.pending_reply = Some(tx);
    app.unblock_pending_workers("test cancellation");
    assert!(matches!(rx.try_recv(), Ok(PermissionReply::Deny)));
    assert!(app.model.pending_permission.is_none());
    assert!(app.permission_owner.is_none());
}

#[test]
fn turn_lifecycle_cancel_rejects_queued_host_call() {
    let ws = tempfile::tempdir().unwrap();
    let mut app = test_app(ws.path());
    let (reply, rx) = std::sync::mpsc::sync_channel(1);
    app.flow_tx
        .send(ToolFlowEvent::HostCall {
            cancel: app.turn_cancel.clone(),
            tool: HOST_TOOL_PANES_LIST.into(),
            input_json: "{}".into(),
            reply,
        })
        .unwrap();
    app.model.streaming.in_flight = true;
    let input = crate::app::input_router::PlexiInput::synthetic(
        vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
        egui::Modifiers::NONE,
    );
    assert!(matches!(app.handle_key(&input), KeyDisposition::Consumed));
    assert!(app.turn_cancel.is_cancelled());
    app.pump_turn_io();
    assert!(
        app.pending_commands.is_empty(),
        "cancelled host call was forwarded"
    );
    assert!(rx.try_recv().unwrap().error.is_some());
}

#[test]
fn turn_lifecycle_cancel_rejects_already_forwarded_host_command() {
    let ws = tempfile::tempdir().unwrap();
    let mut app = test_app(ws.path());
    let (reply, rx) = std::sync::mpsc::sync_channel(1);
    app.handle_host_call(HOST_TOOL_PANES_LIST, "{}", reply);
    assert_eq!(app.pending_commands.len(), 1);
    app.turn_cancel.cancel();
    let mut harness = crate::ui_tests::PlexiUiHarness::new_sized(900.0, 600.0);
    harness.open_assistant_built(app, ws.path().to_path_buf());
    harness.run_steps(3);
    let result = rx.try_recv().expect("host serviced the queued command");
    assert!(result
        .error
        .as_deref()
        .unwrap_or("")
        .starts_with("cancelled:"));
}

#[test]
fn turn_lifecycle_plan_stops_existing_dispatcher() {
    let ws = tempfile::tempdir().unwrap();
    let mut app = test_app(ws.path());
    let dispatcher = app.gated_dispatcher();
    app.model.streaming.in_flight = true;
    let path = ws.path().join("agents/settings.toml");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, "[permissions]\ndefault_posture = \"plan\"\n").unwrap();
    app.reload_settings();
    assert!(app.turn_cancel.is_cancelled());
    let result = dispatcher.dispatch_call("cancelled".into(), HOST_TOOL_PANES_LIST, "{}".into());
    assert_eq!(result.error_code.as_deref(), Some("cancelled"));
}

#[test]
fn turn_lifecycle_provenance_change_keeps_turn_alive() {
    let ws = tempfile::tempdir().unwrap();
    let mut app = test_app(ws.path());
    app.model.streaming.in_flight = true;
    let before = app.settings.permissions.clone();
    let path = ws.path().join("agents/settings.toml");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, "[permissions]\ndefault_posture = \"review\"\n").unwrap();
    app.reload_settings();
    assert_eq!(
        before.broker_posture(),
        app.settings.permissions.broker_posture()
    );
    assert_ne!(
        before.posture.source,
        app.settings.permissions.posture.source
    );
    assert!(!app.turn_cancel.is_cancelled());
}

#[test]
fn turn_lifecycle_queued_ask_does_not_park_cancelled_worker() {
    let ws = tempfile::tempdir().unwrap();
    let mut app = test_plugin_with_context(ws.path(), 1578001);
    register_echo_provider(1578001, &["cancel.ask"], 1578001);
    let rx = dispatch_on_worker(Arc::new(app.gated_dispatcher()), "cancel.ask");
    let timeout = crate::testing::load_aware_timeout(std::time::Duration::from_secs(5));
    let ask = app
        .flow_rx
        .recv_timeout(timeout)
        .expect("worker queued approval");
    assert!(matches!(&ask, ToolFlowEvent::Ask { .. }));
    app.turn_cancel.cancel();
    let result = rx.recv_timeout(timeout);
    app.handle_flow_event(ask);
    tool_dispatch::unregister(1578001);
    assert!(result.is_ok(), "cancelled presenter remained blocked");
    assert!(
        app.model.pending_permission.is_none(),
        "stale Ask opened a sheet"
    );
}

#[test]
fn turn_lifecycle_event_preemption_clears_parked_approval() {
    let ws = tempfile::tempdir().unwrap();
    let timeline = chess_timeline();
    let mut app = test_plugin_with_timeline(ws.path(), timeline.clone());
    app.subscribe_stream("chess", "move.played");
    app.model.streaming.in_flight = true;
    register_echo_provider(1578004, &["cancel.preempt"], 1);
    let rx = dispatch_on_worker(Arc::new(app.gated_dispatcher()), "cancel.preempt");
    let timeout = crate::testing::load_aware_timeout(std::time::Duration::from_secs(5));
    let ask = app
        .flow_rx
        .recv_timeout(timeout)
        .expect("real approval event");
    assert!(matches!(&ask, ToolFlowEvent::Ask { .. }));
    app.handle_flow_event(ask);
    assert!(app.pending_permission_cancel.is_some());
    emit_move(&timeline, PluginEventActor::User, None, None);
    app.pump_deliveries();
    assert!(app.turn_cancel.is_cancelled());
    assert!(rx
        .recv_timeout(timeout)
        .expect("preempted worker returned")
        .error
        .is_some());
    assert!(app.model.pending_permission.is_none());
    assert!(app.pending_reply.is_none());
    tool_dispatch::unregister(1578004);
}

fn write_lifecycle_agent(ws: &Path, posture: &str) {
    let dir = ws.join("agents/lifecycle");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("AGENT.md"), "Lifecycle test agent").unwrap();
    std::fs::write(dir.join("settings.toml"), format!(
        "[agent]\nid = \"lifecycle\"\ndisplay_name = \"Lifecycle\"\ndefault_tier = \"medium\"\n[permissions]\ndefault_posture = \"{posture}\"\n"
    )).unwrap();
}

#[test]
fn turn_lifecycle_stricter_agent_switch_cancels_dispatchers() {
    let ws = tempfile::tempdir().unwrap();
    write_lifecycle_agent(ws.path(), "deny");
    let mut app = test_app(ws.path());
    app.model.streaming.in_flight = true;
    let dispatcher = app.gated_dispatcher();
    app.cmd_switch_agent("lifecycle");
    assert_eq!(app.active_posture().default_posture, Decision::Deny);
    assert!(app.turn_cancel.is_cancelled());
    assert!(app.side_cancel.is_cancelled());
    assert_eq!(
        dispatcher
            .dispatch_call("old".into(), HOST_TOOL_PANES_LIST, "{}".into())
            .error_code
            .as_deref(),
        Some("cancelled")
    );
}

#[test]
fn turn_lifecycle_agent_reload_cancels_dispatchers() {
    let ws = tempfile::tempdir().unwrap();
    write_lifecycle_agent(ws.path(), "ask");
    let mut app = test_app(ws.path());
    app.cmd_switch_agent("lifecycle");
    assert!(
        !app.turn_cancel.is_cancelled(),
        "same effective posture stays alive"
    );
    app.model.streaming.in_flight = true;
    write_lifecycle_agent(ws.path(), "deny");
    app.reload_agents();
    assert_eq!(app.active_posture().default_posture, Decision::Deny);
    assert!(app.turn_cancel.is_cancelled());
    assert!(app.side_cancel.is_cancelled());
}

#[test]
fn turn_lifecycle_executed_tool_retains_cancelled_transcript_row() {
    check_executed_tool_transcript(false, false);
}

#[test]
fn turn_lifecycle_cancel_during_execution_retains_transcript_row() {
    check_executed_tool_transcript(true, false);
}

#[test]
fn turn_lifecycle_late_completion_does_not_enter_next_conversation() {
    check_executed_tool_transcript(true, true);
}

fn check_executed_tool_transcript(cancel_in_handler: bool, switch_conversation: bool) {
    let ws = tempfile::tempdir().unwrap();
    let mut app = test_app(ws.path());
    app.model.streaming.in_flight = true;
    let mut dispatcher = app.gated_dispatcher();
    let cancel = app.turn_cancel.clone();
    let executed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let observed = executed.clone();
    dispatcher.add_host_tools(
        Vec::new(),
        Arc::new(move |_, _| {
            observed.store(true, std::sync::atomic::Ordering::SeqCst);
            if cancel_in_handler {
                cancel.cancel();
            }
            ToolCallResult::ok("{\"ran\":true}")
        }),
    );
    let rx = dispatch_on_worker(Arc::new(dispatcher), HOST_TOOL_PANES_LIST);
    pump_until(&mut app, "approval", |a| {
        a.model.pending_permission.is_some()
    });
    app.resolve_permission(PermissionChoice::AllowOnce);
    let result = rx
        .recv_timeout(crate::testing::load_aware_timeout(
            std::time::Duration::from_secs(5),
        ))
        .unwrap();
    assert!(result.error.is_none(), "{result:?}");
    assert!(executed.load(std::sync::atomic::Ordering::SeqCst));
    app.cancel_turn(false, "test cancel after execution");
    if switch_conversation {
        let effects = app.model.switch_conversation("next".into(), Vec::new());
        app.execute_effects(effects);
        app.pump_turn_io();
        assert!(!app
            .model
            .turns
            .iter()
            .any(|row| row.text.contains(HOST_TOOL_PANES_LIST)));
        return;
    }
    app.pump_turn_io();
    app.model
        .finish_turn(&app.model.conversation_id.clone(), Ok("stopped".into()));
    assert!(
        app.model
            .turns
            .iter()
            .any(|row| row.text.contains(HOST_TOOL_PANES_LIST)
                && row.text.contains(if cancel_in_handler {
                    "cancelled"
                } else {
                    "completed before cancel"
                })),
        "executed tool vanished (cancel_in_handler={cancel_in_handler})"
    );
    if !cancel_in_handler {
        let row = app
            .model
            .turns
            .iter()
            .find(|row| row.text.contains(HOST_TOOL_PANES_LIST))
            .unwrap();
        assert_eq!(row.status, Some(model::ToolStatus::Succeeded));
        assert!(!row.text.contains("cancelled:"));
    }
}

#[test]
fn turn_lifecycle_cancel_during_hook_stops_handler() {
    struct CancelHook(CancelToken);
    impl ToolCallHooks for CancelHook {
        fn before_call(&self, _: &str, _: &str) {
            self.0.cancel();
        }
        fn after_call(&self, _: &str, _: Option<&str>, _: Option<&str>) {}
    }
    let ws = tempfile::tempdir().unwrap();
    let mut app = test_plugin_with_context(ws.path(), 1578002);
    register_echo_provider(1578002, &["cancel.hook"], 1578002);
    grant_through_sheet(&mut app, "cancel.hook");
    let mut dispatcher = app.gated_dispatcher();
    dispatcher.set_hooks(Arc::new(CancelHook(app.turn_cancel.clone())));
    let result = dispatch_on_worker(Arc::new(dispatcher), "cancel.hook")
        .recv_timeout(crate::testing::load_aware_timeout(
            std::time::Duration::from_secs(5),
        ))
        .expect("cancelled hook returns without dispatching");
    tool_dispatch::unregister(1578002);
    assert_eq!(result.error_code.as_deref(), Some("cancelled"));
}

#[test]
fn turn_lifecycle_plan_cancels_isolated_approval_and_allows_next_request() {
    struct AskingBroker;
    impl AiBroker for AskingBroker {
        fn dispatch(
            &self,
            request: AiBrokerRequest,
            _: &mut dyn FnMut(TurnDelta<'_>),
        ) -> AiBrokerResponse {
            let result = request.tool_dispatcher.unwrap().dispatch_call(
                "cli-ask".into(),
                "cancel.cli",
                "{}".into(),
            );
            assert!(result.error.is_some(), "cancelled tool must not run");
            AiBrokerResponse::ok("cancelled".into(), 0, 0)
        }
    }
    let ws = tempfile::tempdir().unwrap();
    let mut app = AssistantView::new(ws.path().into(), Arc::new(AskingBroker), ws.path(), 1578003);
    register_echo_provider(1578003, &["cancel.cli"], 1578003);
    app.dispatch_model_turn(
        "cli".into(),
        "cli-turn".into(),
        "hello".into(),
        Some(Vec::new()),
        true,
        LedgerTags {
            client: None,
            kind: None,
        },
    );
    pump_until(&mut app, "isolated approval", |a| {
        a.model.pending_permission.is_some()
    });
    assert!(app.permission_owner.is_some());
    let path = ws.path().join("agents/settings.toml");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, "[permissions]\ndefault_posture = \"plan\"\n").unwrap();
    app.reload_settings();
    assert!(app.model.pending_permission.is_none());
    assert!(app.permission_owner.is_none());
    pump_until(&mut app, "isolated cancellation completion", |a| {
        a.side_in_flight.is_none()
    });
    app.submit_external_turn(
        "next request".into(),
        "next".into(),
        ws.path()
            .join("next-response.json")
            .to_string_lossy()
            .into_owned(),
        crate::app::view_trait::ExternalTurnOpts {
            conversation_id: None,
            join_desktop: false,
            status_for: None,
            client: None,
            kind: None,
        },
    )
    .unwrap();
    assert!(
        app.side_in_flight.is_some(),
        "next request must dispatch, not wait on a dead sheet"
    );
    pump_until(&mut app, "next request completion", |a| {
        a.side_in_flight.is_none()
    });
    tool_dispatch::unregister(1578003);
}

#[test]
fn turn_lifecycle_settings_watch_cancels_without_command() {
    let logs = crate::testing::test_log::capture();
    let ws = tempfile::tempdir().unwrap();
    let mut app = test_app(ws.path());
    let wake: Arc<dyn crate::app::ui_mailbox::UiWake> =
        Arc::new(crate::app::ui_mailbox::RecordingWake::new());
    app.service_external(&wake);
    arm_lifecycle_settings_watch();
    app.model.streaming.in_flight = true;
    let dispatcher = app.gated_dispatcher();
    let path = ws.path().join("agents/settings.toml");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, "[permissions]\ndefault_posture = \"plan\"\n").unwrap();
    let deadline = std::time::Instant::now()
        + crate::testing::load_aware_timeout(std::time::Duration::from_secs(5));
    while !app.turn_cancel.is_cancelled() && std::time::Instant::now() < deadline {
        app.service_external(&wake);
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(
        app.turn_cancel.is_cancelled(),
        "disk change must cancel without a command: {:?}",
        logs.lines()
    );
    assert_eq!(
        dispatcher
            .dispatch_call("watched".into(), HOST_TOOL_PANES_LIST, "{}".into())
            .error_code
            .as_deref(),
        Some("cancelled")
    );
}

#[test]
fn turn_lifecycle_completed_turn_replacement_and_drop_are_quiet() {
    let ws = tempfile::tempdir().unwrap();
    let mut app = test_app(ws.path());
    app.model.composer = "first".into();
    let effects = app.model.submit();
    app.execute_effects(effects);
    wait_for_turn(&mut app);
    let logs = crate::testing::test_log::capture();
    app.model.composer = "second".into();
    let effects = app.model.submit();
    app.execute_effects(effects);
    wait_for_turn(&mut app);
    drop(app);
    assert!(
        !logs
            .lines()
            .iter()
            .any(|line| line.contains("turn cancelled")),
        "{:?}",
        logs.lines()
    );
}

#[test]
fn turn_lifecycle_live_cancel_logs_once_with_identity() {
    let logs = crate::testing::test_log::capture();
    let token = CancelToken::for_turn("1597-live");
    token.cancel_with_reason("test live cancellation");
    token.cancel_with_reason("second cancel");
    let lines: Vec<_> = logs
        .lines()
        .into_iter()
        .filter(|line| line.contains("turn cancelled"))
        .collect();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains("turn_id=1597-live reason=test live cancellation"));
}

#[test]
fn turn_lifecycle_cancel_after_committed_allow_returns_cancelled() {
    struct CommitThenCancel {
        monitor: Arc<crate::broker::gate::PermissionMonitor>,
        cancel: CancelToken,
    }
    impl tool_dispatch::PermissionPresenter for CommitThenCancel {
        fn present(
            &self,
            pending: &str,
            _: &str,
            _: &str,
            _: &str,
            _: &str,
        ) -> Result<crate::broker::gate::ApprovalChoice, String> {
            let choice = crate::broker::gate::ApprovalChoice::Once;
            self.monitor.approve_pending(pending, choice)?;
            self.cancel.cancel();
            Ok(choice)
        }
    }
    let ws = tempfile::tempdir().unwrap();
    let app = test_app(ws.path());
    let mut dispatcher = app.gated_dispatcher();
    dispatcher.set_presenter(Arc::new(CommitThenCancel {
        monitor: app.monitor.clone(),
        cancel: app.turn_cancel.clone(),
    }));
    dispatcher.add_host_tools(Vec::new(), Arc::new(|_, _| panic!("cancelled tool ran")));
    let logs = crate::testing::test_log::capture();
    let result = dispatcher.dispatch_call("late-allow".into(), HOST_TOOL_PANES_LIST, "{}".into());
    assert_eq!(
        result.error_code.as_deref(),
        Some("cancelled"),
        "{result:?}"
    );
    assert!(
        !logs
            .records()
            .iter()
            .any(|(level, _)| *level == log::Level::Error),
        "{:?}",
        logs.lines()
    );
}

#[test]
fn turn_lifecycle_settings_watch_hidden_host_all_scopes() {
    let logs = crate::testing::test_log::capture();
    for scope in ["user", "workspace", "local"] {
        let mut h = crate::testing::HostHarness::new();
        let pane = h.add_assistant_pane();
        let (path, cancel, dispatcher) = {
            let app = h.assistant_mut(pane);
            app.model.streaming.in_flight = true;
            let path = if scope == "user" {
                app.profile_dir.join("agents/settings.toml")
            } else {
                app.workspace_root
                    .join(crate::config::workspace_channel_dir())
                    .join("agents")
                    .join(if scope == "local" {
                        "settings.local.toml"
                    } else {
                        "settings.toml"
                    })
            };
            (path, app.turn_cancel.clone(), app.gated_dispatcher())
        };
        h.hidden_frame();
        arm_lifecycle_settings_watch();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let temp = path.with_extension("new");
        std::fs::write(&temp, "[permissions]\ndefault_posture = \"plan\"\n").unwrap();
        std::fs::rename(temp, path).unwrap();
        let deadline = std::time::Instant::now()
            + crate::testing::load_aware_timeout(std::time::Duration::from_secs(5));
        while !cancel.is_cancelled() && std::time::Instant::now() < deadline {
            h.hidden_frame();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(
            cancel.is_cancelled(),
            "{scope} must reload on a hidden logic pass: {:?}",
            logs.lines()
        );
        assert_eq!(
            dispatcher
                .dispatch_call("hidden-watch".into(), HOST_TOOL_PANES_LIST, "{}".into())
                .error_code
                .as_deref(),
            Some("cancelled")
        );
    }
}

#[test]
fn turn_lifecycle_settings_watch_failure_backs_off() {
    let ws = tempfile::tempdir().unwrap();
    let mut app = test_app(ws.path());
    let logs = crate::testing::test_log::capture();
    let now = std::time::Instant::now();
    let mut attempts = 0;
    let mut fail = |_: &Path, _: Option<std::ffi::OsString>| {
        attempts += 1;
        Err("injected watcher failure".to_owned())
    };
    assert!(app.service_settings_watches(now, &mut fail));
    for _ in 0..10 {
        assert!(
            !app.service_settings_watches(now, &mut fail),
            "must not reload or retry during backoff"
        );
    }
    assert!(!app.service_settings_watches(
        now + std::time::Duration::from_millis(999),
        &mut fail,
    ));
    assert_eq!(attempts, 3);
    assert_eq!(
        logs.lines()
            .iter()
            .filter(|line| line.contains("injected watcher failure"))
            .count(),
        3
    );
    assert!(
        app.service_settings_watches(now + std::time::Duration::from_secs(1), |_, _| {
            attempts += 1;
            Err("injected retry failure".to_owned())
        })
    );
    assert_eq!(attempts, 6);
    let wake: Arc<dyn crate::app::ui_mailbox::UiWake> =
        Arc::new(crate::app::ui_mailbox::RecordingWake::new());
    assert!(app.service_settings_watches(
        now + std::time::Duration::from_secs(2),
        |parent, name| {
            crate::host::path_watch::PathWatch::new(parent, name, wake.clone(), "test")
        }
    ));
    assert_eq!(app.settings_watches.len(), 3);
    assert!(
        !app.service_settings_watches(now + std::time::Duration::from_secs(3), |_, _| panic!(
            "healthy watches must be retained"
        ))
    );
}

fn arm_lifecycle_settings_watch() {
    // FSEvents registration is asynchronous, as in PathWatch's arm helper.
    std::thread::sleep(crate::testing::load_aware_timeout(
        std::time::Duration::from_millis(200),
    ));
}

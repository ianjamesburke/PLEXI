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
    app.turn_cancel.cancel();
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
    app.model
        .permission_requested_scoped("tool", "{}", "actor", "resource", "pending");
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    app.pending_reply = Some(tx);
    emit_move(&timeline, PluginEventActor::User, None, None);
    app.pump_deliveries();
    assert!(app.turn_cancel.is_cancelled());
    assert!(matches!(rx.try_recv(), Ok(PermissionReply::Deny)));
    assert!(app.model.pending_permission.is_none());
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

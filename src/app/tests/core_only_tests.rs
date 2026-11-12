use crate::testing::HostHarness;

fn profile(core_only: bool) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("config.toml"),
        format!("core_only = {core_only}\n"),
    )
    .unwrap();
    dir
}

#[test]
fn core_only_refuses_native_and_path_launch_without_allocating_a_view() {
    let profile = profile(true);
    let mut h = HostHarness::from_profile_dir(profile.path());
    for id in ["agents", "text-editor", "file_browser", "assistant", "calc"] {
        let result = h.app.launch_view_by_id_with_layout(id, None, &[], None);
        assert!(result.is_err(), "{id} must be refused: {result:?}");
        let error = result.unwrap_err();
        assert!(
            error.contains(id) && error.contains("core-only mode"),
            "{error}"
        );
    }
    let result = h
        .app
        .launch_view_by_path_with_layout("/missing/plugin", None, None, &[]);
    assert!(result.unwrap_err().contains("core-only mode"));
    assert!(h.app.windows[0].panes.is_empty());
}

#[test]
fn core_only_normal_start_keeps_native_launch() {
    let profile = profile(false);
    let mut h = HostHarness::from_profile_dir(profile.path());
    assert!(h
        .app
        .launch_view_by_id_with_layout("agents", None, &[], None)
        .is_ok());
    assert_eq!(h.app.windows[0].panes.len(), 1);
}

#[test]
fn core_only_restore_save_normal_restart_preserves_plugin_state() {
    let profile = profile(false);
    let saved;
    {
        let mut h = HostHarness::from_profile_dir(profile.path());
        h.app
            .launch_view_by_id_with_layout("agents", None, &[], None)
            .unwrap();
        assert!(h.app.save_workspace_now());
        saved = crate::workspace::WorkspaceFile::load_from(profile.path()).unwrap();
    }
    std::fs::write(profile.path().join("config.toml"), "core_only = true\n").unwrap();
    {
        let mut h = HostHarness::from_profile_dir(profile.path());
        let pane = h.app.windows[0]
            .panes
            .values()
            .next()
            .unwrap()
            .as_view()
            .unwrap();
        assert_eq!(pane.runtime.type_id(), "disabled-plugin-view");
        assert!(!pane.runtime.needs_background_tick());
        h.app.force_reload_focused_view();
        assert_eq!(
            h.app.plugin_notice.as_deref(),
            Some(crate::app::core_only::unavailable("agents").as_str())
        );
        assert!(h.app.save_workspace_now());
        let after = crate::workspace::WorkspaceFile::load_from(profile.path()).unwrap();
        assert_eq!(
            serde_json::to_value(&saved.windows[0].panes).unwrap(),
            serde_json::to_value(&after.windows[0].panes).unwrap()
        );
    }
    std::fs::write(profile.path().join("config.toml"), "core_only = false\n").unwrap();
    let h = HostHarness::from_profile_dir(profile.path());
    let pane = h.app.windows[0]
        .panes
        .values()
        .next()
        .unwrap()
        .as_view()
        .unwrap();
    assert_eq!(pane.runtime.type_id(), "agents");
}

#[test]
fn core_only_registry_stays_empty_after_rescan_and_context_changes() {
    for enabled in [false, true] {
        let profile = profile(enabled);
        let mut h = HostHarness::from_profile_dir(profile.path());
        let root = tempfile::tempdir().unwrap();
        let plugin = root
            .path()
            .join(crate::config::workspace_channel_dir())
            .join("apps/probe");
        std::fs::create_dir_all(&plugin).unwrap();
        std::fs::write(plugin.join("manifest.toml"), "schema_version = 2\n\n[plugin]\nid = 'probe'\nname = 'Probe'\nversion = '0.1.0'\n\n[runtime]\nkind = \"python\"\nentry = 'run.sh'\n\n[[contributes.views]]\nid = \"probe.main\"\ndefault = true\n").unwrap();
        std::fs::write(plugin.join("run.sh"), "#!/bin/sh\nexit 0\n").unwrap();
        let registry = h.app.registries.rescan_root(root.path());
        assert_eq!(registry.get("probe").is_none(), enabled);
        assert_eq!(
            h.app
                .registries
                .view_for_root(root.path())
                .list()
                .is_empty(),
            enabled
        );
        assert!(h.app.background_apps.is_empty());
        if enabled {
            assert!(h.app.registries.roots().is_empty());
        }
    }
}

#[test]
fn core_only_shortcuts_refuse_before_creating_files_and_report_even_without_notifications() {
    let profile = profile(true);
    let mut h = HostHarness::from_profile_dir(profile.path());
    h.app.notifications_enabled = false;
    h.app.open_scratchpad();
    h.app.open_quick_note_modal();
    h.app.open_notes_picker();
    h.app.open_secrets_manager();
    h.app.open_file_browser();
    h.app.open_assistant_pane();
    h.app
        .open_config_in_editor(profile.path().join("config.toml"));
    assert!(h.app.windows[0].panes.is_empty());
    assert!(h
        .app
        .plugin_notice
        .as_ref()
        .unwrap()
        .contains("core-only mode"));
    assert!(!h
        .app
        .focus_stack
        .contains(&crate::app::FocusKind::QuickNote));
    assert!(!h
        .app
        .focus_stack
        .contains(&crate::app::FocusKind::NotesPicker));
}

#[test]
fn core_only_status_reports_startup_mode_after_config_is_changed() {
    let profile = profile(true);
    let mut h = HostHarness::from_profile_dir(profile.path());
    std::fs::write(profile.path().join("config.toml"), "core_only = false\n").unwrap();
    let response = profile.path().join("status.json");
    h.inject_ipc(crate::protocol::HostRequest::GetBuildInfo {
        response_file: response.to_string_lossy().into_owned(),
    });
    h.run_frames(2);
    let value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(response).unwrap()).unwrap();
    assert_eq!(value["core_only"], true);
}

#[test]
fn core_only_profile_mode_ignores_workspace_and_live_reload() {
    let profile = profile(true);
    let mut h = HostHarness::from_profile_dir(profile.path());
    let root = tempfile::tempdir().unwrap();
    let workspace_config = root
        .path()
        .join(crate::config::workspace_channel_dir())
        .join("config.toml");
    std::fs::create_dir_all(workspace_config.parent().unwrap()).unwrap();
    std::fs::write(&workspace_config, "core_only = false\n").unwrap();
    let config = crate::config::PlexiConfig::load_with_workspace(Some(root.path()));
    assert_eq!(config.core_only, Some(true));
    std::fs::write(profile.path().join("config.toml"), "core_only = false\n").unwrap();
    h.app.reload_config_for_workspace(Some(root.path()));
    assert_eq!(h.app.config.core_only, Some(true));
    assert!(h
        .app
        .launch_view_by_id_with_layout("agents", None, &[], None)
        .is_err());
}

#[test]
fn core_only_external_restore_keeps_launch_and_opaque_state_without_touching_package() {
    let profile = profile(false);
    {
        let mut h = HostHarness::from_profile_dir(profile.path());
        h.app
            .launch_view_by_id_with_layout("agents", None, &[], None)
            .unwrap();
        assert!(h.app.save_workspace_now());
    }
    let mut saved = crate::workspace::WorkspaceFile::load_from(profile.path()).unwrap();
    let pane = &mut saved.windows[0].panes[0];
    pane.plugin_id = Some("external-probe".into());
    pane.runtime_kind = Some("python-wasm".into());
    pane.plugin_state = Some(serde_json::json!({"document":"keep me", "cursor":17}));
    pane.launch = Some(crate::workspace::SavedViewLaunch::Python {
        plugin_dir: profile.path().join("deliberately-absent-plugin"),
        args: vec!["--keep".into()],
    });
    saved.save_to(profile.path()).unwrap();
    std::fs::write(profile.path().join("config.toml"), "core_only = true\n").unwrap();
    let mut h = HostHarness::from_profile_dir(profile.path());
    assert_eq!(
        h.app.windows[0]
            .panes
            .values()
            .next()
            .unwrap()
            .as_view()
            .unwrap()
            .runtime
            .type_id(),
        "disabled-plugin-view"
    );
    h.run_frames(3);
    h.render_png(crate::ui_tests::evidence_png!()).unwrap();
    assert!(h.app.save_workspace_now());
    let after = crate::workspace::WorkspaceFile::load_from(profile.path()).unwrap();
    assert_eq!(
        serde_json::to_value(&after.windows[0].panes).unwrap(),
        serde_json::to_value(&saved.windows[0].panes).unwrap()
    );
}

#[test]
#[ignore = "requires-pty"]
fn core_only_terminal_executes_and_splits() {
    let _shell = crate::host::shell::set_test_shell_override("/bin/sh");
    let profile = profile(true);
    let mut h = HostHarness::from_profile_dir(profile.path());
    let terminal = h
        .app
        .seed_root_pane(
            None,
            false,
            Some(h.workspace_root().to_path_buf()),
            false,
            false,
        )
        .expect("core-only startup must create a terminal");
    h.focus_pane(terminal);
    h.run_frames(3);
    let marker = h.workspace_root().join("terminal-executed");
    h.terminal_backend(terminal)
        .process_command(egui_term::BackendCommand::Write(
            format!("printf success > '{}'\n", marker.display()).into_bytes(),
        ));
    let deadline = std::time::Instant::now()
        + crate::testing::load_aware_timeout(std::time::Duration::from_secs(10));
    while !marker.exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "terminal command did not execute"
        );
        h.run_frames(1);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(std::fs::read_to_string(marker).unwrap(), "success");
    h.app
        .launch_view_by_id_with_layout("terminal", Some("split_v".into()), &[], None)
        .unwrap();
    assert_eq!(
        h.app.windows[0]
            .panes
            .values()
            .filter(|p| p.as_terminal().is_some())
            .count(),
        2
    );
    assert!(h.app.windows[0]
        .panes
        .values()
        .all(|p| p.as_view().is_none()));
}

/// Stint 1210: the core-only notice owns the keyboard while it is up, Esc and
/// Enter each dismiss it, and input returns to the pane it came from.
#[test]
fn core_only_notice_takes_focus_and_escape_or_enter_dismisses_it() {
    use crate::app::input_owner::{InputOwner, OverlaySurface};
    for key in [egui::Key::Escape, egui::Key::Enter] {
        let profile = profile(true);
        let mut h = HostHarness::from_profile_dir(profile.path());
        let pane = h.add_test_pane();
        h.focus_pane(pane);
        h.run_frames(1);
        let ctx = h.app.ctx.clone();
        assert_eq!(h.app.input_owner(&ctx), InputOwner::Pane(pane));

        h.app.open_notes_picker();
        h.run_frames(1);
        assert!(
            h.app.plugin_notice.is_some(),
            "Notes is refused in core-only mode"
        );
        assert_eq!(
            h.app.input_owner(&ctx),
            InputOwner::Overlay(OverlaySurface::Layer(crate::app::FocusKind::PluginNotice)),
            "the notice owns the keyboard while it is up"
        );

        h.press_key(key, egui::Modifiers::NONE);
        assert!(
            h.app.plugin_notice.is_none(),
            "{key:?} must dismiss the notice"
        );
        assert!(!h
            .app
            .focus_stack
            .contains(&crate::app::FocusKind::PluginNotice));
        assert_eq!(
            h.app.input_owner(&ctx),
            InputOwner::Pane(pane),
            "{key:?}: input returns to the pane under the notice"
        );
    }
}

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
        let result = h.app.launch_app_by_id_with_layout(id, None, &[], None);
        assert!(result.is_err(), "{id} must be refused: {result:?}");
        let error = result.unwrap_err();
        assert!(
            error.contains(id) && error.contains("core-only mode"),
            "{error}"
        );
    }
    let result = h
        .app
        .launch_app_by_path_with_layout("/missing/plugin", None, None, &[]);
    assert!(result.unwrap_err().contains("core-only mode"));
    assert!(h.app.windows[0].panes.is_empty());
}

#[test]
fn core_only_normal_start_keeps_native_launch() {
    let profile = profile(false);
    let mut h = HostHarness::from_profile_dir(profile.path());
    assert!(h
        .app
        .launch_app_by_id_with_layout("agents", None, &[], None)
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
            .launch_app_by_id_with_layout("agents", None, &[], None)
            .unwrap();
        assert!(h.app.save_workspace_now());
        saved = crate::workspace::WorkspaceFile::load_from(profile.path()).unwrap();
    }
    std::fs::write(profile.path().join("config.toml"), "core_only = true\n").unwrap();
    {
        let h = HostHarness::from_profile_dir(profile.path());
        let pane = h.app.windows[0]
            .panes
            .values()
            .next()
            .unwrap()
            .as_app()
            .unwrap();
        assert_eq!(pane.runtime.type_id(), "disabled-plugin-view");
        assert!(!pane.runtime.needs_background_tick());
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
        .as_app()
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
        std::fs::write(plugin.join("manifest.toml"), "schema_version = 1\n[app]\nid = 'probe'\ntype = 'app'\nname = 'Probe'\nversion = '0.1.0'\nentry = 'run.sh'\n").unwrap();
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
    h.inject_ipc(crate::protocol::AppRequest::GetBuildInfo {
        response_file: response.to_string_lossy().into_owned(),
    });
    h.run_frames(2);
    let value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(response).unwrap()).unwrap();
    assert_eq!(value["core_only"], true);
}

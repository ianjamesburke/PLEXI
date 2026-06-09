use super::super::*;

#[test]
fn load_app_state_returns_empty_when_no_files() {
    let dir = tempfile::tempdir().expect("tempdir");
    let result = load_app_state("test-app", dir.path());
    assert_eq!(result, serde_json::Value::Object(serde_json::Map::new()));
}

#[test]
fn load_app_state_reads_workspace_file_over_global() {
    let ws_dir = tempfile::tempdir().expect("workspace tempdir");
    let channel_dir = crate::config::workspace_channel_dir();
    let state_dir = ws_dir.path().join(&channel_dir).join("app_states");
    std::fs::create_dir_all(&state_dir).expect("mkdir");
    let state_path = state_dir.join("my-app.json");
    std::fs::write(&state_path, r#"{"interval_idx":3}"#).expect("write");

    let result = load_app_state("my-app", ws_dir.path());
    assert_eq!(result["interval_idx"], serde_json::json!(3));
}

#[test]
fn load_app_state_migrates_old_app_state_dir() {
    let ws_dir = tempfile::tempdir().expect("workspace tempdir");
    let channel_dir = crate::config::workspace_channel_dir();
    let old_dir = ws_dir.path().join(&channel_dir).join("app_state");
    std::fs::create_dir_all(&old_dir).expect("mkdir");
    std::fs::write(old_dir.join("my-app.json"), r#"{"migrated":true}"#).expect("write");

    let result = load_app_state("my-app", ws_dir.path());
    assert_eq!(result["migrated"], serde_json::json!(true));
    // Old dir must be gone, new dir must exist.
    assert!(
        !old_dir.exists(),
        "old app_state dir should have been renamed"
    );
    assert!(ws_dir.path().join(&channel_dir).join("app_states").exists());
}

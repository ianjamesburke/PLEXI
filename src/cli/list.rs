pub fn list_cli() -> i32 {
    let cwd = std::env::current_dir().unwrap_or_default();
    let registry = crate::app_registry::AppRegistry::load(&cwd);
    let installed = registry.list();
    if installed.is_empty() {
        println!("no apps installed");
        println!("install one with: plexi app install <source>[@ref]");
        return 0;
    }
    // Read versions directly from the global apps dir for the source-of-truth
    // version field — the registry only carries `manifest.version` at load time.
    let global_versions = crate::install::installed_versions(&crate::app_registry::apps_dir());
    let workspace_root = crate::app_registry::resolve_workspace_root(&cwd);
    let core_ids = crate::install::core_pack_ids();
    let example_ids = crate::install::examples_pack_ids();
    let workspace_ids = workspace_root
        .as_ref()
        .map(|r| crate::install::workspace_manifest_ids(r))
        .unwrap_or_default();
    let mut globals: Vec<(String, String, String, &'static str)> = Vec::new();
    let mut workspace: Vec<(String, String, String, &'static str)> = Vec::new();
    for app in installed {
        let version = global_versions
            .get(&app.manifest.id)
            .cloned()
            .unwrap_or_else(|| app.manifest.version.clone());
        let badge = if core_ids.contains(app.manifest.id.as_str()) {
            "[core]"
        } else if example_ids.contains(app.manifest.id.as_str()) {
            "[example]"
        } else if workspace_ids.contains(app.manifest.id.as_str()) {
            "[workspace]"
        } else {
            ""
        };
        let row = (app.manifest.id.clone(), app.manifest.name.clone(), version, badge);
        match app.source {
            crate::app_registry::RegistrySource::Global => globals.push(row),
            crate::app_registry::RegistrySource::LocalApp
            | crate::app_registry::RegistrySource::LocalAgent => workspace.push(row),
        }
    }
    if !globals.is_empty() {
        println!("Global apps ({})", crate::app_registry::apps_dir().display());
        for (id, name, version, badge) in &globals {
            if badge.is_empty() {
                println!("  {:30} {:30} {}", id, name, version);
            } else {
                println!("  {:30} {:30} {}  {}", id, name, version, badge);
            }
        }
    }
    if !workspace.is_empty() {
        if let Some(root) = workspace_root {
            println!();
            println!("Workspace apps ({})", root.display());
            for (id, name, version, badge) in &workspace {
                if badge.is_empty() {
                    println!("  {:30} {:30} {}", id, name, version);
                } else {
                    println!("  {:30} {:30} {}  {}", id, name, version, badge);
                }
            }
        }
    }
    0
}

/// `plexi app freeze <path>` — write a `pack.toml` snapshot of installed apps to `path`.
/// See `crate::install::export_pack` for the source-spec inference rules.
pub fn freeze_cli(dest_path: &str) -> i32 {
    let target_root = crate::app_registry::apps_dir();
    let dest = std::path::PathBuf::from(dest_path);
    match crate::install::export_pack(&target_root, &dest) {
        Ok(n) => {
            println!("wrote {n} apps → {}", dest.display());
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

/// Parse a single `--choice` argument into `(key, label, host_action)`.
///
/// Accepted formats:
/// - `key:Label`                            → (key, Label, None)
/// - `Label:action_type:action_arg`         → (Label, Label, Some("action_type:action_arg"))
/// - `key:Label:action_type:action_arg`     → (key, Label, Some("action_type:action_arg"))
///
/// Supported host action types:
/// - `pane_focus:<pane_id>` — navigate to the given pane when clicked
/// - `snooze:<seconds>`     — re-deliver the notification after N seconds (CLI stays blocked)
///
/// Any other segment count is rejected with a clear error string.
pub fn parse_notify_choice(raw: &str) -> Result<(String, String, Option<String>), String> {
    let segments: Vec<&str> = raw.splitn(5, ':').collect();
    match segments.as_slice() {
        [key, label, action_type, action_arg] => Ok((
            key.to_string(),
            label.to_string(),
            Some(format!("{action_type}:{action_arg}")),
        )),
        [label, action_type, action_arg] => {
            let label_str = label.to_string();
            Ok((label_str.clone(), label_str, Some(format!("{action_type}:{action_arg}"))))
        }
        [key, label] => Ok((key.to_string(), label.to_string(), None)),
        _ => Err(format!(
            "error: --choice requires 2, 3, or 4 colon-separated segments \
             (key:Label / Label:action:arg / key:Label:action:arg) — got {} in {:?}",
            segments.len(),
            raw
        )),
    }
}

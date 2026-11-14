/// `plexi plugin list`: the human table, or with `--json` one JSON array of
/// loaded and skipped plugins (stint 1488).
pub fn list_cli(json: bool) -> i32 {
    if super::host::plugin_mode_core_only() {
        println!(
            "{}",
            if json {
                "[]"
            } else {
                "No plugins available: core-only mode is on."
            }
        );
        return 0;
    }
    let cwd = std::env::current_dir().unwrap_or_default();
    let registry = crate::app::registry::PluginRegistry::load_with_global(
        &cwd,
        &crate::app::registry::plugins_dir(),
    );
    let installed = registry.list();
    let broken = skipped_plugin_lines(registry.skipped());
    log::info!(
        "plugin_list: loaded={} skipped={}",
        installed.len(),
        registry.skipped().len()
    );
    if installed.is_empty() && !json {
        println!("no plugins installed");
        println!("install one with: plexi plugin install <source>[@ref]");
        print_lines(&broken);
        return 0;
    }
    // Read versions directly from the global plugins dir for the source-of-truth
    // version field — the registry only carries `manifest.version` at load time.
    let global_versions =
        crate::cli::install_host::installed_versions(&crate::app::registry::plugins_dir());
    let workspace_root = crate::app::registry::resolve_workspace_root(&cwd);
    let core_ids = crate::cli::install_host::core_pack_ids();
    let example_ids = crate::cli::install_host::examples_pack_ids();
    let workspace_ids = workspace_root
        .as_ref()
        .map(|r| crate::cli::install_host::workspace_manifest_ids(r))
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
        let row = (
            app.manifest.id.clone(),
            app.manifest.name.clone(),
            version,
            badge,
        );
        match app.source {
            crate::app::registry::RegistrySource::Global => globals.push(row),
            crate::app::registry::RegistrySource::LocalPlugin
            | crate::app::registry::RegistrySource::LocalAgent => workspace.push(row),
        }
    }
    if json {
        let entries = plugin_list_json(&globals, &workspace, registry.skipped());
        match serde_json::to_string_pretty(&entries) {
            Ok(text) => {
                println!("{text}");
                return 0;
            }
            Err(error) => {
                eprintln!("error: plugin list could not encode JSON: {error}");
                return 1;
            }
        }
    }
    if !globals.is_empty() {
        println!(
            "Global plugins ({})",
            crate::app::registry::plugins_dir().display()
        );
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
            println!("Workspace plugins ({})", root.display());
            for (id, name, version, badge) in &workspace {
                if badge.is_empty() {
                    println!("  {:30} {:30} {}", id, name, version);
                } else {
                    println!("  {:30} {:30} {}  {}", id, name, version, badge);
                }
            }
        }
    }
    print_lines(&broken);
    0
}

type ListRow = (String, String, String, &'static str);

/// One JSON object per plugin: loaded ones with `status: "ok"`, skipped ones
/// with `status: "skipped"`, the directory and the error that skipped them.
fn plugin_list_json(
    globals: &[ListRow],
    workspace: &[ListRow],
    skipped: &[crate::app::registry::SkippedPlugin],
) -> serde_json::Value {
    let loaded = |rows: &[ListRow], scope: &str| -> Vec<serde_json::Value> {
        rows.iter()
            .map(|(id, name, version, badge)| {
                serde_json::json!({
                    "id": id,
                    "name": name,
                    "version": version,
                    "scope": scope,
                    "pack": badge.trim_matches(['[', ']']).to_string(),
                    "status": "ok",
                })
            })
            .collect()
    };
    let mut entries = loaded(globals, "global");
    entries.extend(loaded(workspace, "workspace"));
    entries.extend(skipped.iter().map(|plugin| {
        let id = plugin
            .dir
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| plugin.dir.display().to_string());
        serde_json::json!({
            "id": id,
            "path": plugin.dir.display().to_string(),
            "scope": match plugin.source {
                crate::app::registry::RegistrySource::Global => "global",
                _ => "workspace",
            },
            "status": "skipped",
            "error": plugin.error,
        })
    }));
    serde_json::Value::Array(entries)
}

fn print_lines(lines: &[String]) {
    for line in lines {
        println!("{line}");
    }
}

/// The `plexi plugin list` section for plugins the registry found but could
/// not load: each directory, its error, and the command that diagnoses it.
/// Empty when every plugin loaded.
fn skipped_plugin_lines(skipped: &[crate::app::registry::SkippedPlugin]) -> Vec<String> {
    if skipped.is_empty() {
        return Vec::new();
    }
    let mut lines = vec![
        String::new(),
        format!("Plugins that failed to load ({})", skipped.len()),
    ];
    for plugin in skipped {
        let name = plugin
            .dir
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| plugin.dir.display().to_string());
        lines.push(format!("  {name:30} broken: {}", plugin.error));
        lines.push(format!(
            "  {:30} diagnose: plexi plugin check {}",
            "",
            plugin.dir.display()
        ));
    }
    lines
}

/// `plexi plugin freeze <path>` — write a `pack.toml` snapshot of installed plugins to `path`.
/// See `crate::cli::install_host::export_pack` for the source-spec inference rules.
pub fn freeze_cli(dest_path: &str) -> i32 {
    let target_root = crate::app::registry::plugins_dir();
    let dest = std::path::PathBuf::from(dest_path);
    match crate::cli::install_host::export_pack(&target_root, &dest) {
        Ok(n) => {
            println!("wrote {n} plugins → {}", dest.display());
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
            Ok((
                label_str.clone(),
                label_str,
                Some(format!("{action_type}:{action_arg}")),
            ))
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Stint 1260: `plexi plugin list` names a plugin that failed to load,
    /// its error, and the command that diagnoses it.
    #[test]
    fn skipped_plugins_are_listed_with_their_error_and_the_check_command() {
        let skipped = [crate::app::registry::SkippedPlugin {
            dir: std::path::PathBuf::from("/profile/plugins/broken-app"),
            source: crate::app::registry::RegistrySource::Global,
            error: "invalid manifest: missing field `runtime`".into(),
        }];
        let text = skipped_plugin_lines(&skipped).join("\n");
        assert!(text.contains("broken-app"), "{text}");
        assert!(
            text.contains("invalid manifest: missing field `runtime`"),
            "{text}"
        );
        assert!(
            text.contains("plexi plugin check /profile/plugins/broken-app"),
            "{text}"
        );
        assert!(skipped_plugin_lines(&[]).is_empty());
    }

    /// Stint 1488: the `--json` shape, loaded and skipped plugins together.
    #[test]
    fn plugin_list_json_carries_loaded_and_skipped_plugins() {
        let globals = [(
            "notes".to_string(),
            "Notes".to_string(),
            "0.1.0".to_string(),
            "[core]",
        )];
        let skipped = [crate::app::registry::SkippedPlugin {
            dir: std::path::PathBuf::from("/profile/plugins/broken-app"),
            source: crate::app::registry::RegistrySource::Global,
            error: "invalid manifest: missing field `runtime`".into(),
        }];
        let value = plugin_list_json(&globals, &[], &skipped);
        assert_eq!(
            value,
            serde_json::json!([
                {"id": "notes", "name": "Notes", "version": "0.1.0", "scope": "global",
                 "pack": "core", "status": "ok"},
                {"id": "broken-app", "path": "/profile/plugins/broken-app", "scope": "global",
                 "status": "skipped", "error": "invalid manifest: missing field `runtime`"}
            ])
        );
    }
}

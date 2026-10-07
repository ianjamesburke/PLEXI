//! Live description of this build. The Assistant's `host.introspect` tool and
//! `plexi app info` both read from here. Nothing in this module is a maintained
//! help document: command names come from the clap tree, app tools from the
//! installed app's source, panes and permission rows from the caller.

use std::path::Path;

/// A tool exposed by a running app pane. Merged onto the declared list.
#[derive(Debug, Clone)]
pub struct LiveTool {
    pub app_id: String,
    pub pane_id: u64,
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, serde::Serialize)]
struct CliNode {
    path: String,
    about: String,
    args: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
struct DeclaredTool {
    name: String,
    description: String,
    source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pane_id: Option<u64>,
}

#[derive(Debug, Clone, serde::Serialize)]
struct InstalledApp {
    id: String,
    name: String,
    description: String,
    tools: Vec<DeclaredTool>,
}

/// Commands a person can run to change a stored permission, taken from the
/// clap tree of this binary. Callers paste this into a denial; they do not
/// keep a second copy of the command list.
pub fn permission_undo_text() -> String {
    let wanted = [
        "permissions list",
        "permissions reset",
        "permissions revoke",
        "permissions allow",
        "app open",
        "needs-you list",
    ];
    let tree = cli_tree();
    let mut found = Vec::new();
    for suffix in wanted {
        if let Some(node) = tree.iter().find(|node| node.path.ends_with(suffix)) {
            if !found.contains(&node.path) {
                found.push(node.path.clone());
            }
        }
    }
    format!(
        "A Deny choice refuses only that call. An always-deny stays stored until it is reset or allowed. Commands from this build: {}.",
        found.join(", ")
    )
}

/// Backtick citations in `answer` that do not appear in `evidence`.
#[cfg(test)]
fn ungrounded_citations(answer: &str, evidence: &str) -> Vec<String> {
    let mut bad = Vec::new();
    let mut rest = answer;
    while let Some(start) = rest.find('`') {
        rest = &rest[start + 1..];
        let Some(end) = rest.find('`') else {
            break;
        };
        let cite = &rest[..end];
        rest = &rest[end + 1..];
        if cite.is_empty() || cite.contains('\n') {
            continue;
        }
        if !evidence.contains(cite) {
            bad.push(cite.to_string());
        }
    }
    bad
}

/// CLI tree, installed apps, panes, and permission rows. `query` keeps only
/// rows whose text contains it. With no query, the CLI section is the
/// top-level commands; nested commands appear when a query names them.
pub fn assemble(
    query: Option<&str>,
    apps_dir: &Path,
    panes: &[serde_json::Value],
    permissions: &[serde_json::Value],
    live_tools: &[LiveTool],
) -> serde_json::Value {
    let query = query.map(str::trim).filter(|value| !value.is_empty());
    let cli = match query {
        Some(query) => cli_tree()
            .into_iter()
            .filter(|node| contains_query(&node.path, query) || contains_query(&node.about, query))
            .collect::<Vec<_>>(),
        None => cli_tree()
            .into_iter()
            .filter(|node| node.path.split_whitespace().count() == 2)
            .map(|mut node| {
                node.args.clear();
                node
            })
            .collect::<Vec<_>>(),
    };
    let mut apps = installed_apps(apps_dir);
    merge_live(&mut apps, live_tools);
    if let Some(query) = query {
        if !contains_query("apps", query) {
            apps.retain(|app| app_matches(app, query));
        }
    }
    let panes = rows_for("panes", panes, query);
    let permissions = rows_for("permissions", permissions, query);
    serde_json::json!({
        "ok": true,
        "query": query,
        "cli": cli,
        "apps": apps,
        "panes": panes,
        "permissions": permissions,
    })
}

/// Tools declared with `@tools.tool` under `app_dir`.
pub fn declared_tools(app_dir: &Path) -> Vec<(String, String)> {
    let mut tools = Vec::new();
    walk_py(app_dir, &mut tools);
    tools
        .into_iter()
        .map(|tool| (tool.name, tool.description))
        .collect()
}

fn cli_tree() -> Vec<CliNode> {
    let mut out = Vec::new();
    walk_command(&crate::cli::help::gated_command(), "plexi", &mut out);
    out
}

fn walk_command(cmd: &clap::Command, path: &str, out: &mut Vec<CliNode>) {
    if path != "plexi" {
        out.push(CliNode {
            path: path.to_string(),
            about: cmd
                .get_about()
                .map(|text| text.to_string())
                .unwrap_or_default(),
            args: visible_args(cmd),
        });
    }
    for sub in cmd.get_subcommands() {
        if sub.is_hide_set() {
            continue;
        }
        let child = format!("{path} {}", sub.get_name());
        walk_command(sub, &child, out);
    }
}

fn visible_args(cmd: &clap::Command) -> Vec<String> {
    cmd.get_arguments()
        .filter(|arg| !arg.is_hide_set())
        .filter(|arg| arg.get_id() != "help" && arg.get_id() != "version")
        .map(|arg| {
            if let Some(long) = arg.get_long() {
                format!("--{long}")
            } else if arg.is_positional() {
                arg.get_id().as_str().to_string()
            } else if let Some(short) = arg.get_short() {
                format!("-{short}")
            } else {
                arg.get_id().as_str().to_string()
            }
        })
        .collect()
}

fn installed_apps(apps_dir: &Path) -> Vec<InstalledApp> {
    let mut apps = Vec::new();
    let entries = match std::fs::read_dir(apps_dir) {
        Ok(entries) => entries,
        Err(error) => {
            log::info!(
                "introspect: no installed apps at {}: {error}",
                apps_dir.display()
            );
            return apps;
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let manifest_path = path.join("manifest.toml");
        let Ok(raw) = std::fs::read_to_string(&manifest_path) else {
            continue;
        };
        let Ok(manifest) = toml::from_str::<crate::app::registry::AppManifest>(&raw) else {
            log::info!(
                "introspect: skipping unreadable manifest {}",
                manifest_path.display()
            );
            continue;
        };
        let mut tools = Vec::new();
        walk_py(&path, &mut tools);
        apps.push(InstalledApp {
            id: manifest.app.id,
            name: manifest.app.name,
            description: manifest.app.description,
            tools,
        });
    }
    apps.sort_by(|left, right| left.id.cmp(&right.id));
    apps
}

fn merge_live(apps: &mut Vec<InstalledApp>, live_tools: &[LiveTool]) {
    for tool in live_tools {
        if !apps.iter().any(|app| app.id == tool.app_id) {
            apps.push(InstalledApp {
                id: tool.app_id.clone(),
                name: tool.app_id.clone(),
                description: String::new(),
                tools: Vec::new(),
            });
        }
        let Some(app) = apps.iter_mut().find(|app| app.id == tool.app_id) else {
            continue;
        };
        if let Some(existing) = app.tools.iter_mut().find(|row| row.name == tool.name) {
            existing.description = tool.description.clone();
            existing.source = "live".to_string();
            existing.pane_id = Some(tool.pane_id);
        } else {
            app.tools.push(DeclaredTool {
                name: tool.name.clone(),
                description: tool.description.clone(),
                source: "live".to_string(),
                pane_id: Some(tool.pane_id),
            });
        }
    }
    apps.sort_by(|left, right| left.id.cmp(&right.id));
}

fn walk_py(dir: &Path, out: &mut Vec<DeclaredTool>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) => {
            log::info!("introspect: cannot read {}: {error}", dir.display());
            return;
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            if matches!(
                name.as_ref(),
                ".venv" | "__pycache__" | "target" | "node_modules" | ".git"
            ) {
                continue;
            }
            walk_py(&path, out);
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) != Some("py") {
            continue;
        }
        let Ok(source) = std::fs::read_to_string(&path) else {
            continue;
        };
        scan_source(&source, out);
    }
}

fn scan_source(source: &str, out: &mut Vec<DeclaredTool>) {
    let marker = "@tools.tool";
    let mut search_from = 0;
    while let Some(rel) = source[search_from..].find(marker) {
        let at = search_from + rel;
        let after = &source[at + marker.len()..];
        let Some(paren) = after.find('(') else {
            break;
        };
        if !after[..paren].chars().all(char::is_whitespace) {
            search_from = at + marker.len();
            continue;
        }
        let (name, description) = strings_before_options(&after[paren + 1..]);
        if !name.is_empty() && !out.iter().any(|tool| tool.name == name) {
            out.push(DeclaredTool {
                name,
                description,
                source: "declared".to_string(),
                pane_id: None,
            });
        }
        search_from = at + marker.len();
    }
}

/// The first two string literals of a `@tools.tool(...)` call, stopping at a
/// dict, a keyword argument, or the closing paren.
fn strings_before_options(body: &str) -> (String, String) {
    let bytes = body.as_bytes();
    let mut index = 0;
    let mut found = Vec::new();
    while found.len() < 2 && index < bytes.len() {
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if index >= bytes.len() {
            break;
        }
        match bytes[index] {
            b'"' | b'\'' => {
                let quote = bytes[index];
                index += 1;
                let start = index;
                while index < bytes.len() && bytes[index] != quote {
                    if bytes[index] == b'\\' {
                        index += 1;
                    }
                    if index < bytes.len() {
                        index += 1;
                    }
                }
                found.push(body[start..index.min(body.len())].to_string());
                if index < bytes.len() && bytes[index] == quote {
                    index += 1;
                }
            }
            b',' => index += 1,
            _ => break,
        }
    }
    let name = found.first().cloned().unwrap_or_default();
    let description = found.get(1).cloned().unwrap_or_default();
    (name, description)
}

fn app_matches(app: &InstalledApp, query: &str) -> bool {
    contains_query(&app.id, query)
        || contains_query(&app.name, query)
        || contains_query(&app.description, query)
        || app.tools.iter().any(|tool| {
            contains_query(&tool.name, query) || contains_query(&tool.description, query)
        })
}

fn rows_for(
    section: &str,
    rows: &[serde_json::Value],
    query: Option<&str>,
) -> Vec<serde_json::Value> {
    match query {
        None => rows.to_vec(),
        Some(query) if contains_query(section, query) => rows.to_vec(),
        Some(_) => filter_values(rows, query),
    }
}

fn filter_values(rows: &[serde_json::Value], query: Option<&str>) -> Vec<serde_json::Value> {
    let Some(query) = query else {
        return rows.to_vec();
    };
    rows.iter()
        .filter(|row| contains_query(&row.to_string(), query))
        .cloned()
        .collect()
}

fn contains_query(text: &str, query: &str) -> bool {
    text.to_ascii_lowercase()
        .contains(&query.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_commands_come_from_clap() {
        let text = permission_undo_text();
        assert!(text.contains("plexi permissions list"), "{text}");
        assert!(text.contains("plexi permissions reset"), "{text}");
        assert!(text.contains("plexi permissions allow"), "{text}");
        assert!(text.contains("plexi permissions revoke"), "{text}");
        assert!(!text.contains("gear"), "{text}");
        assert!(!text.contains("Cmd+Shift"), "{text}");
    }

    #[test]
    fn new_app_tools_appear_and_reenable_answer_cites_only_the_tool() {
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().join("widget");
        std::fs::create_dir(&app).unwrap();
        std::fs::write(
            app.join("manifest.toml"),
            r#"
schema_version = 1

[app]
id = "widget"
name = "Widget"
entry = "main.py"
type = "app"
version = "0.1.0"
description = "A newly installed widget."
"#,
        )
        .unwrap();
        std::fs::write(
            app.join("main.py"),
            r#"
@tools.tool(
    "widget.ping",
    "Ping the widget.",
    read_only=True,
)
def _ping():
    return {}
"#,
        )
        .unwrap();

        let panes = vec![serde_json::json!({
            "id": 4,
            "type": "app",
            "title": "Permissions",
            "app_id": "permissions"
        })];
        let permissions = vec![serde_json::json!({
            "id": "grant-deny",
            "kind": "deny",
            "tool": "chess.state",
            "actor_id": "pane:1"
        })];
        let doc = assemble(Some("permission"), dir.path(), &panes, &permissions, &[]);
        let evidence = doc.to_string();
        assert!(evidence.contains("plexi permissions reset"), "{evidence}");
        assert!(evidence.contains("plexi permissions list"), "{evidence}");
        assert!(evidence.contains("plexi permissions allow"), "{evidence}");
        assert!(evidence.contains("grant-deny"), "{evidence}");
        assert!(evidence.contains("Permissions"), "{evidence}");
        assert!(
            !evidence.contains("widget.ping"),
            "a permissions query must not invent unrelated tools: {evidence}"
        );

        let listed = assemble(Some("widget"), dir.path(), &[], &[], &[]);
        let listed_text = listed.to_string();
        assert!(listed_text.contains("widget.ping"), "{listed_text}");
        assert!(listed_text.contains("Ping the widget."), "{listed_text}");

        let reset = doc["cli"]
            .as_array()
            .unwrap()
            .iter()
            .find_map(|node| {
                let path = node["path"].as_str()?;
                path.ends_with("permissions reset")
                    .then_some(path.to_string())
            })
            .expect("reset command");
        let pane = doc["panes"][0]["title"].as_str().unwrap();
        let answer = format!("To allow that denial again, run `{reset}`. Open pane `{pane}`.");
        assert!(
            ungrounded_citations(&answer, &evidence).is_empty(),
            "{answer}"
        );
        let invented = "Click the `gear icon` or press `Cmd+Shift+P`.";
        let bad = ungrounded_citations(invented, &evidence);
        assert!(bad.iter().any(|cite| cite.contains("gear")), "{bad:?}");
        assert!(
            bad.iter().any(|cite| cite.contains("Cmd+Shift+P")),
            "{bad:?}"
        );
    }
}

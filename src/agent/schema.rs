use clap::CommandFactory;
use serde_json::{json, Value};

/// Subcommands that are too destructive or irreversible to expose as agent tools.
const DENYLIST: &[&str] = &[
    "upgrade",
    "workspace_init",
    "app_uninstall",
    "uninstall",
    "update",
    "pack_export",
    "completions",
    "shell_init",
];

/// Walk a clap Command tree and collect leaf subcommands as tool descriptors.
/// `prefix` is the flattened name so far (e.g. "pane" for `plexi pane`).
pub fn collect_tools_pub(cmd: &clap::Command, prefix: &str, tools: &mut Vec<Value>) {
    let name = if prefix.is_empty() {
        cmd.get_name().to_string()
    } else {
        format!("{}_{}", prefix, cmd.get_name())
    };

    let subcommands: Vec<_> = cmd.get_subcommands().collect();

    if subcommands.is_empty() {
        // Leaf — emit as a tool
        if DENYLIST.contains(&name.as_str()) {
            return;
        }

        let description = cmd
            .get_about()
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("Run: plexi {}", name.replace('_', " ")));

        let mut properties: serde_json::Map<String, Value> = serde_json::Map::new();
        let mut required: Vec<String> = Vec::new();

        for arg in cmd.get_arguments() {
            // Skip internal/hidden args
            if arg.is_hide_set() {
                continue;
            }
            let arg_name = arg.get_id().as_str().to_string();
            if arg_name == "help" || arg_name == "version" {
                continue;
            }

            let arg_desc = arg.get_help().map(|s| s.to_string()).unwrap_or_default();

            let arg_type = if arg.get_action().takes_values() {
                "string"
            } else {
                "boolean"
            };

            properties.insert(
                arg_name.clone(),
                json!({
                    "type": arg_type,
                    "description": arg_desc,
                }),
            );

            if arg.is_required_set() {
                required.push(arg_name);
            }
        }

        tools.push(json!({
            "type": "function",
            "name": name,
            "description": description,
            "parameters": {
                "type": "object",
                "properties": properties,
                "required": required,
            }
        }));
    } else {
        for sub in subcommands {
            let sub_prefix = if prefix.is_empty() {
                cmd.get_name().to_string()
            } else {
                name.clone()
            };
            collect_tools_pub(sub, &sub_prefix, tools);
        }
    }
}

pub fn print_schema() -> i32 {
    let cmd = crate::cli_args::Cli::command();
    let mut tools: Vec<Value> = Vec::new();

    for sub in cmd.get_subcommands() {
        collect_tools_pub(sub, "", &mut tools);
    }

    // Skip the top-level "agent" command from tool exposure
    tools.retain(|t| {
        let name = t["name"].as_str().unwrap_or("");
        !name.starts_with("agent_")
    });

    match serde_json::to_string_pretty(&tools) {
        Ok(json) => {
            println!("{json}");
            0
        }
        Err(e) => {
            eprintln!("error: failed to serialize schema: {e}");
            1
        }
    }
}

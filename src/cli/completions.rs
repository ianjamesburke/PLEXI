/// Output completion candidates for `plexi app open <prefix>`.
///
/// Called by the hidden `_complete-open` subcommand. Outputs one candidate
/// per line, ready for shell consumption.
///
/// - `"cli:"` -> embedded + cached CLI names, prefixed with `cli:`
/// - `"mcp:"` -> embedded + user MCP names, prefixed with `mcp:`
/// - `"app:"` -> installed app IDs, prefixed with `app:`
/// - `""` or anything else -> all three categories plus bare app IDs
pub fn complete_open_cli(prefix: &str) -> i32 {
    match prefix {
        "cli:" | "cli" => {
            let mut names: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
            for name in crate::cli::registry::list_clis() {
                names.insert(name);
            }
            for name in crate::cli::registry::list_cached_cli_names() {
                names.insert(name);
            }
            for name in names {
                println!("cli:{name}");
            }
        }
        "mcp:" | "mcp" => {
            for name in crate::cli::registry::list_mcp_names() {
                println!("mcp:{name}");
            }
        }
        "app:" | "app" => {
            let apps_dir = crate::config::config_dir().join("apps");
            if let Ok(read) = std::fs::read_dir(&apps_dir) {
                let mut names: Vec<String> = read
                    .flatten()
                    .filter(|e| e.path().is_dir())
                    .filter_map(|e| e.file_name().into_string().ok())
                    .collect();
                names.sort();
                for name in names {
                    println!("app:{name}");
                }
            }
        }
        _ => {
            // All categories: bare app IDs + prefixed variants
            let apps_dir = crate::config::config_dir().join("apps");
            if let Ok(read) = std::fs::read_dir(&apps_dir) {
                let mut names: Vec<String> = read
                    .flatten()
                    .filter(|e| e.path().is_dir())
                    .filter_map(|e| e.file_name().into_string().ok())
                    .collect();
                names.sort();
                for name in &names {
                    println!("{name}");
                }
            }
            // CLI names
            let mut cli_names: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
            for name in crate::cli::registry::list_clis() {
                cli_names.insert(name);
            }
            for name in crate::cli::registry::list_cached_cli_names() {
                cli_names.insert(name);
            }
            for name in cli_names {
                println!("cli:{name}");
            }
            // MCP names
            for name in crate::cli::registry::list_mcp_names() {
                println!("mcp:{name}");
            }
        }
    }
    0
}

pub fn completions_cli(shell: &str, binary_name: &str) -> i32 {
    use clap::CommandFactory;
    use clap_complete::{generate, Shell};
    let Ok(shell_variant) = shell.parse::<Shell>() else {
        eprintln!("error: unsupported shell {shell:?} — supported: bash, zsh, fish");
        return 1;
    };
    log::info!("completions: generating {:?} completions for binary {:?}", shell, binary_name);
    let mut cmd = crate::cli::args::Cli::command();
    if shell_variant == Shell::Zsh {
        // Generate to a buffer so we can post-process the zsh script.
        let mut buf = Vec::new();
        generate(shell_variant, &mut cmd, binary_name, &mut buf);
        let script = String::from_utf8(buf).unwrap_or_default();
        let script = fix_zsh_workspace_path_conflict(&script);
        let script = inject_zsh_open_completions(&script, binary_name);
        print!("{script}");
    } else {
        generate(shell_variant, &mut cmd, binary_name, &mut std::io::stdout());
    }
    0
}

/// Inject a dynamic completion function for `plexi app open <TAB>` into the
/// zsh completion script. Replaces the static `type_id` spec with a call to
/// the hidden `_complete-open` subcommand, which enumerates app IDs, CLI
/// registry names, MCP registry names, and cached descriptors at runtime.
fn inject_zsh_open_completions(script: &str, binary_name: &str) -> String {
    // The generated script has a line like:
    //   ':type_id -- App id to open ...:' \
    // We replace it with a dynamic completion that calls our binary.
    let mut result = String::with_capacity(script.len() + 512);
    let mut injected_helper = false;
    for line in script.lines() {
        if line.contains("::type_id") && line.contains("App id to open") {
            // Replace static spec with dynamic completion
            result.push_str(&format!(
                "':type_id -- App, CLI, or MCP name (app:, cli:, mcp: prefix):__{binary_name}_open_completions' \\",
            ));
            result.push('\n');
            if !injected_helper {
                injected_helper = true;
            }
        } else {
            result.push_str(line);
            result.push('\n');
        }
    }

    // Append the helper function at the end (before the final line which is
    // usually the compdef call or the last function call).
    if injected_helper {
        let helper = format!(
            r#"
__{binary_name}_open_completions() {{
    local -a completions
    completions=(${{(f)"$({binary_name} _complete-open "${{words[CURRENT]%%:*}}"  2>/dev/null)"}})
    compadd -a completions
}}
"#,
        );
        // Insert before the final compdef line
        if let Some(pos) = result.rfind("\ncompdef ") {
            result.insert_str(pos, &helper);
        } else {
            // No compdef found, append at end
            result.push_str(&helper);
        }
    }

    result
}

/// clap_complete emits `workspace_path` as a positional spec before the
/// subcommand slot. When the user types `plexi workspace <Tab>`, zsh assigns
/// "workspace" to that spec (position 1) instead of the subcommand slot
/// (position 2), so `$line[2]` is empty and no subcommand case matches.
///
/// Fix: strip the conflicting workspace_path spec and reindex the outer
/// dispatch from `$line[2]` to `$line[1]`. All nested subcommand blocks
/// already use `$line[1]`, so a global `$line[2]` → `$line[1]` substitution
/// only affects the outer dispatch.
fn fix_zsh_workspace_path_conflict(script: &str) -> String {
    let without_spec: String = script
        .lines()
        .filter(|line| !line.contains("::workspace_path"))
        .flat_map(|line| [line, "\n"])
        .collect();
    without_spec.replace("$line[2]", "$line[1]")
}

#[cfg(test)]
mod completions_tests {
    use super::fix_zsh_workspace_path_conflict;
    use clap::CommandFactory;
    use clap_complete::{generate, Shell};

    fn generated_zsh() -> String {
        let mut cmd = crate::cli::args::Cli::command();
        let mut buf = Vec::new();
        generate(Shell::Zsh, &mut cmd, "plexi", &mut buf);
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn zsh_fix_removes_workspace_path_spec() {
        let raw = generated_zsh();
        assert!(
            raw.contains("workspace_path"),
            "raw script should contain workspace_path spec"
        );
        let fixed = fix_zsh_workspace_path_conflict(&raw);
        assert!(
            !fixed.contains("workspace_path"),
            "fixed script must not contain workspace_path spec"
        );
    }

    #[test]
    fn zsh_fix_removes_line2_references() {
        let raw = generated_zsh();
        assert!(raw.contains("$line[2]"), "raw script should use line[2] for outer dispatch");
        let fixed = fix_zsh_workspace_path_conflict(&raw);
        assert!(
            !fixed.contains("$line[2]"),
            "fixed script must not contain $line[2] — outer dispatch should use $line[1]"
        );
    }

    #[test]
    fn zsh_fix_preserves_workspace_subcommand_completions() {
        let raw = generated_zsh();
        let fixed = fix_zsh_workspace_path_conflict(&raw);
        assert!(
            fixed.contains("_plexi__subcmd__workspace_commands"),
            "workspace subcommand completion function must still be present"
        );
        assert!(
            fixed.contains("(workspace)"),
            "workspace case branch must still be present"
        );
    }

    #[test]
    fn zsh_fix_is_idempotent() {
        let raw = generated_zsh();
        let once = fix_zsh_workspace_path_conflict(&raw);
        let twice = fix_zsh_workspace_path_conflict(&once);
        assert_eq!(once, twice, "fix must be idempotent");
    }
}

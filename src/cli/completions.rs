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
        print!("{}", fix_zsh_workspace_path_conflict(&script));
    } else {
        generate(shell_variant, &mut cmd, binary_name, &mut std::io::stdout());
    }
    0
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

use clap::CommandFactory;

use crate::cli::args::Cli;
use crate::release::ReleaseFeature;

const HELP_GROUPS: &[(&str, &[&str])] = &[
    (
        "Workspace",
        &["run", "workspace", "secret", "routine", "agent", "context"],
    ),
    ("Apps", &["app", "account", "registry", "events"]),
    ("Panes", &["pane", "notify"]),
    ("AI", &["ai"]),
    (
        "System",
        &[
            "completions",
            "host",
            "config",
            "notes",
            "note",
            "doctor",
            "demo",
            "update",
            "uninstall",
        ],
    ),
];

/// The clap command tree with every release-gated surface hidden for `enabled`.
///
/// Single source for help, completions, and parsing so a gated command is never
/// advertised on a channel whose execution-time gate would refuse it. Hidden
/// commands still parse, so the `exit_if_feature_disabled` backstop in `main`
/// keeps producing the standard unavailable message.
pub fn gated_command() -> clap::Command {
    gate_command(Cli::command(), crate::release::feature_enabled)
}

/// `Cli::try_parse_from` over the gated command tree.
pub fn parse_gated<I, T>(args: I) -> Result<Cli, clap::Error>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    use clap::FromArgMatches;
    let mut cmd = gated_command();
    let mut matches = cmd.try_get_matches_from_mut(args)?;
    Cli::from_arg_matches_mut(&mut matches).map_err(|e| e.format(&mut cmd))
}

pub fn gate_command(cmd: clap::Command, enabled: impl Fn(ReleaseFeature) -> bool) -> clap::Command {
    let mut cmd = cmd;
    if !enabled(ReleaseFeature::Marketplace) {
        let text = crate::release::feature_unavailable_text(ReleaseFeature::Marketplace);
        cmd = cmd
            .mut_subcommand("app", |app| {
                app.mut_subcommand("publish", |c| c.hide(true))
                    .mut_subcommand("browse", |c| c.hide(true))
                    .mut_subcommand("search", |c| c.hide(true))
            })
            .mut_subcommand("account", |account| {
                let names: Vec<String> = account
                    .get_subcommands()
                    .map(|c| c.get_name().to_string())
                    .collect();
                names
                    .into_iter()
                    .fold(account, |a, n| a.mut_subcommand(n, |c| c.hide(true)))
                    .hide(true)
                    .about(text)
                    .long_about(None::<&str>)
            });
    }
    if !enabled(ReleaseFeature::McpClient) {
        cmd = cmd.mut_subcommand("events", |events| {
            events.mut_subcommand("mcp-config", |c| c.hide(true))
        });
    }
    if !enabled(ReleaseFeature::AppWrappers) {
        cmd = cmd.mut_subcommand("app", |app| {
            app.mut_subcommand("open", |open| {
                open.mut_arg("mcp", |a| a.hide(true))
                    .mut_arg("cli", |a| a.hide(true))
            })
        });
    }
    cmd
}

pub fn print_grouped_help() {
    let cmd = gated_command();
    let no_color = std::env::var_os("NO_COLOR").is_some();

    // Apply ANSI to already-padded text so width counting is based on visible chars.
    let header = |s: &str| -> String {
        if no_color {
            format!("{s}:")
        } else {
            format!("\x1b[1;32m{s}:\x1b[0m")
        }
    };
    let lit = |s: String| -> String {
        if no_color {
            s
        } else {
            format!("\x1b[1;36m{s}\x1b[0m")
        }
    };
    let dim = |s: &str| -> String {
        if no_color {
            s.to_string()
        } else {
            format!("\x1b[2m{s}\x1b[0m")
        }
    };

    // About
    if let Some(about) = cmd.get_about() {
        println!("{about}");
        println!();
    }

    // Usage
    let bin = cmd.get_name();
    println!("{} {bin} [OPTIONS] [COMMAND]", header("Usage"));
    println!();

    // Visible positional arguments (workspace_path)
    let visible_positional: Vec<_> = cmd
        .get_arguments()
        .filter(|a| !a.is_hide_set() && a.is_positional())
        .filter(|a| a.get_id() != "help" && a.get_id() != "version")
        .collect();

    if !visible_positional.is_empty() {
        println!("{}", header("Arguments"));
        for arg in &visible_positional {
            let name = format!("[{}]", arg.get_id().as_str().to_uppercase());
            let padded = format!("{name:<22}");
            let help = arg.get_help().map(|s| s.to_string()).unwrap_or_default();
            println!("  {} {help}", lit(padded));
        }
        println!();
    }

    // Options: -h and -V only (--profile is hidden)
    println!("{}", header("Options"));
    println!("  {} Print help", lit(format!("{:<22}", "-h, --help")));
    if cmd.get_version().is_some() {
        println!(
            "  {} Print version",
            lit(format!("{:<22}", "-V, --version"))
        );
    }
    println!();

    // Grouped subcommands
    let col_width = HELP_GROUPS
        .iter()
        .flat_map(|(_, names)| names.iter())
        .filter(|&&n| cmd.find_subcommand(n).is_some_and(|s| !s.is_hide_set()))
        .map(|n| n.len())
        .max()
        .unwrap_or(10)
        + 2;

    for (heading, names) in HELP_GROUPS {
        println!("{}", header(heading));
        for &name in *names {
            if let Some(sub) = cmd.find_subcommand(name).filter(|s| !s.is_hide_set()) {
                let about = sub.get_about().map(|s| s.to_string()).unwrap_or_default();
                println!("  {} {about}", lit(format!("{name:<col_width$}")));
            }
        }
        println!();
    }

    // After-help footer
    if let Some(after) = cmd.get_after_help() {
        println!("{}", dim(&after.to_string()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::release::feature_enabled_for_channel;

    fn gated_for(channel: Option<&str>) -> clap::Command {
        gate_command(Cli::command(), |f| feature_enabled_for_channel(f, channel))
    }

    fn visible_subs(cmd: &clap::Command) -> Vec<String> {
        cmd.get_subcommands()
            .filter(|c| !c.is_hide_set())
            .map(|c| c.get_name().to_string())
            .collect()
    }

    #[test]
    fn stable_hides_marketplace_from_app_and_account_help() {
        for channel in [None, Some("main"), Some("rc-010")] {
            let cmd = gated_for(channel);
            let app = visible_subs(cmd.find_subcommand("app").unwrap());
            for gated in ["publish", "browse", "search"] {
                assert!(!app.contains(&gated.to_string()), "{channel:?}: {app:?}");
            }
            // App runtime stays v1-visible.
            for runtime in [
                "open",
                "install",
                "list",
                "uninstall",
                "render",
                "test",
                "check",
                "init",
                "trust",
                "prune",
            ] {
                assert!(app.contains(&runtime.to_string()), "{channel:?}: {runtime}");
            }
            let account = cmd.find_subcommand("account").unwrap();
            assert!(account.is_hide_set());
            assert!(visible_subs(account).is_empty());
            let mut account = account.clone();
            let help = account.render_long_help().to_string();
            assert!(!help.contains("login"), "{help}");
            assert!(help.contains("marketplace requires the beta channel"), "{help}");
        }
    }

    #[test]
    fn stable_hides_mcp_config_and_wrapper_flags_but_keeps_events() {
        let cmd = gated_for(None);
        let events = cmd.find_subcommand("events").unwrap();
        assert!(!events.is_hide_set());
        let subs = visible_subs(events);
        assert!(!subs.contains(&"mcp-config".to_string()), "{subs:?}");
        assert!(subs.contains(&"subscribe".to_string()), "{subs:?}");
        let open = cmd
            .find_subcommand("app")
            .unwrap()
            .find_subcommand("open")
            .unwrap();
        for flag in ["mcp", "cli"] {
            assert!(open.get_arguments().find(|a| a.get_id() == flag).unwrap().is_hide_set());
        }
    }

    #[test]
    fn beta_and_alpha_keep_marketplace_and_mcp_help() {
        for channel in [Some("beta"), Some("alpha"), Some("pr-2259")] {
            let cmd = gated_for(channel);
            let app = visible_subs(cmd.find_subcommand("app").unwrap());
            for shown in ["publish", "browse", "search", "open"] {
                assert!(app.contains(&shown.to_string()), "{channel:?}: {app:?}");
            }
            assert!(!cmd.find_subcommand("account").unwrap().is_hide_set());
            let events = visible_subs(cmd.find_subcommand("events").unwrap());
            assert!(events.contains(&"mcp-config".to_string()));
        }
    }

    #[test]
    fn gated_commands_still_parse_for_the_execution_backstop() {
        use clap::FromArgMatches;
        let cmd = gated_for(None);
        let mut m = cmd
            .try_get_matches_from(["plexi", "events", "mcp-config"])
            .expect("hidden command must still parse");
        assert!(Cli::from_arg_matches_mut(&mut m).is_ok());
    }

    #[test]
    fn every_visible_top_level_command_is_in_exactly_one_help_group() {
        for channel in [None, Some("alpha")] {
            let cmd = gated_for(channel);
            for sub in cmd.get_subcommands().filter(|c| !c.is_hide_set()) {
                let n = HELP_GROUPS
                    .iter()
                    .filter(|(_, names)| names.contains(&sub.get_name()))
                    .count();
                assert_eq!(n, 1, "{channel:?}: `{}` in {n} HELP_GROUPS entries", sub.get_name());
            }
        }
    }
}

use clap::CommandFactory;

use crate::cli::args::Cli;
use crate::release::{GatePath, ReleaseFeature};

const HELP_GROUPS: &[(&str, &[&str])] = &[
    (
        "Workspace",
        &["run", "workspace", "secret", "routine", "agent", "context"],
    ),
    ("Apps", &["app", "account", "registry", "events"]),
    ("Panes", &["pane", "notify"]),
    (
        "AI",
        &[
            "assistant",
            "ai",
            "changes",
            "command-view",
            "ledger",
            "needs-you",
            "permissions",
            "relay",
        ],
    ),
    (
        "System",
        &[
            "experiments",
            "skill",
            "completions",
            "host",
            "lease",
            "config",
            "note",
            "doctor",
            "debug",
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

/// Hide every path owned by an experiment that `enabled` reports off.
///
/// Driven entirely by `ReleaseFeature::owned_paths`, so help, completions and
/// the `exit_if_feature_disabled` backstop cannot drift apart. A hidden
/// top-level command keeps an `about` explaining why, so `plexi <cmd> --help`
/// still says how to turn it on.
pub fn gate_command(cmd: clap::Command, enabled: impl Fn(ReleaseFeature) -> bool) -> clap::Command {
    let mut cmd = cmd;
    for feature in ReleaseFeature::ALL.iter().copied().filter(|f| !enabled(*f)) {
        let text = crate::release::feature_unavailable_text(feature);
        for path in feature.owned_paths() {
            cmd = match *path {
                GatePath::Sub(path) => hide_subcommand(cmd, path, &text),
                GatePath::Arg(path, id) => hide_arg(cmd, path, id),
            };
        }
    }
    cmd
}

fn hide_subcommand(cmd: clap::Command, path: &[&str], text: &str) -> clap::Command {
    let Some((head, rest)) = path.split_first() else {
        return cmd;
    };
    cmd.mut_subcommand(*head, |sub| {
        if rest.is_empty() {
            let names: Vec<String> = sub
                .get_subcommands()
                .map(|c| c.get_name().to_string())
                .collect();
            names
                .into_iter()
                .fold(sub, |s, n| s.mut_subcommand(n, |c| c.hide(true)))
                .hide(true)
                .about(text.to_string())
                .long_about(None::<&str>)
        } else {
            hide_subcommand(sub, rest, text)
        }
    })
}

fn hide_arg(cmd: clap::Command, path: &[&str], id: &str) -> clap::Command {
    let Some((head, rest)) = path.split_first() else {
        return cmd.mut_arg(id, |a| a.hide(true));
    };
    cmd.mut_subcommand(*head, |sub| hide_arg(sub, rest, id))
}

/// The first experiment that `matches` invokes while `enabled` reports it off.
///
/// Walks the matched subcommand chain against `ReleaseFeature::owned_paths`,
/// the same table `gate_command` hides, so the execution backstop cannot drift
/// from help and completions.
pub fn blocked_feature(
    matches: &clap::ArgMatches,
    enabled: impl Fn(ReleaseFeature) -> bool,
) -> Option<ReleaseFeature> {
    let mut chain: Vec<(&str, &clap::ArgMatches)> = Vec::new();
    let mut current = matches;
    while let Some((name, sub)) = current.subcommand() {
        chain.push((name, sub));
        current = sub;
    }
    ReleaseFeature::ALL
        .iter()
        .copied()
        .filter(|f| !enabled(*f))
        .find(|f| {
            f.owned_paths().iter().any(|path| match *path {
                GatePath::Sub(p) => {
                    chain.len() >= p.len() && chain.iter().zip(p).all(|((n, _), want)| n == want)
                }
                GatePath::Arg(p, id) => {
                    chain.len() == p.len()
                        && chain.iter().zip(p).all(|((n, _), want)| n == want)
                        && chain.last().is_some_and(|(_, m)| {
                            m.value_source(id) == Some(clap::parser::ValueSource::CommandLine)
                        })
                }
            })
        })
}

pub fn print_grouped_help() {
    print!(
        "{}",
        grouped_help(&gated_command(), std::env::var_os("NO_COLOR").is_some())
    );
}

/// Diagnose command-shaped words before the workspace-path fallback consumes them.
/// Existing directories and explicitly spelled paths retain workspace semantics.
pub fn unknown_command_help(args: &[String], cmd: &clap::Command) -> Option<String> {
    let mut iter = args.iter().skip(1);
    let word = loop {
        let arg = iter.next()?;
        if matches!(arg.as_str(), "--profile" | "--socket") {
            iter.next();
        } else if !arg.starts_with('-') {
            break arg;
        }
    };
    if cmd
        .get_subcommands()
        .any(|sub| sub.get_name() == word || sub.get_all_aliases().any(|alias| alias == word))
        || word.contains('/')
        || word.contains('\\')
        || word.starts_with('.')
        || std::path::Path::new(word).exists()
    {
        return None;
    }
    let parser = clap::Command::new("plexi")
        .color(clap::ColorChoice::Never)
        .subcommands(cmd.get_subcommands().cloned());
    let error = parser
        .try_get_matches_from(["plexi", word.as_str()])
        .unwrap_err();
    let suggestion = match error.get(clap::error::ContextKind::SuggestedSubcommand) {
        Some(clap::error::ContextValue::Strings(names)) => {
            format!("Did you mean {}?\n\n", names.join(" or "))
        }
        Some(clap::error::ContextValue::String(name)) => format!("Did you mean {name}?\n\n"),
        _ => String::new(),
    };
    Some(format!("{error}\n{suggestion}{}", grouped_help(cmd, true)))
}

pub fn grouped_help(cmd: &clap::Command, no_color: bool) -> String {
    use std::fmt::Write;
    let mut output = String::new();

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
        writeln!(output, "{about}").unwrap();
        writeln!(output).unwrap();
    }

    // Usage
    let bin = cmd.get_name();
    writeln!(output, "{} {bin} [OPTIONS] [COMMAND]", header("Usage")).unwrap();
    writeln!(output).unwrap();

    // Visible positional arguments (workspace_path)
    let visible_positional: Vec<_> = cmd
        .get_arguments()
        .filter(|a| !a.is_hide_set() && a.is_positional())
        .filter(|a| a.get_id() != "help" && a.get_id() != "version")
        .collect();

    if !visible_positional.is_empty() {
        writeln!(output, "{}", header("Arguments")).unwrap();
        for arg in &visible_positional {
            let name = format!("[{}]", arg.get_id().as_str().to_uppercase());
            let padded = format!("{name:<22}");
            let help = arg.get_help().map(|s| s.to_string()).unwrap_or_default();
            writeln!(output, "  {} {help}", lit(padded)).unwrap();
        }
        writeln!(output).unwrap();
    }

    // Options: -h and -V only (--profile is hidden)
    writeln!(output, "{}", header("Options")).unwrap();
    writeln!(
        output,
        "  {} Print help",
        lit(format!("{:<22}", "-h, --help"))
    )
    .unwrap();
    if cmd.get_version().is_some() {
        writeln!(
            output,
            "  {} Print version",
            lit(format!("{:<22}", "-V, --version"))
        )
        .unwrap();
    }
    writeln!(output).unwrap();

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
        writeln!(output, "{}", header(heading)).unwrap();
        for &name in *names {
            if let Some(sub) = cmd.find_subcommand(name).filter(|s| !s.is_hide_set()) {
                let about = sub.get_about().map(|s| s.to_string()).unwrap_or_default();
                writeln!(output, "  {} {about}", lit(format!("{name:<col_width$}"))).unwrap();
            }
        }
        writeln!(output).unwrap();
    }

    // After-help footer
    if let Some(after) = cmd.get_after_help() {
        writeln!(output, "{}", dim(&after.to_string())).unwrap();
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_help_snapshots() {
        let cmd = all_off();
        for (name, argv) in [
            ("short", vec!["plexi", "-h"]),
            ("long", vec!["plexi", "--help"]),
            ("typo", vec!["plexi", "pnae"]),
            ("typo-help", vec!["plexi", "pnae", "-h"]),
        ] {
            let args: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
            let error = unknown_command_help(&args, &cmd);
            let output = error.clone().unwrap_or_else(|| grouped_help(&cmd, true));
            if name.starts_with("typo") {
                assert!(error.is_some());
                assert!(
                    output.contains("Did you mean") && output.contains("pane"),
                    "{output}"
                );
                assert!(output.ends_with(&grouped_help(&cmd, true)));
            }
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(format!("src/cli/snapshots/help-{name}.txt"));
            if std::env::var_os("UPDATE_HELP_SNAPSHOTS").is_some() {
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(&path, &output).unwrap();
            }
            assert_eq!(std::fs::read_to_string(path).unwrap(), output);
        }
    }

    fn all_off() -> clap::Command {
        gate_command(Cli::command(), |_| false)
    }

    fn all_on() -> clap::Command {
        gate_command(Cli::command(), |_| true)
    }

    fn only(feature: ReleaseFeature) -> clap::Command {
        gate_command(Cli::command(), |f| f == feature)
    }

    fn visible_subs(cmd: &clap::Command) -> Vec<String> {
        cmd.get_subcommands()
            .filter(|c| !c.is_hide_set())
            .map(|c| c.get_name().to_string())
            .collect()
    }

    fn sub_at<'a>(cmd: &'a clap::Command, path: &[&str]) -> &'a clap::Command {
        path.iter().fold(cmd, |c, name| {
            c.find_subcommand(name)
                .unwrap_or_else(|| panic!("owned path {path:?} is not in the clap tree"))
        })
    }

    fn path_hidden(cmd: &clap::Command, path: GatePath) -> bool {
        match path {
            GatePath::Sub(p) => sub_at(cmd, p).is_hide_set(),
            GatePath::Arg(p, id) => sub_at(cmd, p)
                .get_arguments()
                .find(|a| a.get_id() == id)
                .unwrap_or_else(|| panic!("owned arg {p:?} --{id} is not in the clap tree"))
                .is_hide_set(),
        }
    }

    /// Experiments with no clap path of their own: a runtime surface only.
    const RUNTIME_ONLY: [ReleaseFeature; 5] = [
        ReleaseFeature::WasmApps,
        ReleaseFeature::PageGrid,
        ReleaseFeature::Daw,
        ReleaseFeature::MediaIo,
        ReleaseFeature::Accessibility,
    ];

    #[test]
    fn every_variant_hides_at_least_one_path_when_off() {
        let cmd = all_off();
        for feature in ReleaseFeature::ALL {
            let paths = feature.owned_paths();
            if RUNTIME_ONLY.contains(feature) {
                assert!(paths.is_empty(), "{} is runtime-only", feature.key());
                continue;
            }
            assert!(!paths.is_empty(), "{} owns no clap path", feature.key());
            for path in paths {
                assert!(path_hidden(&cmd, *path), "{}: {path:?}", feature.key());
            }
        }
    }

    #[test]
    fn enabling_one_experiment_shows_only_its_own_paths() {
        for feature in ReleaseFeature::ALL {
            let cmd = only(*feature);
            for path in feature.owned_paths() {
                assert!(!path_hidden(&cmd, *path), "{}: {path:?}", feature.key());
            }
            for other in ReleaseFeature::ALL.iter().filter(|o| *o != feature) {
                for path in other.owned_paths() {
                    // A path is shared only if another enabled feature owns it too.
                    if feature.owned_paths().contains(path) {
                        continue;
                    }
                    assert!(
                        path_hidden(&cmd, *path),
                        "{} on must not reveal {}: {path:?}",
                        feature.key(),
                        other.key()
                    );
                }
            }
        }
    }

    #[test]
    fn default_help_lists_no_experiment_commands() {
        let cmd = all_off();
        let top = visible_subs(&cmd);
        for name in [
            "assistant",
            "relay",
            "changes",
            "ledger",
            "command-view",
            "account",
            "routine",
        ] {
            assert!(!top.contains(&name.to_string()), "{top:?}");
        }
        assert!(top.contains(&"experiments".to_string()), "{top:?}");
        let agent = visible_subs(cmd.find_subcommand("agent").unwrap());
        assert_eq!(
            agent,
            [
                "team", "next", "fleet", "create", "list", "show", "call", "message", "handoff",
                "spawn", "done", "ask", "report", "status", "history", "reopen", "hook"
            ],
            "{agent:?}"
        );
        let app = visible_subs(cmd.find_subcommand("app").unwrap());
        for gated in ["publish", "browse", "search"] {
            assert!(!app.contains(&gated.to_string()), "{app:?}");
        }
        for core in ["open", "install", "list", "render", "check"] {
            assert!(app.contains(&core.to_string()), "{app:?}");
        }
        let open = sub_at(&cmd, &["app", "open"]);
        let hidden = |id: &str| {
            open.get_arguments()
                .find(|a| a.get_id() == id)
                .unwrap()
                .is_hide_set()
        };
        assert!(hidden("mcp"));
        assert!(!hidden("cli"), "--cli is v1 core");
    }

    fn blocked(args: &[&str], on: &[ReleaseFeature]) -> Option<ReleaseFeature> {
        let m = Cli::command()
            .try_get_matches_from(args)
            .unwrap_or_else(|e| panic!("{args:?} must parse: {e}"));
        blocked_feature(&m, |f| on.contains(&f))
    }

    #[test]
    fn execution_backstop_blocks_every_owned_surface_when_off() {
        let cases: &[(&[&str], ReleaseFeature)] = &[
            (
                &["plexi", "assistant", "tool", "x", "--input", "{}"],
                ReleaseFeature::Assistant,
            ),
            (
                &["plexi", "assistant", "permission", "list"],
                ReleaseFeature::Assistant,
            ),
            (&["plexi", "ai", "setup"], ReleaseFeature::AiSetup),
            (&["plexi", "agent", "run", "list"], ReleaseFeature::Leads),
            (
                &["plexi", "command-view", "--json"],
                ReleaseFeature::CommandView,
            ),
            (
                &["plexi", "changes", "refresh", "x"],
                ReleaseFeature::ChangeSets,
            ),
            (&["plexi", "ledger"], ReleaseFeature::Ledger),
            (&["plexi", "relay", "status"], ReleaseFeature::Relay),
            (
                &["plexi", "account", "status"],
                ReleaseFeature::CloudAccount,
            ),
            (
                &["plexi", "secret", "rm", "A", "--folder", "/tmp"],
                ReleaseFeature::FolderSecrets,
            ),
            (
                &["plexi", "secret", "set", "A", "--folder", "/tmp"],
                ReleaseFeature::FolderSecrets,
            ),
            (
                &["plexi", "app", "open", "--mcp", "x"],
                ReleaseFeature::McpClient,
            ),
            (
                &["plexi", "app", "search", "x"],
                ReleaseFeature::Marketplace,
            ),
            (&["plexi", "routine", "list"], ReleaseFeature::Routines),
        ];
        for (args, feature) in cases {
            assert_eq!(blocked(args, &[]), Some(*feature), "{args:?}");
            assert_eq!(blocked(args, &[*feature]), None, "{args:?} with it on");
        }
    }

    #[test]
    fn execution_backstop_leaves_v1_surfaces_alone() {
        for args in [
            &["plexi", "agent", "report", "--state", "idle"][..],
            &["plexi", "agent", "status"],
            &["plexi", "ai", "doctor"],
            &["plexi", "app", "open", "--cli", "htop"],
            &["plexi", "secret", "set", "A"],
            &["plexi", "secret", "get", "A"],
            &["plexi", "needs-you", "list"],
            &["plexi", "experiments", "list"],
        ] {
            assert_eq!(blocked(args, &[]), None, "{args:?}");
        }
    }

    #[test]
    fn a_hidden_command_explains_how_to_turn_it_on() {
        let mut cmd = all_off();
        let relay = cmd.find_subcommand_mut("relay").unwrap();
        let help = relay.render_long_help().to_string();
        assert!(
            help.contains(
                "relay is an experiment. Turn it on with: plexi experiments enable relay"
            ),
            "{help}"
        );
        assert!(!help.contains("pair"), "{help}");
    }

    #[test]
    fn gated_commands_still_parse_for_the_execution_backstop() {
        use clap::FromArgMatches;
        let mut m = all_off()
            .try_get_matches_from(["plexi", "events", "mcp-config"])
            .expect("hidden command must still parse");
        assert!(Cli::from_arg_matches_mut(&mut m).is_ok());
    }

    #[test]
    fn every_visible_top_level_command_is_in_exactly_one_help_group() {
        for cmd in [all_off(), all_on()] {
            for sub in cmd.get_subcommands().filter(|c| !c.is_hide_set()) {
                let n = HELP_GROUPS
                    .iter()
                    .filter(|(_, names)| names.contains(&sub.get_name()))
                    .count();
                assert_eq!(n, 1, "`{}` in {n} HELP_GROUPS entries", sub.get_name());
            }
        }
    }
}

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
            "cli",
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
    let ws_commands = crate::cli::commands_file();
    let global_commands = crate::cli::global_commands_path();
    let global_scripts = crate::config::config_dir().join("scripts");
    let run_paths = format!(
        "Active channel paths:\n  Workspace: <repo>/{ws_commands}\n  Global:    {}\n  Scripts:   {}",
        global_commands.display(),
        global_scripts.display()
    );
    gate_command(Cli::command(), crate::release::feature_enabled)
        .mut_subcommand("run", |sub| sub.after_help(run_paths))
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
                GatePath::Arg(path, id) => hide_arg(cmd, path, id, &text),
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

fn hide_arg(cmd: clap::Command, path: &[&str], id: &str, text: &str) -> clap::Command {
    let Some((head, rest)) = path.split_first() else {
        let description = cmd
            .get_arguments()
            .find(|a| a.get_id() == id)
            .and_then(|a| a.get_long_help().or_else(|| a.get_help()))
            .map(ToString::to_string)
            .unwrap_or_default();
        let previous = cmd
            .get_after_help()
            .map(ToString::to_string)
            .unwrap_or_default();
        return cmd
            .after_help(format!(
                "{previous}\n--{id}: {description}\nExperiment off: {text}"
            ))
            .mut_arg(id, |a| a.hide(true));
    };
    cmd.mut_subcommand(*head, |sub| hide_arg(sub, rest, id, text))
}

/// Capture the gate result before clap's mutable conversion consumes matches.
pub fn decode_matches(
    mut matches: clap::ArgMatches,
    enabled: impl Fn(ReleaseFeature) -> bool,
) -> Result<(Cli, Option<ReleaseFeature>), clap::Error> {
    use clap::FromArgMatches;
    let blocked = blocked_feature(&matches, enabled);
    let cli = Cli::from_arg_matches_mut(&mut matches)?;
    Ok((cli, blocked))
}

/// Retry only failed parses with optional experiment-arg values so a missing
/// value can still receive the experiment diagnostic. Successful parses always
/// use the original command and its full validation.
pub fn blocked_incomplete_arg(
    args: &[String],
    enabled: impl Fn(ReleaseFeature) -> bool,
) -> Option<ReleaseFeature> {
    fn relax(cmd: clap::Command, path: &[&str], id: &str) -> clap::Command {
        match path.split_first() {
            Some((head, rest)) => cmd.mut_subcommand(*head, |sub| relax(sub, rest, id)),
            None => cmd.mut_arg(id, |arg| arg.num_args(0..=1).value_parser(clap::builder::StringValueParser::new()).default_missing_value("missing-experiment-value")),
        }
    }
    let mut cmd = Cli::command();
    for feature in ReleaseFeature::ALL
        .iter()
        .copied()
        .filter(|feature| !enabled(*feature))
    {
        for path in feature.owned_paths() {
            if let GatePath::Arg(path, id) = path {
                cmd = relax(cmd, path, id);
            }
        }
    }
    let matches = cmd.try_get_matches_from(args).ok()?;
    blocked_feature(&matches, enabled)
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

/// Optimal String Alignment (restricted Damerau-Levenshtein) edit distance:
/// insertions, deletions, substitutions, and adjacent transpositions each cost 1.
fn osa_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let m = a.len();
    let n = b.len();
    let mut dp = vec![vec![0usize; n + 1]; m + 1];
    for (i, row) in dp.iter_mut().enumerate().take(m + 1) {
        row[0] = i;
    }
    for (j, cell) in dp[0].iter_mut().enumerate().take(n + 1) {
        *cell = j;
    }
    for i in 1..=m {
        for j in 1..=n {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            dp[i][j] = (dp[i - 1][j] + 1)
                .min(dp[i][j - 1] + 1)
                .min(dp[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                dp[i][j] = dp[i][j].min(dp[i - 2][j - 2] + 1);
            }
        }
    }
    dp[m][n]
}

/// Extract the invoked binary basename from `args[0]`, stripping `.exe`/`.EXE`
/// and falling back to `"plexi"`.
fn invoked_bin_name(args: &[String]) -> &str {
    args.first()
        .and_then(|arg0| arg0.rsplit(['/', '\\']).next())
        .map(|base| {
            base.strip_suffix(".exe")
                .or_else(|| base.strip_suffix(".EXE"))
                .unwrap_or(base)
        })
        .filter(|s| !s.is_empty())
        .unwrap_or("plexi")
}

/// Return the single closest visible subcommand or alias using OSA edit distance
/// with threshold `distance <= max(1, word.len() / 3)`. Ties at the minimum
/// distance break deterministically by subcommand declaration order.
fn closest_subcommand_suggestion<'a>(word: &str, cmd: &'a clap::Command) -> Option<&'a str> {
    let threshold = (word.chars().count() / 3).max(1);
    let mut best: Option<(usize, &'a str)> = None;
    for sub in cmd.get_subcommands().filter(|sub| !sub.is_hide_set()) {
        for candidate in std::iter::once(sub.get_name()).chain(sub.get_all_aliases()) {
            let dist = osa_distance(word, candidate);
            if dist <= threshold && best.is_none_or(|(best_dist, _)| dist < best_dist) {
                best = Some((dist, candidate));
            }
        }
    }
    best.map(|(_, name)| name)
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
    let bin = invoked_bin_name(args);
    let suggestion = closest_subcommand_suggestion(word, cmd);
    log::info!(
        "cli:unknown_command: bin={bin} word={word:?} suggestion={suggestion:?}"
    );
    let headline = match suggestion {
        Some(matched) => format!("error: unknown command '{word}'. Did you mean `{matched}`?"),
        None => format!("error: unknown command '{word}'"),
    };
    Some(format!(
        "{headline}\n\nUsage: {bin} [OPTIONS] [COMMAND]\n\nFor more information, try '{bin} --help'.\n"
    ))
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
                    output.contains("error: unknown command 'pnae'. Did you mean `pane`?"),
                    "{output}"
                );
                assert!(!output.contains("update"), "{output}");
                assert!(!output.contains("tip:"), "{output}");
                assert!(!output.contains(&grouped_help(&cmd, true)), "{output}");
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
                "start", "spawn", "done", "result", "wait", "ask", "report", "status", "history",
                "reopen", "hook"
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
        decode_matches(m, |f| on.contains(&f)).unwrap().1
    }

    #[test]
    fn folder_gate_survives_cli_conversion_exact_reported_argv() {
        let matches = all_off()
            .try_get_matches_from([
                "plexi",
                "secret",
                "set",
                "DEMO_TOKEN",
                "--folder",
                ".",
                "--from-env",
            ])
            .unwrap();
        let (_cli, blocked) = decode_matches(matches, |_| false).unwrap();
        assert_eq!(blocked, Some(ReleaseFeature::FolderSecrets));
    }

    #[test]
    fn folder_help_and_incomplete_gate_are_actionable() {
        for mut command in [all_off(), all_on()] {
            let help = command
                .find_subcommand_mut("secret")
                .unwrap()
                .find_subcommand_mut("set")
                .unwrap()
                .render_long_help()
                .to_string();
            assert!(help.contains("--folder") && help.contains("folder_secrets"));
            assert!(help.contains("plexi experiments enable folder_secrets"));
        }
        let args: Vec<String> = ["plexi", "secret", "set", "DEMO_TOKEN", "--folder"]
            .into_iter()
            .map(str::to_string)
            .collect();
        assert_eq!(
            blocked_incomplete_arg(&args, |_| false),
            Some(ReleaseFeature::FolderSecrets)
        );
        assert_eq!(blocked_incomplete_arg(&args, |_| true), None);
        let literal: Vec<String> = ["plexi", "secret", "set", "--", "--folder"]
            .into_iter()
            .map(str::to_string)
            .collect();
        assert_eq!(blocked_incomplete_arg(&literal, |_| false), None);
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

    #[test]
    fn run_help_lists_per_channel_commands_toml_paths() {
        let _g = crate::config::set_test_channel("alpha");
        let mut cmd = gated_command();
        let run_sub = cmd.find_subcommand_mut("run").unwrap();
        let help = run_sub.render_long_help().to_string();
        for needle in [
            "~/.plexi-alpha/commands.toml",
            "~/.plexi-beta/commands.toml",
            "~/.plexi/commands.toml",
            "<repo>/.plexi-alpha/commands.toml",
            "resolve_workspace_root_stopping_at",
            "resolve_commands_workspace_root",
            "effective_commands_workspace_root",
            "run_user_command_by_name",
            "workspace `commands.toml` > global `commands.toml` > global `scripts/`",
            "no host restart is needed",
        ] {
            assert!(
                help.contains(needle),
                "run --help missing `{needle}`:\n{help}"
            );
        }
    }

    #[test]
    fn unknown_command_pnae_suggests_pane_only_without_help_dump() {
        let cmd = all_off();
        let args = vec!["plexi".to_string(), "pnae".to_string()];
        let out = unknown_command_help(&args, &cmd).expect("expected unknown command diagnostic");
        assert_eq!(
            out,
            "error: unknown command 'pnae'. Did you mean `pane`?\n\nUsage: plexi [OPTIONS] [COMMAND]\n\nFor more information, try 'plexi --help'.\n"
        );
        assert!(!out.contains("update"));
        assert!(!out.contains("tip:"));
    }

    #[test]
    fn unknown_command_far_typo_omits_did_you_mean() {
        let cmd = all_off();
        let args = vec!["plexi-alpha".to_string(), "zzzzqx".to_string()];
        let out = unknown_command_help(&args, &cmd).expect("expected unknown command diagnostic");
        assert_eq!(
            out,
            "error: unknown command 'zzzzqx'\n\nUsage: plexi-alpha [OPTIONS] [COMMAND]\n\nFor more information, try 'plexi-alpha --help'.\n"
        );
        assert!(!out.contains("Did you mean"));
    }

    #[test]
    fn unknown_command_uses_invoked_binary_basename() {
        let cmd = all_off();
        for (arg0, expected_bin) in [
            ("plexi-alpha", "plexi-alpha"),
            ("/path/to/plexi-alpha.exe", "plexi-alpha"),
            ("C:\\bin\\plexi-alpha.EXE", "plexi-alpha"),
            ("plexi", "plexi"),
            ("", "plexi"),
        ] {
            let args = vec![arg0.to_string(), "pnae".to_string()];
            let out = unknown_command_help(&args, &cmd).expect("expected diagnostic");
            assert!(
                out.contains(&format!("Usage: {expected_bin} [OPTIONS] [COMMAND]")),
                "arg0={arg0:?} output={out}"
            );
            assert!(
                out.contains(&format!("For more information, try '{expected_bin} --help'.")),
                "arg0={arg0:?} output={out}"
            );
        }

        // Workspace-path fallback cases stay untouched (return None).
        for path_arg in [".", "./nonexistent-workspace", "foo/bar", "src"] {
            let args = vec!["plexi".to_string(), path_arg.to_string()];
            assert!(
                unknown_command_help(&args, &cmd).is_none(),
                "expected workspace-path arg {path_arg:?} to bypass unknown_command_help"
            );
        }
    }
}

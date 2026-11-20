use clap::CommandFactory;

use crate::cli::args::Cli;
use crate::release::{GatePath, ReleaseFeature};

const HELP_GROUPS: &[(&str, &[&str])] = &[
    (
        "Workspace",
        &["run", "workspace", "secret", "routine", "agent", "context"],
    ),
    (
        "Plugins",
        &["plugin", "view", "account", "registry", "events"],
    ),
    ("Panes", &["pane", "notify"]),
    (
        "AI",
        &[
            "assistant",
            "ai",
            "changes",
            "agents-dashboard",
            "ledger",
            "needs-you",
            "permissions",
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

/// The public command tree: every experiment hidden, whatever any profile
/// enables. Help, usage, suggestions and completions render from it, so they
/// never read a profile. Hidden commands still parse; dispatch admission
/// (`admission`) refuses an experiment that is off.
pub fn public_command() -> clap::Command {
    gate_command(Cli::command(), |_| false)
}

/// Append the active channel's `plexi run` paths to `run`'s help. Pure path
/// computation from the installed profile selection; it reads no file.
pub fn with_run_paths(cmd: clap::Command) -> clap::Command {
    let ws_commands = crate::cli::commands_file();
    let global_commands = crate::cli::global_commands_path();
    let global_scripts = crate::config::config_dir().join("scripts");
    let run_paths = format!(
        "Active channel paths:\n  Workspace: <repo>/{ws_commands}\n  Global:    {}\n  Scripts:   {}",
        global_commands.display(),
        global_scripts.display()
    );
    cmd.mut_subcommand("run", |sub| sub.after_help(run_paths))
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

/// The command surface an invocation names, captured from intact matches
/// before typed conversion consumes them: the subcommand path and the ids of
/// the leaf's arguments given on the command line.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Surface {
    pub path: Vec<String>,
    pub explicit: Vec<String>,
}

impl Surface {
    pub fn capture(matches: &clap::ArgMatches) -> Self {
        let mut path = Vec::new();
        let mut leaf = matches;
        while let Some((name, sub)) = leaf.subcommand() {
            path.push(name.to_string());
            leaf = sub;
        }
        let explicit = leaf
            .ids()
            .filter(|id| {
                leaf.value_source(id.as_str()) == Some(clap::parser::ValueSource::CommandLine)
            })
            .map(|id| id.as_str().to_string())
            .collect();
        Self { path, explicit }
    }
}

/// Dispatch admission: the experiment that is off and owns `surface`, if any.
/// `agent start/spawn/open/reopen` round-trip to the host, which reads
/// agent_mesh fresh at admission, so the CLI leaves that decision to it.
pub fn admission(
    surface: &Surface,
    enabled: impl Fn(ReleaseFeature) -> bool,
) -> Option<ReleaseFeature> {
    let host_agent_route = surface.path.first().is_some_and(|name| name == "agent")
        && matches!(
            surface.path.get(1).map(String::as_str),
            Some("start" | "spawn" | "open" | "reopen")
        );
    blocked_feature(surface, |feature| {
        (host_agent_route && feature == ReleaseFeature::AgentMesh) || enabled(feature)
    })
}

/// The first experiment that `surface` invokes while `enabled` reports it off.
///
/// Matches the surface against `ReleaseFeature::owned_paths`, the same table
/// `gate_command` hides, so dispatch cannot drift from help and completions.
pub fn blocked_feature(
    surface: &Surface,
    enabled: impl Fn(ReleaseFeature) -> bool,
) -> Option<ReleaseFeature> {
    let on_path = |p: &[&str]| surface.path.iter().zip(p).all(|(n, want)| n == want);
    ReleaseFeature::ALL
        .iter()
        .copied()
        .filter(|f| !enabled(*f))
        .find(|f| {
            f.owned_paths().iter().any(|path| match *path {
                GatePath::Sub(p) => surface.path.len() >= p.len() && on_path(p),
                GatePath::Arg(p, id) => {
                    surface.path.len() == p.len()
                        && on_path(p)
                        && surface.explicit.iter().any(|given| given == id)
                }
            })
        })
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

/// Return the single closest visible subcommand or alias using OSA edit distance
/// with threshold `distance <= max(1, word.len() / 3)`. Ties at the minimum
/// distance break deterministically by subcommand declaration order.
pub(crate) fn closest_subcommand_suggestion<'a>(word: &str, cmd: &'a clap::Command) -> Option<&'a str> {
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

pub fn grouped_help(cmd: &clap::Command, bin: &str, no_color: bool) -> String {
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

    #[test]
    fn removed_relay_is_not_a_command_or_experiment() {
        assert!(Cli::command().find_subcommand("relay").is_none());
        assert!(ReleaseFeature::from_key("relay").is_none());
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
            "changes",
            "ledger",
            "agents-dashboard",
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
                "list", "show", "call", "message", "handoff", "done", "ask", "report", "status",
                "hook"
            ],
            "{agent:?}"
        );
        let app = visible_subs(cmd.find_subcommand("plugin").unwrap());
        for gated in ["publish", "browse", "search"] {
            assert!(!app.contains(&gated.to_string()), "{app:?}");
        }
        for core in ["install", "list", "check"] {
            assert!(app.contains(&core.to_string()), "{app:?}");
        }
        let open = sub_at(&cmd, &["view", "open"]);
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
        admission(&Surface::capture(&m), |f| on.contains(&f))
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
        let surface = Surface::capture(&matches);
        let cli = <Cli as clap::FromArgMatches>::from_arg_matches(&matches);
        assert!(cli.is_ok());
        assert_eq!(admission(&surface, |_| false), Some(ReleaseFeature::FolderSecrets));
    }

    #[test]
    fn folder_help_is_actionable() {
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
                &["plexi", "agents-dashboard", "--json"],
                ReleaseFeature::AgentsDashboard,
            ),
            (
                &["plexi", "changes", "refresh", "x"],
                ReleaseFeature::ChangeSets,
            ),
            (&["plexi", "ledger"], ReleaseFeature::Ledger),
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
                &["plexi", "view", "open", "--mcp", "x"],
                ReleaseFeature::McpClient,
            ),
            (
                &["plexi", "plugin", "search", "x"],
                ReleaseFeature::Marketplace,
            ),
            (&["plexi", "routine", "list"], ReleaseFeature::Routines),
            (
                &["plexi", "agent", "fleet", "dismiss", "1"],
                ReleaseFeature::AgentMesh,
            ),
            (
                &["plexi", "agent", "team", "show", "x"],
                ReleaseFeature::AgentMesh,
            ),
        ];
        for (args, feature) in cases {
            assert_eq!(blocked(args, &[]), Some(*feature), "{args:?}");
            assert_eq!(blocked(args, &[*feature]), None, "{args:?} with it on");
        }
    }

    #[test]
    fn agent_mesh_gate_cli_defers_host_routes_to_the_host() {
        for args in [
            vec!["plexi", "agent", "start", "x"],
            vec!["plexi", "agent", "spawn", "x", "--task", "y"],
            vec!["plexi", "agent", "open", "x"],
            vec!["plexi", "agent", "reopen", "session"],
        ] {
            assert_eq!(blocked(&args, &[]), None, "{args:?}");
        }
    }

    #[test]
    fn execution_backstop_leaves_v1_surfaces_alone() {
        for args in [
            &["plexi", "agent", "report", "--state", "idle"][..],
            &["plexi", "agent", "status"],
            &["plexi", "agent", "list", "--json"],
            &["plexi", "agent", "show", "x"],
            &["plexi", "agent", "done", "--result", "{}"],
            &["plexi", "agent", "ask", "q"],
            &["plexi", "agent", "hook", "install"],
            &["plexi", "ai", "doctor"],
            &["plexi", "view", "open", "--cli", "htop"],
            &["plexi", "secret", "set", "A"],
            &["plexi", "secret", "get", "A"],
            &["plexi", "needs-you", "list"],
            &["plexi", "experiments", "list"],
        ] {
            assert_eq!(blocked(args, &[]), None, "{args:?}");
        }
    }

    #[test]
    fn gated_commands_still_parse_for_the_execution_backstop() {
        use clap::FromArgMatches;
        let mut m = all_off()
            .try_get_matches_from(["plexi", "events", "mcp-config"])
            .expect("hidden command must still parse");
        assert!(Cli::from_arg_matches_mut(&mut m).is_ok());
    }

    /// Stint 1520: every row of `plexi --help` fits one 80-column line, so the
    /// command list never wraps. The full text belongs in `plexi <cmd> --help`.
    #[test]
    fn every_top_level_help_row_fits_80_columns() {
        for cmd in [all_off(), all_on()] {
            let long: Vec<String> = grouped_help(&cmd, "plexi", true)
                .lines()
                .filter(|line| line.chars().count() > 80)
                .map(|line| format!("{} cols: {line}", line.chars().count()))
                .collect();
            assert!(
                long.is_empty(),
                "help rows over 80 columns:\n{}",
                long.join("\n")
            );
        }
    }

    /// The short `about` is only the list row: `plexi <cmd> --help` still
    /// carries the full description.
    #[test]
    fn shortened_commands_keep_their_full_text_in_long_help() {
        let mut cmd = all_on();
        for (name, full) in [
            (
                "run",
                "~/.plexi-<channel>/commands.toml` (or global scripts)",
            ),
            (
                "agent",
                "spawning and teams need the `agent_mesh` experiment",
            ),
            ("ai", "scan hardware, check integrations, recommend models"),
        ] {
            let help = cmd
                .find_subcommand_mut(name)
                .expect("command")
                .render_long_help()
                .to_string();
            assert!(
                help.contains(full),
                "`{name} --help` lost its text:\n{help}"
            );
        }
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
        let mut cmd = with_run_paths(public_command());
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
            "queue_user_command",
            "workspace `commands.toml` > global `commands.toml` > global `scripts/`",
            "no host restart is needed",
        ] {
            assert!(
                help.contains(needle),
                "run --help missing `{needle}`:\n{help}"
            );
        }
    }
}

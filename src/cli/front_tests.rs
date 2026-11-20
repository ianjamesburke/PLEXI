//! The front end's contract at the argv boundary: each argv yields exactly one
//! outcome. Probes from the PR #3267 reviews are reproduced here verbatim.

use super::*;
use crate::cli::args::Commands;
use clap::CommandFactory;

const BIN: &str = "plexi-pr-9";

fn argv(parts: &[&str]) -> Vec<String> {
    std::iter::once(BIN)
        .chain(parts.iter().copied())
        .map(str::to_string)
        .collect()
}

fn parse_parts(parts: &[&str]) -> Outcome {
    outcome(BIN, &argv(parts))
}

fn kind(outcome: &Outcome) -> &'static str {
    match outcome {
        Outcome::Help(_) => "help",
        Outcome::Version => "version",
        Outcome::UsageError(UsageError::Profile(_)) => "profile",
        Outcome::UsageError(UsageError::Retired(_)) => "retired",
        Outcome::UsageError(UsageError::UnknownCommand { .. }) => "unknown_command",
        Outcome::UsageError(UsageError::MissingCommand { .. }) => "missing_command",
        Outcome::UsageError(UsageError::Rebound { .. }) => "rebound",
        Outcome::UsageError(UsageError::NotUtf8 { .. }) => "not_utf8",
        Outcome::UsageError(UsageError::Grammar { .. }) => "grammar",
        Outcome::UsageError(UsageError::Internal(message)) => panic!("internal: {message}"),
        Outcome::Completions { .. } => "completions",
        Outcome::Run(_) => "run",
        Outcome::Launch(_) => "launch",
    }
}

fn expect(parts: &[&str], want: &str) -> Outcome {
    let outcome = parse_parts(parts);
    let got = kind(&outcome);
    let text = match &outcome {
        Outcome::UsageError(error) => usage_error_text(BIN, error),
        _ => String::new(),
    };
    assert_eq!(got, want, "{parts:?}: {text}");
    outcome
}

fn help_of(parts: &[&str]) -> (Vec<String>, bool, ProfileSelection) {
    match expect(parts, "help") {
        Outcome::Help(help) => (help.path, help.long, help.selection),
        _ => unreachable!(),
    }
}

fn run_of(parts: &[&str]) -> Box<Run> {
    match expect(parts, "run") {
        Outcome::Run(run) => run,
        _ => unreachable!(),
    }
}

fn error_text(parts: &[&str]) -> String {
    match parse_parts(parts) {
        Outcome::UsageError(error) => usage_error_text(BIN, &error),
        other => panic!("{parts:?} is {}, not a usage error", kind(&other)),
    }
}

fn path(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

fn named(name: &str) -> ProfileSelection {
    ProfileSelection::parse(name).unwrap()
}

/// Round 3 P1 and round 4 P1: a duplicate selector is a usage error even when
/// display or an experiment refusal is also requested.
#[test]
fn a_duplicate_profile_is_a_usage_error_beside_display_and_experiments() {
    for parts in [
        &["--version", "--profile", "one", "host", "--profile", "two"][..],
        &["--profile", "one", "host", "--help", "--profile", "two"],
        &["--profile", "one", "--version", "--profile"],
        &[
            "--profile",
            "one",
            "assistant",
            "permission",
            "list",
            "--profile",
            "two",
        ],
        &[
            "--profile",
            "one",
            "assistant",
            "permission",
            "list",
            "--profile",
            "one",
        ],
        &["--profile", "one", "--profile", "two", "host", "stop"],
        &["host", "--profile", "one", "stop", "--profile", "two"],
        &["--profile", "default", "host", "stop", "--profile", "two"],
    ] {
        let text = error_text(parts);
        assert!(text.contains("--profile"), "{parts:?}: {text}");
        assert_eq!(kind(&parse_parts(parts)), "profile", "{parts:?}");
    }
}

#[test]
fn a_malformed_profile_is_a_usage_error_before_anything_else() {
    for parts in [
        &["host", "stop", "--profile"][..],
        &["host", "restart", "--profile", "--yes"],
        &["host", "stop", "--profile="],
        &["--profile", "nested/name", "host", "stop"],
        &["--profile", "../x", "host", "stop"],
        &["--profile", "Default", "--help"],
        &["--version", "--profile"],
    ] {
        expect(parts, "profile");
    }
    let text = error_text(&["--profile", "Default", "host", "status"]);
    assert!(text.contains("~/.plexi-Default exists"), "{text}");
    assert!(text.contains("~/.plexi-default-legacy"), "{text}");
}

/// Round 5 P1: `--render` is an explicit entry point only as the whole argv.
#[test]
fn render_with_any_other_token_is_not_an_entry_point() {
    expect(&["--render", "--profile"], "grammar");
    expect(&["--render", "--profile", "t1"], "grammar");
}

#[test]
fn profile_is_position_independent() {
    let base = run_of(&["host", "status"]);
    for parts in [
        &["--profile", "t1", "host", "status"][..],
        &["host", "--profile", "t1", "status"],
        &["host", "status", "--profile", "t1"],
        &["host", "status", "--profile=t1"],
    ] {
        let run = run_of(parts);
        assert_eq!(run.selection, named("t1"), "{parts:?}");
        assert_eq!(run.surface, base.surface, "{parts:?}");
    }
    assert_eq!(base.selection, ProfileSelection::Default);
    assert_eq!(
        run_of(&["--profile", "default", "host", "status"]).selection,
        ProfileSelection::Default
    );
}

/// Round 3 P2: a valid trailing selector on `help <path>`.
#[test]
fn help_subcommand_accepts_the_selector_anywhere() {
    let after = help_of(&["help", "host", "--profile", "t1"]);
    let before = help_of(&["--profile", "t1", "help", "host"]);
    assert_eq!(after, before);
    assert_eq!(after, (path(&["host"]), true, named("t1")));
    expect(&["help", "no-such-command"], "unknown_command");
    expect(&["help", "host", "no-such-verb"], "unknown_command");
    expect(&["help", "host", "--bogus"], "grammar");
}

/// Round 5 P1: a valid selector never changes the command recognized at a
/// node whose arguments conflict with subcommands.
#[test]
fn a_selector_does_not_change_args_conflicts_with_subcommands_nodes() {
    assert_eq!(
        help_of(&["note", "--profile", "t1", "list", "--help"]).0,
        path(&["note", "list"])
    );
    assert_eq!(
        help_of(&["pane", "result", "--profile", "t1", "read", "--help"]).0,
        path(&["pane", "result", "read"])
    );
    assert_eq!(
        run_of(&["note", "--profile", "t1", "list"]).surface.path,
        path(&["note", "list"])
    );
}

/// Round 3 P1: a child's tokens after `--` are forwarded untouched.
#[test]
fn the_child_tail_is_forwarded_untouched() {
    let tail = ["--profile", "two", "--help", "config", "-ws"];
    let mut parts = vec!["run", "example", "--"];
    parts.extend(tail);
    let run = run_of(&parts);
    assert_eq!(run.selection, ProfileSelection::Default);
    let Some(Commands::Run {
        command,
        extra_args,
    }) = run.cli.command
    else {
        panic!("expected run");
    };
    assert_eq!(command.as_deref(), Some("example"));
    assert_eq!(extra_args, tail);

    let run = run_of(&["config", "check", "-ws"]);
    let Some(Commands::Config {
        cmd: crate::cli::args::ConfigCmd::Check { scope },
    }) = run.cli.command
    else {
        panic!("expected config check");
    };
    assert_eq!(scope.scope(), crate::cli::args::ConfigScope::Workspace);
}

/// Round 4 P2: parent display is not lost to child requirements.
#[test]
fn display_requests_waive_run_requirements_where_they_appear() {
    assert_eq!(kind(&parse_parts(&["--version", "host"])), "version");
    assert_eq!(
        kind(&parse_parts(&["--version", "host", "--json"])),
        "version"
    );
    assert_eq!(
        help_of(&["pane", "--help", "send"]),
        help_of(&["pane", "--help"])
    );
    assert_eq!(help_of(&["pane", "--help", "send"]).0, path(&["pane"]));
    assert_eq!(
        help_of(&["pane", "send", "--help"]).0,
        path(&["pane", "send"])
    );
    assert!(!help_of(&["pane", "send", "-h"]).1, "-h is the short form");
    expect(&["pane", "send", "--help", "--bogus"], "grammar");
    expect(&["--version", "--bogus"], "grammar");
    // Help beats version when both are valid.
    assert_eq!(help_of(&["--version", "--help"]).0, Vec::<String>::new());
    // A missing option value is still a usage error on display.
    expect(&["secret", "set", "A", "--folder", "--help"], "grammar");
}

/// Round 4 P2: missing-command output lists only visible commands and names
/// the invoked binary.
#[test]
fn missing_command_output_hides_hidden_commands_and_names_the_binary() {
    for parts in [&["agent"][..], &["agent", "--json"]] {
        let text = error_text(parts);
        assert_eq!(kind(&parse_parts(parts)), "missing_command");
        assert!(text.contains(&format!("Usage: {BIN} agent")), "{text}");
        for hidden in ["team", "fleet"] {
            assert!(
                !text
                    .lines()
                    .any(|line| line.trim_start().starts_with(hidden)),
                "{parts:?} advertised {hidden}:\n{text}"
            );
        }
    }
    assert_eq!(help_of(&["agent", "--help"]).0, path(&["agent"]));
    let host = error_text(&["host", "--json"]);
    assert!(host.contains(&format!("Usage: {BIN} host")), "{host}");
    // Round 5 P2: the selector's position does not change the diagnosis.
    assert_eq!(
        error_text(&["host", "--profile", "pr-3267"]),
        error_text(&["--profile", "pr-3267", "host"])
    );
}

#[test]
fn version_names_the_invoked_binary() {
    assert!(grammar(BIN)
        .render_version()
        .starts_with(&format!("{BIN} ")));
}

#[test]
fn root_operands_and_unknown_commands() {
    let text = error_text(&["pnae"]);
    assert!(
        text.starts_with("error: unknown command 'pnae'. Did you mean `pane`?"),
        "{text}"
    );
    assert!(
        text.contains(&format!("Usage: {BIN} [OPTIONS] [COMMAND]")),
        "{text}"
    );
    expect(&["pnae", "-h"], "unknown_command");
    expect(&["app", "list"], "retired");
    expect(&["relay"], "unknown_command");
    let far = error_text(&["zzzzqx"]);
    assert!(!far.contains("Did you mean"), "{far}");
    let nested = error_text(&["agent", "tem"]);
    assert!(nested.contains(&format!("Usage: {BIN} agent")), "{nested}");
    assert!(
        !nested.contains("team"),
        "hidden command suggested: {nested}"
    );
}

#[test]
fn launch_is_positive_intent_only() {
    let tmp = std::env::temp_dir();
    let tmp = tmp.to_string_lossy();
    for parts in [
        &[][..],
        &["--profile", "alpha"],
        &["--profile", "doctor"],
        &["--socket", "/tmp/s.sock"],
        &[tmp.as_ref()],
    ] {
        expect(parts, "launch");
    }
    for parts in [
        &["--version"][..],
        &["-V"],
        &["--help"],
        &["-h"],
        &["help"],
        &["--profile", "alpha", "--version"],
        &["--bogus-flag"],
        &["plugin", "list"],
    ] {
        assert_ne!(kind(&parse_parts(parts)), "launch", "{parts:?}");
    }
    expect(&["completions", "zsh"], "completions");
    expect(&["completions", "--help"], "help");
}

#[test]
fn experiment_admission_follows_the_captured_surface() {
    let run = run_of(&["--profile", "t1", "assistant", "permission", "list"]);
    assert_eq!(
        help::admission(&run.surface, |_| false),
        Some(crate::release::ReleaseFeature::Assistant)
    );
    assert_eq!(help::admission(&run.surface, |_| true), None);
    let folder = run_of(&["secret", "set", "A", "--folder", "."]);
    assert_eq!(
        help::admission(&folder.surface, |_| false),
        Some(crate::release::ReleaseFeature::FolderSecrets)
    );
    let host_route = run_of(&["agent", "start", "x"]);
    assert_eq!(help::admission(&host_route.surface, |_| false), None);
}

/// `scripts/secret-inventory-e2e.py` needs the example and the experiment
/// hint on the missing `--folder` value, whatever the profile enables.
#[test]
fn a_missing_experiment_value_carries_static_guidance() {
    let text = error_text(&["secret", "set", "DEMO_TOKEN", "--folder"]);
    assert!(text.contains("--folder ."), "{text}");
    assert!(text.contains("Usage:"), "{text}");
    assert!(
        text.contains(&format!("{BIN} experiments enable folder_secrets")),
        "{text}"
    );
}

/// Removing `--profile` or a display request never hands a token to another
/// argument: wherever the selector sits, the outcome is the same, or a usage
/// error. Round 1's probe and its moved-selector twin.
#[test]
fn removal_never_rebinds_a_value() {
    for parts in [
        &[
            "--profile",
            "t1",
            "pane",
            "info",
            "--previous",
            "--help",
            "3",
        ][..],
        &[
            "pane",
            "info",
            "--previous",
            "--profile",
            "t1",
            "--help",
            "3",
        ],
        &[
            "pane",
            "info",
            "--previous",
            "--help",
            "--profile",
            "t1",
            "3",
        ],
        &["pane", "list", "--context", "--help", "3"],
        &["pane", "list", "--context", "--profile", "t1", "3"],
    ] {
        expect(parts, "rebound");
    }
    help_of(&["pane", "list", "--context", "--help"]);
    let run = run_of(&["pane", "list", "--context", "--profile", "t1", "--json"]);
    assert_eq!(run.selection, named("t1"));
    assert_eq!(
        help_of(&["pane", "info", "--previous", "3", "--help"]).0,
        path(&["pane", "info"])
    );
    // A required value cannot be skipped by a removed token.
    expect(
        &["secret", "set", "A", "--folder", "--profile", "t1", "."],
        "grammar",
    );
}

/// A value for `arg` that its parser accepts.
fn sample_value(arg: &clap::Arg) -> String {
    arg.get_possible_values()
        .first()
        .map(|value| value.get_name().to_string())
        .unwrap_or_else(|| "1".to_string())
}

/// argv shapes for one command: with and without its positionals filled, and
/// each of its own long options at every value count it accepts (up to two).
fn shapes(node: &Command, p: &[String]) -> Vec<Vec<String>> {
    let bare: Vec<String> = std::iter::once(BIN.to_string())
        .chain(p.iter().cloned())
        .collect();
    let mut filled = bare.clone();
    filled.extend(node.get_positionals().map(sample_value));
    let mut out = vec![bare.clone(), filled.clone()];
    for arg in node
        .get_arguments()
        .filter(|a| !a.is_positional() && !a.is_global_set() && takes_values(a))
    {
        let Some(long) = arg.get_long() else { continue };
        let (min, max) = values(arg);
        for count in min..=max.min(2) {
            for base in [&bare, &filled] {
                let mut shape = base.clone();
                if arg.is_require_equals_set() && count == 1 {
                    shape.push(format!("--{long}={}", sample_value(arg)));
                } else {
                    shape.push(format!("--{long}"));
                    shape.extend(std::iter::repeat_n(sample_value(arg), count));
                }
                out.push(shape);
            }
        }
    }
    out.dedup();
    out
}

/// The oracle's grammar: the public grammar (run requirements waived when
/// help is being asked for), `--profile` declared as a global option whose
/// occurrences are counted, and help as a flag that does not stop the parse.
/// Built from clap alone.
fn oracle_grammar(
    occurrences: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    display: bool,
) -> Command {
    fn own_help(cmd: Command) -> Command {
        cmd.disable_help_flag(true).mut_subcommands(own_help)
    }
    let base = if display {
        display_grammar(grammar(BIN))
    } else {
        grammar(BIN)
    };
    own_help(base)
        .disable_version_flag(true)
        .arg(
            clap::Arg::new("oracle_profile")
                .long("profile")
                .global(true)
                .action(ArgAction::Append)
                .value_parser(move |value: &str| {
                    occurrences.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    Ok::<String, String>(value.to_string())
                }),
        )
        .arg(
            clap::Arg::new("oracle_help")
                .short('h')
                .long("help")
                .global(true)
                .action(ArgAction::SetTrue),
        )
}

type Reading = (Vec<String>, Vec<(Vec<String>, String, Vec<String>)>);

/// Clap's reading of an argv: the subcommand path, and the values every
/// non-global argument took from the command line.
fn clap_reading(tree: &Command, matches: &clap::ArgMatches) -> Reading {
    let mut path = Vec::new();
    let mut values = Vec::new();
    let mut level = Some(matches);
    while let Some(m) = level {
        let node = path
            .iter()
            .try_fold(tree, |node: &Command, name: &String| {
                node.find_subcommand(name)
            })
            .unwrap();
        for arg in node
            .get_arguments()
            .filter(|a| !a.is_global_set() && takes_values(a))
        {
            let id = arg.get_id().as_str();
            if m.value_source(id) == Some(clap::parser::ValueSource::CommandLine) {
                let raw: Vec<String> = m
                    .get_raw(id)
                    .map(|raw| raw.map(|v| v.to_string_lossy().into_owned()).collect())
                    .unwrap_or_default();
                values.push((path.clone(), id.to_string(), raw));
            }
        }
        level = m.subcommand().map(|(name, sub)| {
            path.push(name.to_string());
            sub
        });
    }
    values.sort();
    (path, values)
}

/// The front end's reading of a `Run`: its path and the values its captured
/// ownership assigns to each non-global argument.
fn run_reading(tree: &Command, run: &Run) -> Reading {
    let mut values: Vec<(Vec<String>, String, Vec<String>)> = Vec::new();
    for (token, owner) in run.tokens.iter().zip(&run.owners) {
        let Some(owner) = owner else { continue };
        let global = owner
            .path
            .iter()
            .try_fold(tree, |node: &Command, name: &String| {
                node.find_subcommand(name)
            })
            .and_then(|node| {
                node.get_arguments()
                    .find(|a| a.get_id() == owner.id.as_str())
            })
            .is_some_and(clap::Arg::is_global_set);
        if global {
            continue;
        }
        let value = token[owner.prefix..].to_string();
        match values
            .iter_mut()
            .find(|(path, id, _)| *path == owner.path && *id == owner.id)
        {
            Some((_, _, list)) => list.push(value),
            None => values.push((owner.path.clone(), owner.id.clone(), vec![value])),
        }
    }
    values.sort();
    (run.surface.path.clone(), values)
}

/// Clap fills an option given with no value token from its default-missing
/// value; the front end records no token for it. Drop such entries, found from
/// the literal option tokens the generator wrote.
fn without_bare_options(tree: &Command, argv: &[String], reading: &mut Reading) {
    reading.1.retain(|(path, id, _)| {
        let Some(arg) = path
            .iter()
            .try_fold(tree, |node: &Command, name: &String| {
                node.find_subcommand(name)
            })
            .and_then(|node| node.get_arguments().find(|a| a.get_id() == id.as_str()))
        else {
            return true;
        };
        let Some(long) = arg.get_long() else {
            return true;
        };
        let flag = format!("--{long}");
        let at = argv.iter().position(|token| *token == flag);
        // Bare when the option token takes no value through `=`-less
        // spelling, or is last, or is followed by another option.
        !at.is_some_and(|at| {
            arg.is_require_equals_set() || argv.get(at + 1).is_none_or(|next| next.starts_with('-'))
        })
    });
}

/// The differential, with its verdict computed from clap alone. For every
/// command path and option-arity shape, insert `--profile t1` or `--help` at
/// every position. Clap with `--profile` and help declared reads the argv as
/// written; clap without them reads the argv with the inserted tokens removed.
/// When the inserted tokens are Plexi controls and both readings agree, the
/// front end must accept with that reading; when they disagree, a removal
/// would move a value and the front end must refuse. When clap reads the
/// inserted tokens as data, the front end must accept with clap's reading.
/// The one documented exception: a control placed between a command whose
/// arguments conflict with subcommands and its subcommand never changes the
/// command (`a_selector_does_not_change_args_conflicts_with_subcommands_nodes`).
#[test]
fn ownership_equals_clap_at_every_insertion_position() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    let mut tree = grammar(BIN);
    tree.build();
    let occurrences = Arc::new(AtomicUsize::new(0));
    let oracle_display = oracle_grammar(occurrences.clone(), true);
    let oracle_run = oracle_grammar(occurrences.clone(), false);
    let without_display = display_grammar(grammar(BIN));
    let without_run = grammar(BIN);
    let (mut agreed, mut moved, mut data) = (0usize, 0usize, 0usize);
    for p in every_path() {
        let node = p
            .iter()
            .try_fold(&tree, |node: &Command, name: &String| {
                node.find_subcommand(name)
            })
            .unwrap();
        for shape in shapes(node, &p) {
            let removed_display = without_display.clone().try_get_matches_from(&shape);
            let removed_run = without_run.clone().try_get_matches_from(&shape);
            for insert in [&["--profile", "t1"][..], &["--help"]] {
                for at in 1..=shape.len() {
                    // argv[1..at] is the path before the inserted tokens.
                    let conflicting_node = at <= p.len()
                        && p[..at - 1]
                            .iter()
                            .try_fold(&tree, |node: &Command, name: &String| {
                                node.find_subcommand(name)
                            })
                            .is_some_and(Command::is_args_conflicts_with_subcommands_set);
                    if conflicting_node {
                        continue;
                    }
                    let mut x = shape.clone();
                    for (k, token) in insert.iter().enumerate() {
                        x.insert(at + k, token.to_string());
                    }
                    // Help is a display request only where clap reads it as
                    // the flag; run requirements are waived only then.
                    let help_control = insert[0] == "--help"
                        && oracle_display
                            .clone()
                            .try_get_matches_from(&x)
                            .is_ok_and(|m| m.get_flag("oracle_help"));
                    occurrences.store(0, Ordering::SeqCst);
                    let written = if help_control {
                        oracle_display.clone()
                    } else {
                        oracle_run.clone()
                    }
                    .try_get_matches_from(&x);
                    let removed = if help_control {
                        &removed_display
                    } else {
                        &removed_run
                    };
                    if written.is_err() && removed.is_err() {
                        continue;
                    }
                    let got = outcome(BIN, &x);
                    let accepted = !matches!(got, Outcome::UsageError(_));
                    if let Outcome::UsageError(UsageError::Internal(message)) = &got {
                        panic!("{x:?}: {message}");
                    }
                    let Ok(written) = written else {
                        // Clap rejects the argv as written but accepts it with
                        // the inserted tokens removed: removal would change it.
                        assert!(!accepted, "{x:?}: clap rejects it as written");
                        moved += 1;
                        continue;
                    };
                    let control = if insert[0] == "--profile" {
                        occurrences.load(Ordering::SeqCst) > 0
                    } else {
                        help_control
                    };
                    let mut as_written = clap_reading(&tree, &written);
                    let expected = if control {
                        let Ok(removed) = removed else {
                            assert!(!accepted, "{x:?}: clap rejects it without the control");
                            continue;
                        };
                        let mut without_reading = clap_reading(&tree, removed);
                        without_bare_options(&tree, &shape, &mut without_reading);
                        without_bare_options(&tree, &x, &mut as_written);
                        if as_written != without_reading {
                            assert!(!accepted, "{x:?}: removing the control moves a value");
                            moved += 1;
                            continue;
                        }
                        agreed += 1;
                        without_reading
                    } else {
                        without_bare_options(&tree, &x, &mut as_written);
                        data += 1;
                        as_written
                    };
                    assert!(accepted, "{x:?}: clap accepts it, the front end refused");
                    if let Outcome::Run(run) = &got {
                        assert_eq!(run_reading(&tree, run), expected, "{x:?}");
                    }
                }
            }
        }
    }
    assert!(
        agreed > 1000 && moved > 0 && data > 0,
        "agreed={agreed} moved={moved} data={data}"
    );
}

fn visible_paths(cmd: &Command, prefix: &mut Vec<String>, out: &mut Vec<Vec<String>>) {
    for sub in cmd.get_subcommands().filter(|s| s.get_name() != "help") {
        prefix.push(sub.get_name().to_string());
        out.push(prefix.clone());
        visible_paths(sub, prefix, out);
        prefix.pop();
    }
}

fn every_path() -> Vec<Vec<String>> {
    let mut tree = grammar(BIN);
    tree.build();
    let mut out = Vec::new();
    visible_paths(&tree, &mut Vec::new(), &mut out);
    out
}

/// Help is valid at every path and at every ancestor position, whatever the
/// command requires to run; an unknown option beside it is a usage error.
#[test]
fn every_command_accepts_help() {
    for p in every_path() {
        let words: Vec<&str> = p.iter().map(String::as_str).collect();
        for flag in ["--help", "-h"] {
            let mut parts = words.clone();
            parts.push(flag);
            assert_eq!(help_of(&parts).0, p, "{parts:?}");
            for at in 1..words.len() {
                let mut parts = words.clone();
                parts.insert(at, flag);
                assert_eq!(help_of(&parts).0, p[..at].to_vec(), "{parts:?}");
            }
            let mut bad = words.clone();
            bad.extend([flag, "--json=1663"]);
            expect(&bad, "grammar");
        }
        let mut parts = vec!["help"];
        parts.extend(&words);
        assert_eq!(
            help_of(&parts),
            (p.clone(), true, ProfileSelection::Default)
        );
    }
}

/// Help renders identically to clap's own display of the public grammar.
#[test]
fn help_matches_clap_native_rendering() {
    for p in every_path() {
        for (flag, long) in [("--help", true), ("-h", false)] {
            let mut native = vec![BIN.to_string()];
            native.extend(p.iter().cloned());
            native.push(flag.to_string());
            let error = match grammar(BIN).try_get_matches_from(&native) {
                Err(error) if error.kind() == ErrorKind::DisplayHelp => error,
                other => panic!("{native:?}: {:?}", other.err().map(|e| e.kind())),
            };
            if p.first().is_some_and(|name| name == "run") {
                continue;
            }
            let ours = render_node_help(BIN, &p, long).to_string();
            assert_eq!(ours, error.render().to_string(), "{native:?}");
        }
    }
}

/// The recognizer models only the clap features the tree uses. A new feature
/// must be modelled in `recognize` before the tree may use it.
#[test]
fn the_grammar_uses_only_modelled_features() {
    fn walk(cmd: &Command, path: &str) {
        assert!(!cmd.is_allow_missing_positional_set(), "{path}");
        assert!(!cmd.is_subcommand_precedence_over_arg_set(), "{path}");
        assert!(!cmd.is_allow_external_subcommands_set(), "{path}");
        assert!(!cmd.is_multicall_set(), "{path}");
        assert!(!cmd.is_no_binary_name_set(), "{path}");
        assert!(!cmd.is_disable_help_flag_set(), "{path}");
        assert!(
            cmd.get_short_flag().is_none() && cmd.get_long_flag().is_none(),
            "{path}"
        );
        let positionals: Vec<_> = cmd.get_positionals().collect();
        for (n, arg) in positionals.iter().enumerate() {
            assert!(!arg.is_last_set(), "{path} {}", arg.get_id());
            assert!(
                n + 1 == positionals.len() || !is_multiple(arg),
                "{path}: multiple positional {} is not last",
                arg.get_id()
            );
        }
        for arg in cmd.get_arguments() {
            let id = arg.get_id();
            assert!(!arg.is_allow_negative_numbers_set(), "{path} {id}");
            assert!(arg.get_value_terminator().is_none(), "{path} {id}");
            assert_ne!(
                arg.get_long(),
                Some(PROFILE_LONG),
                "{path} {id}: --profile is Plexi's own"
            );
        }
        for sub in cmd.get_subcommands() {
            if sub.get_name() != "help" {
                walk(sub, &format!("{path} {}", sub.get_name()));
            }
        }
    }
    let mut tree = grammar(BIN);
    tree.build();
    walk(&tree, BIN);
    // Clap generates `help`; no declared command may take the name.
    fn no_help(cmd: &Command) {
        for sub in cmd.get_subcommands() {
            assert_ne!(sub.get_name(), "help");
            no_help(sub);
        }
    }
    no_help(&Cli::command());
    let source = include_str!("args.rs");
    assert!(!source.contains("infer_subcommands") && !source.contains("infer_long_args"));
}

#[test]
fn cli_help_snapshots() {
    for (name, parts) in [
        ("short", &["-h"][..]),
        ("long", &["--help"]),
        ("typo", &["pnae"]),
        ("typo-help", &["pnae", "-h"]),
    ] {
        let args: Vec<String> = std::iter::once("plexi")
            .chain(parts.iter().copied())
            .map(str::to_string)
            .collect();
        let output = match outcome("plexi", &args) {
            Outcome::Help(help) => {
                assert!(help.path.is_empty());
                crate::cli::help::grouped_help(&grammar("plexi"), "plexi", true)
            }
            Outcome::UsageError(error) => usage_error_text("plexi", &error),
            other => panic!("{parts:?}: {}", kind(&other)),
        };
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!("src/cli/snapshots/help-{name}.txt"));
        if std::env::var_os("UPDATE_HELP_SNAPSHOTS").is_some() {
            std::fs::write(&path, &output).unwrap();
        }
        assert_eq!(std::fs::read_to_string(path).unwrap(), output, "{name}");
    }
}

#[test]
fn invoked_bin_name_strips_paths_and_exe() {
    for (arg0, expected) in [
        ("plexi-alpha", "plexi-alpha"),
        ("/path/to/plexi-alpha.exe", "plexi-alpha"),
        ("C:\\bin\\plexi-alpha.EXE", "plexi-alpha"),
        ("plexi", "plexi"),
        ("", "plexi"),
    ] {
        assert_eq!(invoked_bin_name(Some(arg0)), expected, "{arg0:?}");
    }
    assert_eq!(invoked_bin_name(None), "plexi");
}

/// Round 1 P2s on PR #3269.
#[test]
fn help_subcommand_validates_its_own_tokens() {
    expect(&["help", "host", "--help=oops"], "grammar");
    expect(&["host", "--help=oops"], "grammar");
    assert_eq!(help_of(&["help", "host", "help"]).0, path(&["host"]));
    let text = render_node_help(BIN, &path(&["host"]), true).to_string();
    assert!(text.contains(&format!("Usage: {BIN} host")), "{text}");
}

#[test]
fn completions_validate_the_shell() {
    expect(&["completions", "invalid-shell"], "grammar");
    expect(&["completions", "invalid-shell", "--help"], "grammar");
    expect(&["completions", "fish"], "completions");
}

#[test]
fn hidden_commands_are_never_suggested_or_completed() {
    for parts in [
        &["agent", "tem"][..],
        &["agent", "--json", "tem"],
        &["agent", "--json", "flee"],
    ] {
        let text = error_text(parts);
        assert!(
            !text.contains("team") && !text.contains("fleet"),
            "{parts:?}: {text}"
        );
    }
    for shell in ["fish", "zsh", "bash"] {
        let script =
            String::from_utf8(crate::cli::completions::render_completions(shell, BIN).unwrap())
                .unwrap();
        for hidden in ["assistant", "agents-dashboard", "ledger"] {
            assert!(
                !script.contains(&format!("'{hidden}'"))
                    && !script.contains(&format!("\"{hidden}\""))
                    && !script.contains(&format!(" {hidden} "))
                    && !script.contains(&format!("-a \"{hidden}\"")),
                "{shell} completes hidden {hidden}"
            );
        }
    }
}

#[test]
fn public_help_states_no_profile_setting_and_names_the_binary() {
    let text = render_node_help(BIN, &path(&["secret", "set"]), true).to_string();
    assert!(!text.contains("Experiment off"), "{text}");
    assert!(
        text.contains(&format!("{BIN} experiments enable folder_secrets")),
        "{text}"
    );
}

#[test]
fn a_duplicate_option_gets_no_missing_value_note() {
    let text = error_text(&["secret", "set", "A", "--folder", ".", "--folder", "."]);
    assert!(!text.contains("Example: --folder"), "{text}");
    assert_eq!(text.matches("Usage:").count(), 1, "{text}");
}

#[cfg(unix)]
#[test]
fn non_utf8_argv_is_a_usage_error() {
    use std::os::unix::ffi::OsStringExt;
    let argv = vec![
        std::ffi::OsString::from(BIN),
        std::ffi::OsString::from("--help"),
        std::ffi::OsString::from("--profile"),
        std::ffi::OsString::from_vec(vec![0xff]),
    ];
    assert!(matches!(
        parse(&argv).outcome,
        Outcome::UsageError(UsageError::NotUtf8 { index: 3 })
    ));
}

/// The host's trace record comes from clap's own parse of the argv: the
/// subcommand path and the options given, never a value, never argv[0], and
/// nothing forwarded to a child.
#[test]
fn the_host_record_keeps_no_sender_value() {
    let record = |argv: &[&str]| {
        crate::cli::trace::host_record(&argv.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    };
    for argv in [
        &["plexi", "pane", "send", "SENTINEL"][..],
        &["plexi", "--profile", "SENTINEL", "host", "stop"],
        &[
            "plexi",
            "--profile",
            "one",
            "host",
            "stop",
            "--profile",
            "SENTINEL",
        ],
        &["plexi", "pane", "send", "42", "SENTINEL"],
        &["plexi", "pane", "send", "42", "--text=SENTINEL"],
        &["/home/SENTINEL/bin/plexi-x", "pane", "list"],
        &["plexi", "run", "example", "--", "--SENTINEL", "--json"],
    ] {
        let recorded = record(argv);
        assert!(!recorded.join(" ").contains("SENTINEL"), "{recorded:?}");
        assert_eq!(recorded[0], "plexi", "{recorded:?}");
    }
    assert_eq!(
        record(&["plexi-alpha", "pane", "new", "--right"]),
        ["plexi", "pane", "new", "--right"]
    );
    assert_eq!(
        record(&["plexi", "pane", "new", "-r"]),
        ["plexi", "pane", "new", "--right"]
    );
    // Forwarded child arguments are values: their options are never recorded.
    assert_eq!(
        record(&["plexi", "run", "example", "arg", "--json"]),
        ["plexi", "run"]
    );
    assert_eq!(
        record(&["plexi", "run", "example", "--", "--json"]),
        ["plexi", "run"]
    );
    assert_eq!(
        record(&[
            "plexi",
            "--socket=/tmp/x",
            "run",
            "example",
            "arg",
            "--json"
        ]),
        ["plexi", "run", "--socket"]
    );
    // Real options after positionals are kept.
    assert_eq!(
        record(&["plexi", "pane", "send", "42", "hello", "--submit"]),
        ["plexi", "pane", "send", "--submit"]
    );
    assert_eq!(
        record(&["plexi", "pane", "new", "--name", "worker", "--right"]),
        ["plexi", "pane", "new", "--name", "--right"]
    );
    // An argv clap rejects records only the fixed label.
    assert_eq!(
        record(&["plexi", "pane", "submit", "42", "SENTINEL"]),
        ["plexi", "<unparsed>"]
    );
    assert_eq!(record(&["plexi", "pane", "send"]), ["plexi", "<unparsed>"]);
}

#[test]
fn the_rebinding_diagnostic_names_no_values() {
    let assigned = |id: &str, value: &str| Assigned {
        path: path(&["pane", "info"]),
        id: id.to_string(),
        values: vec![value.to_string()],
    };
    let line = rebinding_diagnostic(
        &[assigned("pane_id", "SENTINEL_A")],
        &[assigned("previous", "SENTINEL_B")],
    );
    assert!(!line.contains("SENTINEL"), "{line}");
    assert!(line.contains("pane info:pane_id") && line.contains("pane info:previous"));
}

#[test]
fn hidden_aliases_and_arguments_are_never_suggested_or_completed() {
    let text = error_text(&["plugin", "delet"]);
    assert!(!text.contains("`delete`"), "{text}");
    let text = error_text(&["secret", "set", "KEY", "--folde"]);
    assert!(!text.contains("--folder"), "{text}");
    for shell in ["bash", "zsh", "fish", "elvish", "powershell"] {
        let script =
            String::from_utf8(crate::cli::completions::render_completions(shell, BIN).unwrap())
                .unwrap();
        assert!(!script.contains("--folder"), "{shell} completes --folder");
    }
}

#[test]
fn help_names_the_invoked_binary_everywhere() {
    for p in [path(&["secret", "set"]), path(&["host", "start"])] {
        let text = render_node_help(BIN, &p, true).to_string();
        for line in text.lines() {
            assert!(
                !line.contains("plexi ") && !line.contains("plexi-pr-2"),
                "{p:?}: {line}"
            );
        }
    }
}

#[test]
fn an_unknown_flag_in_a_display_cluster_is_named() {
    let text = error_text(&["-hx"]);
    assert!(text.contains("'-x'"), "{text}");
}

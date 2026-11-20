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
        Outcome::UsageError(UsageError::Separated { .. }) => "separated",
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
        text.contains("plexi experiments enable folder_secrets"),
        "{text}"
    );
}

/// A token that would become a pending option's value once `--profile` or a
/// display request is removed is refused rather than silently rebound.
#[test]
fn removal_never_rebinds_a_pending_option() {
    help_of(&["pane", "list", "--context", "--help"]);
    expect(&["pane", "list", "--context", "--help", "3"], "separated");
    expect(
        &["pane", "list", "--context", "--profile", "t1", "3"],
        "separated",
    );
    let run = run_of(&["pane", "list", "--context", "--profile", "t1", "--json"]);
    assert_eq!(run.selection, named("t1"));
    // A required value cannot be skipped by a removed token.
    expect(
        &["secret", "set", "A", "--folder", "--profile", "t1", "."],
        "grammar",
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

/// Recognition matches clap's own reading of every command path: the kept
/// tokens are argv unchanged and the path is clap's.
#[test]
fn recognition_agrees_with_clap_on_every_path() {
    let mut tree = grammar(BIN);
    tree.build();
    for p in every_path() {
        let words: Vec<&str> = p.iter().map(String::as_str).collect();
        let args = argv(&words);
        let r = recognize(&tree, &args);
        if r.stop.is_none() {
            assert_eq!(r.kept, args, "{args:?}");
            assert_eq!(r.path, p, "{args:?}");
        }
        let mut with = words.clone();
        with.extend(["--profile", "t1"]);
        let r = recognize(&tree, &argv(&with));
        assert_eq!(r.profiles, vec![Some("t1".to_string())], "{with:?}");
    }
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

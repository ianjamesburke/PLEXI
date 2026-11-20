//! The CLI front end: argv in, exactly one [`Outcome`] out.
//!
//! [`parse`] is the only reader of argv. Recognition walks the built public
//! clap tree token by token with clap 4.6.7's lexing rules, restricted to the
//! grammar features the tree uses (`the_grammar_uses_only_modelled_features`
//! fails when it starts using another). It records the occurrences of
//! Plexi's own `--profile` option and every display request (help flag, help
//! subcommand, version) with its location, and removes them from the token
//! stream before clap sees it, so they never change how clap recognizes the
//! remaining tokens. Clap then validates what is left: with run requirements
//! waived when display was requested, in full otherwise.
//!
//! The outcome decides what the process may do. Display outcomes and usage
//! errors read no profile and contact no host; only `Run` and `Launch` carry a
//! validated profile selection on to startup. The renderers take an outcome,
//! never argv.

use crate::cli::args::Cli;
use crate::cli::help::{self, Surface};
use crate::config::ProfileSelection;
use clap::error::{ContextKind, ContextValue, ErrorKind};
use clap::{ArgAction, Command, FromArgMatches};

/// Plexi's own option. The front end owns it at every command level; clap's
/// grammar never declares it.
const PROFILE_LONG: &str = "profile";

/// One invocation: the invoked binary name and what argv asks for.
pub struct Invocation {
    pub bin: String,
    pub outcome: Outcome,
}

pub enum Outcome {
    /// `-h`/`--help` or `help <path>`: help for `path`, where it was requested.
    Help(Help),
    /// `-V`/`--version` at the root.
    Version,
    UsageError(UsageError),
    Completions {
        shell: Option<String>,
    },
    Run(Box<Run>),
    Launch(Box<Launch>),
}

pub struct Help {
    pub path: Vec<String>,
    pub long: bool,
    /// Installed before rendering so `run --help` names the selected
    /// profile's paths. Selecting reads nothing.
    pub selection: ProfileSelection,
}

pub struct Run {
    pub cli: Cli,
    pub surface: Surface,
    pub selection: ProfileSelection,
    /// argv as given, and which argument owns each token's value: the input
    /// tracing records, captured here so tracing never reparses.
    pub tokens: Vec<String>,
    pub owners: Vec<Option<Owner>>,
}

/// Bare launch, or launch with a workspace operand (`cli.workspace_path`).
pub struct Launch {
    pub cli: Cli,
    pub selection: ProfileSelection,
}

pub enum UsageError {
    /// A malformed or repeated `--profile`.
    Profile(String),
    /// A retired namespace.
    Retired(&'static str),
    /// A word in a command position that names no command.
    UnknownCommand {
        path: Vec<String>,
        word: String,
        suggestion: Option<String>,
    },
    /// A command group invoked without its subcommand.
    MissingCommand { path: Vec<String> },
    /// Removing `--profile` or a display request would hand a token to a
    /// different argument than the one clap gives it with the token present.
    Rebound { argument: String },
    /// An argv token that is not UTF-8.
    NotUtf8 { index: usize },
    /// Clap's own diagnosis, with any static annotation for the argument.
    Grammar {
        error: clap::Error,
        note: Option<String>,
    },
    /// Recognition and the clap grammar disagreed; nothing runs.
    Internal(String),
}

/// The basename the user invoked, without a Windows `.exe` suffix.
pub fn invoked_bin_name(arg0: Option<&str>) -> String {
    arg0.and_then(|arg0| arg0.rsplit(['/', '\\']).next())
        .map(|base| {
            base.strip_suffix(".exe")
                .or_else(|| base.strip_suffix(".EXE"))
                .unwrap_or(base)
        })
        .filter(|s| !s.is_empty())
        .unwrap_or("plexi")
        .to_string()
}

/// The public grammar bound to the invoked binary name. Recognition,
/// validation and every renderer use this one tree.
fn grammar(bin: &str) -> Command {
    help::public_command(bin)
        .bin_name(bin.to_string())
        .display_name(bin.to_string())
}

/// The public grammar with run requirements waived: required arguments,
/// required subcommands, `requires` relationships and required groups. Values,
/// unknown tokens and conflicts are still validated.
fn display_grammar(cmd: Command) -> Command {
    let groups: Vec<String> = cmd.get_groups().map(|g| g.get_id().to_string()).collect();
    groups
        .into_iter()
        .fold(cmd, |cmd, id| cmd.mut_group(id, |g| g.required(false)))
        .subcommand_required(false)
        .arg_required_else_help(false)
        .mut_args(|arg| {
            arg.required(false)
                .required_unless_present(clap::builder::Resettable::<clap::Id>::Reset)
                .requires(clap::builder::Resettable::<clap::Id>::Reset)
        })
        .mut_subcommands(display_grammar)
}

/// Parse argv into one outcome. Reads nothing but argv, except that a word in
/// the root command position is checked for existence as a workspace path.
pub fn parse(argv: &[std::ffi::OsString]) -> Invocation {
    let arg0 = argv.first().map(|a| a.to_string_lossy().into_owned());
    let bin = invoked_bin_name(arg0.as_deref());
    let tokens: Result<Vec<String>, usize> = argv
        .iter()
        .enumerate()
        .map(|(index, token)| token.to_str().map(str::to_string).ok_or(index))
        .collect();
    let outcome = match tokens {
        Ok(tokens) => outcome(&bin, &tokens),
        Err(index) => Outcome::UsageError(UsageError::NotUtf8 { index }),
    };
    log::info!(
        "cli_front: bin={bin} outcome={}",
        match &outcome {
            Outcome::Help(_) => "help",
            Outcome::Version => "version",
            Outcome::UsageError(_) => "usage_error",
            Outcome::Completions { .. } => "completions",
            Outcome::Run(_) => "run",
            Outcome::Launch(_) => "launch",
        }
    );
    Invocation { bin, outcome }
}

fn outcome(bin: &str, argv: &[String]) -> Outcome {
    let public = grammar(bin);
    let mut tree = public.clone();
    tree.build();
    let r = recognize(&tree, argv);
    if let Some(Stop::Own(error)) = r.stop {
        return Outcome::UsageError(error);
    }
    let selection = match select_profile(&r.profiles) {
        Ok(selection) => selection,
        Err(message) => return Outcome::UsageError(UsageError::Profile(message)),
    };
    if let Some(word) = &r.root_operand {
        if let Some(error) = root_operand_error(&tree, word) {
            return Outcome::UsageError(error);
        }
    }
    if let Some(Stop::Clap { prefix }) = r.stop {
        // Clap reports the token recognition stopped at. The prefix ends at
        // that token, so nothing after it can change clap's diagnosis.
        let error = match display_grammar(public).try_get_matches_from(&r.kept[..prefix]) {
            Err(error) => error,
            // Clap accepts what recognition rejected only when a removed
            // token let clap read the stop token as an earlier option's value.
            Ok(matches) => {
                let stop_token = &r.kept[prefix - 1];
                let taker = parsed_values(&tree, &matches)
                    .into_iter()
                    .find(|parsed| parsed.values.last() == Some(stop_token));
                return Outcome::UsageError(match taker {
                    Some(parsed) => UsageError::Rebound {
                        argument: parsed.id,
                    },
                    None => UsageError::Internal(format!(
                        "argument recognition stopped at {stop_token:?} but the grammar accepted it"
                    )),
                });
            }
        };
        return grammar_error(bin, &r.path, error);
    }
    let help_request = r.help.clone().or_else(|| r.help_subcommand.clone());
    if help_request.is_some() || r.version {
        let matches = match display_grammar(public).try_get_matches_from(&r.kept) {
            Ok(matches) => matches,
            Err(error) => return grammar_error(bin, &r.path, error),
        };
        if let Some(error) =
            path_disagreement(&r.path, &matches).or_else(|| rebinding(&tree, &r, argv, &matches))
        {
            return Outcome::UsageError(error);
        }
        return match help_request {
            Some((path, long)) => Outcome::Help(Help {
                path,
                long,
                selection,
            }),
            None => Outcome::Version,
        };
    }
    let mut matches = match public.try_get_matches_from(&r.kept) {
        Ok(matches) => matches,
        Err(error) => return grammar_error(bin, &r.path, error),
    };
    if let Some(error) =
        path_disagreement(&r.path, &matches).or_else(|| rebinding(&tree, &r, argv, &matches))
    {
        return Outcome::UsageError(error);
    }
    let surface = Surface::capture(&matches);
    let cli = match Cli::from_arg_matches_mut(&mut matches) {
        Ok(cli) => cli,
        Err(error) => return grammar_error(bin, &r.path, error),
    };
    match cli.command {
        Some(crate::cli::args::Commands::Completions { shell }) => Outcome::Completions { shell },
        Some(_) => Outcome::Run(Box::new(Run {
            cli,
            surface,
            selection,
            tokens: argv.to_vec(),
            owners: r.owners,
        })),
        None => Outcome::Launch(Box::new(Launch { cli, selection })),
    }
}

fn select_profile(occurrences: &[Option<String>]) -> Result<ProfileSelection, String> {
    match occurrences {
        [] => Ok(ProfileSelection::Default),
        [Some(raw)] => ProfileSelection::parse(raw),
        [None] => Err(
            "--profile needs a value: a profile name, or `default` for this binary's own profile"
                .to_string(),
        ),
        many => Err(format!(
            "--profile was given {} times; give it once",
            many.len()
        )),
    }
}

/// The root's workspace operand: a retired namespace, or a command-shaped
/// word that names neither a command nor an existing path.
fn root_operand_error(tree: &Command, word: &str) -> Option<UsageError> {
    if word == "app" {
        return Some(UsageError::Retired(
            "the `app` namespace was removed; use `plugin` for packages, `view open/render` for views, or `pane action` for a running view",
        ));
    }
    if word.contains('/')
        || word.contains('\\')
        || word.starts_with('.')
        || std::path::Path::new(word).exists()
    {
        return None;
    }
    Some(UsageError::UnknownCommand {
        path: Vec::new(),
        word: word.to_string(),
        suggestion: help::closest_subcommand_suggestion(word, tree).map(str::to_string),
    })
}

fn path_disagreement(path: &[String], matches: &clap::ArgMatches) -> Option<UsageError> {
    let mut chain = Vec::new();
    let mut current = matches;
    while let Some((name, sub)) = current.subcommand() {
        chain.push(name.to_string());
        current = sub;
    }
    (chain != path).then(|| {
        log::error!("cli_front: recognized path {path:?} but clap matched {chain:?}");
        UsageError::Internal(format!(
            "argument recognition found the command {path:?} but the grammar matched {chain:?}"
        ))
    })
}

/// Map a clap error to a usage error. Missing-command diagnostics render from
/// the public tree's visible children; clap's own lists include hidden ones.
fn grammar_error(bin: &str, path: &[String], error: clap::Error) -> Outcome {
    let error = match error.kind() {
        // Recognition removes every display request before clap parses.
        ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => UsageError::Internal(format!(
            "the grammar produced a display request recognition did not see: {}",
            error.kind()
        )),
        ErrorKind::MissingSubcommand | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => {
            UsageError::MissingCommand {
                path: path.to_vec(),
            }
        }
        _ => {
            let note = missing_value_note(bin, path, &error);
            UsageError::Grammar {
                error: without_hidden_suggestions(bin, path, error),
                note,
            }
        }
    };
    Outcome::UsageError(error)
}

/// Clap suggests from every name it knows, hidden ones included. Public
/// diagnostics suggest only what public help shows.
fn without_hidden_suggestions(bin: &str, path: &[String], mut error: clap::Error) -> clap::Error {
    let mut tree = grammar(bin);
    tree.build();
    let Some(node) = node_at(&tree, path) else {
        return error;
    };
    if let Some(ContextValue::String(suggested)) = error.get(ContextKind::SuggestedArg) {
        let name = suggested.trim_start_matches('-');
        let hidden = node.get_arguments().any(|arg| {
            arg.is_hide_set()
                && (arg.get_long() == Some(name)
                    || arg.get_all_aliases().is_some_and(|a| a.contains(&name))
                    || (name.chars().count() == 1 && arg.get_short() == name.chars().next()))
        });
        if hidden {
            error.remove(ContextKind::SuggestedArg);
        }
    }
    if let Some(ContextValue::Strings(names)) = error.get(ContextKind::SuggestedSubcommand) {
        let visible: Vec<String> = names
            .iter()
            .filter(|name| {
                node.get_subcommands().any(|sub| {
                    !sub.is_hide_set()
                        && (sub.get_name() == name.as_str()
                            || sub
                                .get_visible_aliases()
                                .any(|alias| alias == name.as_str()))
                })
            })
            .cloned()
            .collect();
        if visible.is_empty() {
            error.remove(ContextKind::SuggestedSubcommand);
        } else {
            error.insert(
                ContextKind::SuggestedSubcommand,
                ContextValue::Strings(visible),
            );
        }
    }
    error
}

/// Static guidance for a missing value of an experiment-owned option. It names
/// the experiment without claiming whether any profile has it on.
fn missing_value_note(bin: &str, path: &[String], error: &clap::Error) -> Option<String> {
    let missing_value = error.kind() == ErrorKind::InvalidValue
        && matches!(error.get(ContextKind::InvalidValue), Some(ContextValue::String(v)) if v.is_empty());
    if !missing_value {
        return None;
    }
    let Some(ContextValue::String(arg)) = error.get(ContextKind::InvalidArg) else {
        return None;
    };
    let owner = crate::release::ReleaseFeature::ALL
        .iter()
        .find_map(|feature| {
            feature.owned_paths().iter().find_map(|gate| match *gate {
                crate::release::GatePath::Arg(p, id)
                    if p.iter().eq(path.iter()) && is_option(arg, id) =>
                {
                    Some((*feature, id))
                }
                _ => None,
            })
        })?;
    let (feature, id) = owner;
    let mut cmd = grammar(bin);
    cmd.build();
    let node = path
        .iter()
        .try_fold(&cmd, |node, name| node.find_subcommand(name))?;
    let usage = node.clone().render_usage().to_string();
    let example = if id == "folder" {
        "Example: --folder . (or --folder <DIR> for another directory)\n"
    } else {
        ""
    };
    Some(format!(
        "{} {arg}\n{example}--{id}: {}\n",
        usage.trim_end(),
        crate::release::experiment_text(feature, bin)
    ))
}

fn is_option(rendered: &str, id: &str) -> bool {
    rendered
        .strip_prefix("--")
        .and_then(|rest| rest.strip_prefix(id))
        .is_some_and(|rest| rest.is_empty() || rest.starts_with([' ', '=']))
}

// --- Recognition -----------------------------------------------------------

#[derive(Default)]
struct Recognized {
    /// One entry per `--profile` occurrence; `None` when its value is missing.
    profiles: Vec<Option<String>>,
    /// The first help flag: the path where it appeared, and its long form.
    help: Option<(Vec<String>, bool)>,
    /// `help <path>`: the resolved path (always long help).
    help_subcommand: Option<(Vec<String>, bool)>,
    version: bool,
    /// Canonical subcommand path reached.
    path: Vec<String>,
    /// The first positional at the root (`plexi <path>`).
    root_operand: Option<String>,
    /// argv[0] and every token left for clap.
    kept: Vec<String>,
    /// Per argv index: the argument whose value the token carries.
    owners: Vec<Option<Owner>>,
    /// Every value-taking option given, by command path and id.
    given: Vec<(Vec<String>, String)>,
    stop: Option<Stop>,
}

/// The argument a token's value belongs to: its command path, argument id,
/// and the byte length of any option prefix (`--name=`, `-x`) before the value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Owner {
    pub path: Vec<String>,
    pub id: String,
    pub prefix: usize,
}

impl Owner {
    fn new(path: &[String], arg: &clap::Arg, prefix: usize) -> Self {
        Self {
            path: path.to_vec(),
            id: arg.get_id().as_str().to_string(),
            prefix,
        }
    }
}

/// Which argument owns each argv token's value, for a complete invocation
/// clap confirms. `None` when recognition stops or clap's reading differs:
/// callers fail closed. For argv received from another process; an
/// invocation's own ownership travels in [`Run`].
pub(crate) fn argument_owners(argv: &[String]) -> Option<Vec<Option<Owner>>> {
    let bin = "plexi";
    let public = grammar(bin);
    let mut tree = public.clone();
    tree.build();
    let r = recognize(&tree, argv);
    if r.stop.is_some() || r.help_subcommand.is_some() || r.help.is_some() || r.version {
        return None;
    }
    // The full grammar, run requirements included: only an invocation that
    // could have run is confirmed. Anything incomplete fails closed.
    let matches = public.try_get_matches_from(&r.kept).ok()?;
    if path_disagreement(&r.path, &matches).is_some()
        || rebinding(&tree, &r, argv, &matches).is_some()
    {
        return None;
    }
    Some(r.owners)
}

/// The values one non-global argument received at one command path.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Assigned {
    path: Vec<String>,
    id: String,
    values: Vec<String>,
}

fn node_at<'a>(tree: &'a Command, path: &[String]) -> Option<&'a Command> {
    path.iter()
        .try_fold(tree, |node, name| node.find_subcommand(name))
}

fn arg_at<'a>(tree: &'a Command, path: &[String], id: &str) -> Option<&'a clap::Arg> {
    node_at(tree, path)?
        .get_arguments()
        .find(|arg| arg.get_id() == id)
}

/// Every value recognition assigned, by argument, in argv order.
fn recognized_values(tree: &Command, r: &Recognized, argv: &[String]) -> Vec<Assigned> {
    let mut out: Vec<Assigned> = Vec::new();
    for (token, owner) in argv.iter().zip(&r.owners) {
        let Some(owner) = owner else { continue };
        if arg_at(tree, &owner.path, &owner.id).is_some_and(clap::Arg::is_global_set) {
            continue;
        }
        let value = token.get(owner.prefix..).unwrap_or_default().to_string();
        match out
            .iter_mut()
            .find(|a| a.path == owner.path && a.id == owner.id)
        {
            Some(assigned) => assigned.values.push(value),
            None => out.push(Assigned {
                path: owner.path.clone(),
                id: owner.id.clone(),
                values: vec![value],
            }),
        }
    }
    out
}

/// Every value clap assigned from the command line, by argument. Global
/// arguments are left out: clap copies them across levels, and none can be
/// rebound (`the_grammar_uses_only_modelled_features`).
fn parsed_values(tree: &Command, matches: &clap::ArgMatches) -> Vec<Assigned> {
    let mut out = Vec::new();
    let mut path: Vec<String> = Vec::new();
    let mut level = Some(matches);
    while let Some(m) = level {
        if let Some(node) = node_at(tree, &path) {
            for arg in node
                .get_arguments()
                .filter(|a| !a.is_global_set() && takes_values(a))
            {
                let id = arg.get_id().as_str();
                if m.value_source(id) != Some(clap::parser::ValueSource::CommandLine) {
                    continue;
                }
                let values: Vec<String> = m
                    .try_get_raw(id)
                    .ok()
                    .flatten()
                    .map(|raw| raw.map(|v| v.to_string_lossy().into_owned()).collect())
                    .unwrap_or_default();
                out.push(Assigned {
                    path: path.clone(),
                    id: id.to_string(),
                    values,
                });
            }
        }
        level = m.subcommand().map(|(name, sub)| {
            path.push(name.to_string());
            sub
        });
    }
    out
}

/// Clap's reading of the kept tokens must give every value to the argument
/// recognition gave it in argv as written. A difference means removing
/// `--profile` or a display request handed a token to another argument.
fn rebinding(
    tree: &Command,
    r: &Recognized,
    argv: &[String],
    matches: &clap::ArgMatches,
) -> Option<UsageError> {
    let mut ours = recognized_values(tree, r, argv);
    let mut theirs: Vec<Assigned> = parsed_values(tree, matches)
        .into_iter()
        .filter(|parsed| {
            // An option given with no value token holds clap's default-missing
            // value. A token rebound to it would be missing from the argument
            // recognition gave it to, so the comparison still sees the move.
            let given_bare = !ours
                .iter()
                .any(|a| a.path == parsed.path && a.id == parsed.id)
                && r.given
                    .iter()
                    .any(|(path, id)| *path == parsed.path && *id == parsed.id);
            !given_bare
        })
        .collect();
    ours.sort();
    theirs.sort();
    if ours == theirs {
        return None;
    }
    let differing = ours
        .iter()
        .chain(&theirs)
        .find(|a| !ours.contains(a) || !theirs.contains(a))
        .map(|a| a.id.clone())
        .unwrap_or_default();
    log::info!("cli_front: removal would rebind values: ours={ours:?} clap={theirs:?}");
    Some(UsageError::Rebound {
        argument: differing,
    })
}

enum Stop {
    /// Clap rejects the token sequence `kept[..prefix]`.
    Clap {
        prefix: usize,
    },
    Own(UsageError),
}

#[derive(Clone, Copy)]
enum State<'a> {
    Done,
    /// An option collecting values, with the count collected so far.
    Opt(&'a clap::Arg, usize),
    /// A multiple-value positional collecting values.
    Pos(&'a clap::Arg),
}

impl<'a> State<'a> {
    fn hyphen_value_arg(self) -> Option<&'a clap::Arg> {
        match self {
            State::Opt(arg, _) | State::Pos(arg) if arg.is_allow_hyphen_values_set() => Some(arg),
            _ => None,
        }
    }
}

fn values(arg: &clap::Arg) -> (usize, usize) {
    let range = arg.get_num_args().unwrap_or_else(|| 1.into());
    (range.min_values(), range.max_values())
}

fn takes_values(arg: &clap::Arg) -> bool {
    arg.get_num_args()
        .unwrap_or_else(|| 1.into())
        .takes_values()
}

fn is_multiple(arg: &clap::Arg) -> bool {
    let (min, max) = values(arg);
    min != max || min > 1 || matches!(arg.get_action(), ArgAction::Append)
}

fn is_help(arg: &clap::Arg) -> bool {
    matches!(
        arg.get_action(),
        ArgAction::Help | ArgAction::HelpShort | ArgAction::HelpLong
    )
}

fn find_long<'a>(node: &'a Command, name: &str) -> Option<&'a clap::Arg> {
    node.get_arguments().find(|a| {
        a.get_long() == Some(name)
            || a.get_all_aliases()
                .is_some_and(|aliases| aliases.contains(&name))
    })
}

fn find_short(node: &Command, c: char) -> Option<&clap::Arg> {
    node.get_arguments().find(|a| {
        a.get_short() == Some(c)
            || a.get_all_short_aliases()
                .is_some_and(|aliases| aliases.contains(&c))
    })
}

fn positional_at(node: &Command, index: usize) -> Option<&clap::Arg> {
    node.get_positionals()
        .find(|a| a.get_index() == Some(index))
}

/// Clap lexes these as option tokens rather than values.
fn looks_like_option(token: &str) -> bool {
    token == "--" || (token.starts_with('-') && token.len() > 1)
}

/// Clap's subcommand recognition: by name or any alias, unless the node's
/// arguments conflict with subcommands and one was already given.
fn possible_subcommand<'a>(node: &'a Command, token: &str, arg_given: bool) -> Option<&'a Command> {
    if node.is_args_conflicts_with_subcommands_set() && arg_given {
        return None;
    }
    node.find_subcommand(token)
}

fn recognize(tree: &Command, argv: &[String]) -> Recognized {
    let mut r = Recognized {
        kept: argv.first().cloned().into_iter().collect(),
        owners: vec![None; argv.len()],
        ..Recognized::default()
    };
    let mut node = tree;
    let mut state = State::Done;
    let mut pos_counter = 1usize;
    let mut arg_given = false;
    let mut trailing = false;
    let mut i = 1;
    while i < argv.len() {
        let token = &argv[i];
        if !trailing {
            if matches!(state, State::Done) {
                if let Some(sub) = possible_subcommand(node, token, arg_given) {
                    if sub.get_name() == "help" && !node.is_disable_help_subcommand_set() {
                        recognize_help_subcommand(&mut r, node, &argv[i + 1..], i);
                        return r;
                    }
                    r.path.push(sub.get_name().to_string());
                    r.kept.push(token.clone());
                    node = sub;
                    state = State::Done;
                    pos_counter = 1;
                    arg_given = false;
                    i += 1;
                    continue;
                }
            }
            let hyphen_value = state.hyphen_value_arg().is_some();
            if token == "--" && !hyphen_value {
                trailing = true;
                r.kept.push(token.clone());
                i += 1;
                continue;
            }
            let long = token
                .strip_prefix("--")
                .filter(|long| !hyphen_value && !long.is_empty());
            if let Some(long) = long {
                let (name, inline) = match long.split_once('=') {
                    Some((name, value)) => (name, Some(value)),
                    None => (long, None),
                };
                if name == PROFILE_LONG {
                    if let Some(stop) = missing_value_stop(&r, state) {
                        r.stop = Some(stop);
                        return r;
                    }
                    let consumed = match inline {
                        Some(value) => {
                            r.profiles.push(Some(value.to_string()));
                            1
                        }
                        None => match argv.get(i + 1).filter(|v| !looks_like_option(v)) {
                            Some(value) => {
                                r.profiles.push(Some(value.clone()));
                                2
                            }
                            None => {
                                r.profiles.push(None);
                                1
                            }
                        },
                    };
                    state = State::Done;
                    i += consumed;
                    continue;
                }
                if let Some(arg) = find_long(node, name) {
                    if is_display(arg) {
                        if inline.is_some() {
                            r.kept.push(token.clone());
                            return stop_at_end(r);
                        }
                        if let Some(stop) = missing_value_stop(&r, state) {
                            r.stop = Some(stop);
                            return r;
                        }
                        record_display(&mut r, arg, true);
                        state = State::Done;
                        i += 1;
                        continue;
                    }
                    arg_given = true;
                    r.kept.push(token.clone());
                    if takes_values(arg) {
                        r.given.push((r.path.clone(), arg.get_id().to_string()));
                    }
                    if inline.is_some() && takes_values(arg) {
                        r.owners[i] = Some(Owner::new(&r.path, arg, name.len() + 3));
                    }
                    i += 1;
                    state = if !takes_values(arg) {
                        if inline.is_some() {
                            return stop_at_end(r);
                        }
                        State::Done
                    } else if arg.is_require_equals_set() && inline.is_none() {
                        if values(arg).0 > 0 {
                            return stop_at_end(r);
                        }
                        State::Done
                    } else if inline.is_some() {
                        State::Done
                    } else {
                        State::Opt(arg, 0)
                    };
                    continue;
                }
                if !positional_at(node, pos_counter)
                    .is_some_and(|p| p.is_allow_hyphen_values_set() && !p.is_last_set())
                {
                    r.kept.push(token.clone());
                    return stop_at_end(r);
                }
                // Otherwise a hyphenated value for the positional.
            } else if !hyphen_value && token.starts_with('-') && token.len() > 1 && token != "--" {
                let flags = &token[1..];
                let positional_takes_it = positional_at(node, pos_counter).is_some_and(|p| {
                    p.is_allow_hyphen_values_set()
                        && !p.is_last_set()
                        && flags.chars().any(|c| find_short(node, c).is_none())
                });
                if !positional_takes_it {
                    // One short cluster: flags, then at most one option whose
                    // value is the rest of the cluster or the next token.
                    let mut kept_flags = String::new();
                    let mut display: Vec<&clap::Arg> = Vec::new();
                    let mut next_state = State::Done;
                    for (at, c) in flags.char_indices() {
                        let Some(arg) = find_short(node, c) else {
                            // Clap names the unknown flag; display flags
                            // already recognized are not replayed to it.
                            r.kept.push(format!("-{kept_flags}{c}"));
                            return stop_at_end(r);
                        };
                        if is_display(arg) {
                            display.push(arg);
                            continue;
                        }
                        arg_given = true;
                        kept_flags.push(c);
                        if !takes_values(arg) {
                            continue;
                        }
                        r.given.push((r.path.clone(), arg.get_id().to_string()));
                        let rest = &flags[at + c.len_utf8()..];
                        if arg.is_require_equals_set() && !rest.starts_with('=') {
                            if values(arg).0 == 0 {
                                continue;
                            }
                            r.kept.push(format!("-{kept_flags}"));
                            return stop_at_end(r);
                        }
                        if rest.is_empty() {
                            next_state = State::Opt(arg, 0);
                        } else {
                            let eq = usize::from(rest.starts_with('='));
                            r.owners[i] =
                                Some(Owner::new(&r.path, arg, 1 + at + c.len_utf8() + eq));
                            kept_flags.push_str(rest);
                        }
                        break;
                    }
                    if !display.is_empty() {
                        if let Some(stop) = missing_value_stop(&r, state) {
                            r.stop = Some(stop);
                            return r;
                        }
                        for arg in display {
                            record_display(&mut r, arg, false);
                        }
                    }
                    if !kept_flags.is_empty() {
                        r.kept.push(format!("-{kept_flags}"));
                    }
                    state = next_state;
                    i += 1;
                    continue;
                }
            }
            if let State::Opt(arg, count) = state {
                let count = count + 1;
                state = if count < values(arg).1 {
                    State::Opt(arg, count)
                } else {
                    State::Done
                };
                r.owners[i] = Some(Owner::new(&r.path, arg, 0));
                r.kept.push(token.clone());
                i += 1;
                continue;
            }
        }
        // A positional value.
        let Some(arg) = positional_at(node, pos_counter) else {
            if !trailing
                && !looks_like_option(token)
                && node.has_subcommands()
                && node.get_positionals().next().is_none()
            {
                r.stop = Some(Stop::Own(UsageError::UnknownCommand {
                    path: r.path.clone(),
                    word: token.clone(),
                    suggestion: help::closest_subcommand_suggestion(token, node)
                        .map(str::to_string),
                }));
                return r;
            }
            r.kept.push(token.clone());
            return stop_at_end(r);
        };
        if arg.is_trailing_var_arg_set() {
            trailing = true;
        }
        if r.path.is_empty() && r.root_operand.is_none() {
            r.root_operand = Some(token.clone());
        }
        arg_given = true;
        r.owners[i] = Some(Owner::new(&r.path, arg, 0));
        state = if is_multiple(arg) {
            State::Pos(arg)
        } else {
            pos_counter += 1;
            State::Done
        };
        r.kept.push(token.clone());
        i += 1;
    }
    r
}

fn is_display(arg: &clap::Arg) -> bool {
    is_help(arg) || matches!(arg.get_action(), ArgAction::Version)
}

/// Clap rejects the kept tokens so far, ending at the last one.
fn stop_at_end(mut r: Recognized) -> Recognized {
    r.stop = Some(Stop::Clap {
        prefix: r.kept.len(),
    });
    r
}

/// A pending option still lacking a required value when `--profile` or a
/// display request arrives: clap rejects the missing value at that token, and
/// the kept prefix reproduces it.
fn missing_value_stop(r: &Recognized, state: State<'_>) -> Option<Stop> {
    let State::Opt(arg, count) = state else {
        return None;
    };
    (count < values(arg).0).then(|| Stop::Clap {
        prefix: r.kept.len(),
    })
}

fn record_display(r: &mut Recognized, arg: &clap::Arg, long_token: bool) {
    if matches!(arg.get_action(), ArgAction::Version) {
        r.version = true;
        return;
    }
    if r.help.is_none() {
        let long = match arg.get_action() {
            ArgAction::HelpLong => true,
            ArgAction::HelpShort => false,
            _ => long_token,
        };
        r.help = Some((r.path.clone(), long));
    }
}

/// `help <path>` at `node`: the remaining words name the path below `node`.
/// Plexi's `--profile` and the node's global options may follow the path.
fn recognize_help_subcommand(r: &mut Recognized, node: &Command, rest: &[String], at: usize) {
    let mut target = node;
    let mut path = r.path.clone();
    let mut j = 0;
    while j < rest.len() {
        let token = &rest[j];
        if let Some(long) = token.strip_prefix("--").filter(|long| !long.is_empty()) {
            let (name, inline) = match long.split_once('=') {
                Some((name, value)) => (name, Some(value)),
                None => (long, None),
            };
            if name == PROFILE_LONG {
                match inline {
                    Some(value) => r.profiles.push(Some(value.to_string())),
                    None => match rest.get(j + 1).filter(|v| !looks_like_option(v)) {
                        Some(value) => {
                            r.profiles.push(Some(value.clone()));
                            j += 1;
                        }
                        None => r.profiles.push(None),
                    },
                }
                j += 1;
                continue;
            }
            if let Some(arg) = find_long(node, name).filter(|a| a.is_global_set()) {
                r.kept.push(token.clone());
                if takes_values(arg) && inline.is_none() {
                    if let Some(value) = rest.get(j + 1) {
                        r.kept.push(value.clone());
                        j += 1;
                    }
                }
                j += 1;
                continue;
            }
            if inline.is_none() && find_long(node, name).is_some_and(is_help) {
                j += 1;
                continue;
            }
        }
        let short_help = token
            .strip_prefix('-')
            .filter(|flags| flags.chars().count() == 1)
            .and_then(|flags| flags.chars().next())
            .and_then(|c| find_short(node, c))
            .is_some_and(is_help);
        if short_help {
            j += 1;
            continue;
        }
        if looks_like_option(token) {
            // Clap diagnoses the option at the node `help` was given at.
            r.kept.push(token.clone());
            r.stop = Some(Stop::Clap {
                prefix: r.kept.len(),
            });
            return;
        }
        // `help` after a path names clap's generated help command: the path
        // already says what to describe.
        if token == "help" && target.has_subcommands() {
            j += 1;
            continue;
        }
        match target.find_subcommand(token) {
            Some(sub) => {
                path.push(sub.get_name().to_string());
                target = sub;
            }
            None => {
                r.stop = Some(Stop::Own(UsageError::UnknownCommand {
                    path,
                    word: token.clone(),
                    suggestion: help::closest_subcommand_suggestion(token, target)
                        .map(str::to_string),
                }));
                return;
            }
        }
        j += 1;
    }
    log::info!("cli_front: help subcommand at argv[{at}] for {path:?}");
    r.help_subcommand = Some((path, true));
}

// --- Rendering ---------------------------------------------------------------

/// Print help for `help.path` from the public tree to stdout. Exit 0.
pub fn emit_help(bin: &str, help: &Help) -> i32 {
    use std::io::Write;
    let text = if help.path.is_empty() {
        crate::cli::help::grouped_help(&grammar(bin), bin, std::env::var_os("NO_COLOR").is_some())
    } else {
        format!("{}", render_node_help(bin, &help.path, help.long).ansi())
    };
    let mut out = anstream::stdout();
    if let Err(error) = out.write_all(text.as_bytes()).and_then(|()| out.flush()) {
        log::warn!("cli_front: help output failed: {error}");
    }
    0
}

/// Help for a canonical command path, laid out as clap's own help flag lays it
/// out: the long layout only when the command has long help to show.
fn render_node_help(bin: &str, path: &[String], long: bool) -> clap::builder::StyledStr {
    let mut cmd = help::with_run_paths(grammar(bin));
    cmd.build();
    fn render(node: &mut Command, path: &[String], long: bool) -> clap::builder::StyledStr {
        match path.split_first() {
            Some((name, rest)) if node.find_subcommand(name).is_some() => {
                let sub = node
                    .find_subcommand_mut(name)
                    .expect("find_subcommand just found it");
                render(sub, rest, long)
            }
            _ if long && long_help_exists(node) => node.render_long_help(),
            _ => node.render_help(),
        }
    }
    render(&mut cmd, path, long)
}

/// Clap 4.6.7's `Command::long_help_exists` (crate-private): whether `--help`
/// shows more than `-h`. `help_matches_clap_native_rendering` holds the two
/// equal on every path.
fn long_help_exists(cmd: &Command) -> bool {
    let should_long = |arg: &clap::Arg| {
        !arg.is_hide_set()
            && (arg.get_long_help().is_some()
                || arg.is_hide_long_help_set()
                || arg.is_hide_short_help_set()
                || (!arg.is_hide_possible_values_set()
                    && arg
                        .get_possible_values()
                        .iter()
                        .any(|value| !value.is_hide_set() && value.get_help().is_some())))
    };
    cmd.get_long_about().is_some()
        || cmd.get_before_long_help().is_some()
        || cmd.get_after_long_help().is_some()
        || cmd.get_arguments().any(should_long)
}

pub fn emit_version(bin: &str) -> i32 {
    print!("{}", grammar(bin).render_version());
    0
}

/// Print a usage error to stderr. Exit 2, with nothing on stdout.
pub fn emit_usage_error(bin: &str, error: &UsageError) -> i32 {
    use std::io::Write;
    log::info!("cli_front: usage error: {}", usage_error_text(bin, error));
    if let UsageError::Grammar { error, note } = error {
        if let Err(print_error) = error.print() {
            log::warn!("cli_front: error output failed: {print_error}");
        }
        if let Some(note) = note {
            eprint!("{note}");
        }
        return 2;
    }
    let text = match error {
        UsageError::MissingCommand { path } => {
            format!("{}", render_node_help(bin, path, false).ansi())
        }
        other => usage_error_text(bin, other),
    };
    let mut err = anstream::stderr();
    if let Err(print_error) = err.write_all(text.as_bytes()).and_then(|()| err.flush()) {
        log::warn!("cli_front: error output failed: {print_error}");
    }
    2
}

/// The plain text of a usage error.
pub fn usage_error_text(bin: &str, error: &UsageError) -> String {
    let try_help = |path: &[String]| {
        let command = std::iter::once(bin.to_string())
            .chain(path.iter().cloned())
            .collect::<Vec<_>>()
            .join(" ");
        format!("For more information, try '{command} --help'.\n")
    };
    match error {
        UsageError::Profile(message) => format!("error: {message}\n\n{}", try_help(&[])),
        UsageError::Retired(message) => format!("error: {message}\n"),
        UsageError::UnknownCommand {
            path,
            word,
            suggestion,
        } => {
            let command = std::iter::once(bin.to_string())
                .chain(path.iter().cloned())
                .collect::<Vec<_>>()
                .join(" ");
            let headline = match suggestion {
                Some(matched) => format!("error: unknown command '{word}'. Did you mean `{matched}`?"),
                None => format!("error: unknown command '{word}'"),
            };
            format!(
                "{headline}\n\nUsage: {command} [OPTIONS] [COMMAND]\n\n{}",
                try_help(path)
            )
        }
        UsageError::MissingCommand { path } => render_node_help(bin, path, false).to_string(),
        UsageError::Rebound { argument } => format!(
            "error: with `--profile` or a help or version request removed, a value would move to or from '{argument}'; give the value with '=' or move the flag\n\n{}",
            try_help(&[])
        ),
        UsageError::NotUtf8 { index } => {
            format!("error: argument {index} is not valid UTF-8\n\n{}", try_help(&[]))
        }
        UsageError::Grammar { error, note } => format!(
            "{}{}",
            error.render(),
            note.as_deref().unwrap_or_default()
        ),
        UsageError::Internal(message) => format!("error: {message}\n"),
    }
}

#[cfg(test)]
#[path = "front_tests.rs"]
mod tests;

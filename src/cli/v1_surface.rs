//! Coverage rule for the public CLI surface.
//!
//! Every command path visible with all experiments off must be listed in
//! `v1_surface.txt` (the approved v1 inventory made machine readable) or it is
//! drift. A path owned by an experiment is hidden when the experiment is off,
//! so it never needs an entry. See `docs/2026-09-20-cli-v1-inventory.md`.

use super::args::Cli;
use crate::release::{GatePath, ReleaseFeature};
use clap::CommandFactory;
use std::collections::BTreeSet;

const ALLOWLIST: &str = include_str!("v1_surface.txt");

/// A parsed allowlist: exact paths and `path *` subtrees.
struct Allowlist {
    exact: BTreeSet<String>,
    subtrees: BTreeSet<String>,
}

impl Allowlist {
    fn parse(text: &str) -> Self {
        let mut exact = BTreeSet::new();
        let mut subtrees = BTreeSet::new();
        for line in text.lines() {
            let entry = line.split('#').next().unwrap_or("").trim();
            if entry.is_empty() {
                continue;
            }
            match entry.strip_suffix(" *") {
                Some(path) => subtrees.insert(path.to_string()),
                None => exact.insert(entry.to_string()),
            };
        }
        Self { exact, subtrees }
    }

    fn allows(&self, path: &str) -> bool {
        self.exact.contains(path)
            || self
                .subtrees
                .iter()
                .any(|root| path == root || path.starts_with(&format!("{root} ")))
    }
}

fn visible_paths(command: &clap::Command, parent: &str, out: &mut BTreeSet<String>) {
    for sub in command
        .get_subcommands()
        .filter(|sub| !sub.is_hide_set() && sub.get_name() != "help")
    {
        let path = format!("{parent} {}", sub.get_name()).trim().to_string();
        out.insert(path.clone());
        visible_paths(sub, &path, out);
    }
}

fn tree(enabled: impl Fn(ReleaseFeature) -> bool) -> clap::Command {
    let mut cmd = super::help::gate_command(Cli::command(), enabled);
    cmd.build();
    cmd
}

fn visible(enabled: impl Fn(ReleaseFeature) -> bool) -> BTreeSet<String> {
    let mut paths = BTreeSet::new();
    visible_paths(&tree(enabled), "", &mut paths);
    paths
}

fn drift(paths: &BTreeSet<String>, list: &Allowlist) -> Vec<String> {
    paths.iter().filter(|p| !list.allows(p)).cloned().collect()
}

#[test]
fn every_visible_command_is_v1_or_owned_by_an_experiment() {
    let list = Allowlist::parse(ALLOWLIST);
    let drifted = drift(&visible(|_| false), &list);
    assert!(
        drifted.is_empty(),
        "{} visible command path(s) are neither in src/cli/v1_surface.txt nor owned by an \
         experiment. Gate each in ReleaseFeature::owned_paths (src/release/catalog.rs) or, if the \
         approved inventory marks it v1, add it to the allowlist:\n{}",
        drifted.len(),
        drifted.join("\n")
    );
}

#[test]
fn every_allowlist_entry_is_a_visible_default_command() {
    let list = Allowlist::parse(ALLOWLIST);
    let visible = visible(|_| false);
    let stale: Vec<_> = list
        .exact
        .iter()
        .chain(&list.subtrees)
        .filter(|entry| !visible.contains(*entry))
        .collect();
    assert!(
        stale.is_empty(),
        "src/cli/v1_surface.txt lists paths that are not visible with experiments off (renamed, \
         removed, or owned by an experiment): {stale:?}"
    );
}

#[test]
fn an_untagged_sample_command_is_reported_as_drift() {
    let list = Allowlist::parse(ALLOWLIST);
    let cmd = Cli::command()
        .subcommand(clap::Command::new("zz-untagged").subcommand(clap::Command::new("child")))
        .mut_subcommand("agent", |agent| {
            agent.subcommand(clap::Command::new("zz-untagged-agent"))
        });
    let mut cmd = super::help::gate_command(cmd, |_| false);
    cmd.build();
    let mut paths = BTreeSet::new();
    visible_paths(&cmd, "", &mut paths);
    assert_eq!(
        drift(&paths, &list),
        [
            "agent zz-untagged-agent",
            "zz-untagged",
            "zz-untagged child"
        ]
    );
}

#[test]
fn an_experiment_owned_path_needs_no_allowlist_entry() {
    let list = Allowlist::parse(ALLOWLIST);
    for feature in ReleaseFeature::ALL {
        for path in feature.owned_paths() {
            if let GatePath::Sub(path) = path {
                let joined = path.join(" ");
                assert!(
                    !list.allows(&joined),
                    "{} owns `{joined}`, so the allowlist must not also cover it",
                    feature.key()
                );
            }
        }
    }
}

/// The help footer must advertise a command a new user can run on a default profile.
#[test]
fn help_footer_advertises_a_default_visible_command() {
    let footer = Cli::command()
        .get_after_help()
        .map(ToString::to_string)
        .expect("plexi has an after_help footer");
    let advertised = footer
        .strip_prefix("Get started: ")
        .and_then(|rest| rest.split(" | ").next())
        .unwrap_or_else(|| panic!("footer must start with `Get started: <command>`: {footer}"));
    let words: Vec<_> = advertised
        .split_whitespace()
        .skip(1)
        .take_while(|w| !w.starts_with('-') && !w.starts_with('<'))
        .collect();
    assert!(
        !words.is_empty(),
        "footer command has no subcommand: {advertised}"
    );
    let path = words.join(" ");
    let visible = visible(|_| false);
    assert!(
        visible.contains(&path),
        "footer advertises `{advertised}` but `{path}` is not visible with experiments off"
    );
}

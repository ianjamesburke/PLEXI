//! Experiment flags for everything that is not part of the stable v1 surface.
//!
//! Every [`ReleaseFeature`] is an experiment: off by default on every channel,
//! including alpha, and turned on one at a time through the `[experiments]`
//! table of the global `config.toml` (`plexi experiments enable <name>`).
//! Channels decide update cadence and profile directory, never features.
//!
//! One table drives everything. [`ReleaseFeature::owned_paths`] lists the clap
//! paths an experiment owns, and `cli::help::gate_command` hides them from help
//! and completions, `exit_if_feature_disabled` refuses them at execution time,
//! and the channel skill strips the sections tagged with the experiment.
//!
//! Experiment data rule: an experiment owns its storage in its own versioned
//! location and never writes core state (config, layout, grants). There is no
//! migration code for experiments: on a format change, move the old data aside
//! and start fresh with a notice. Graduation to stable freezes the format.
//!
//! Experiment gate rule: a gate guards only the experiment's own surface,
//! never a shared servicing path that also does core work. Core app I/O must
//! run with every experiment off, including during hidden-window logic passes.
//!
//! The enabled set is read from the *global* config only. A workspace config
//! must not be able to turn an experiment on, or a cloned repository could
//! enable the relay by shipping a `.plexi/config.toml`.

use std::collections::BTreeSet;
use std::sync::OnceLock;

/// A clap path owned by an experiment.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GatePath {
    /// The subcommand at this path, and everything below it.
    Sub(&'static [&'static str]),
    /// The argument with this id on the subcommand at this path.
    Arg(&'static [&'static str], &'static str),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum ReleaseFeature {
    Assistant,
    AiSetup,
    Leads,
    CommandView,
    ChangeSets,
    Ledger,
    Relay,
    CloudAccount,
    FolderSecrets,
    WasmApps,
    PageGrid,
    Marketplace,
    Daw,
    MediaIo,
    Accessibility,
    McpClient,
    Routines,
}

impl ReleaseFeature {
    pub const ALL: &'static [ReleaseFeature] = &[
        Self::Assistant,
        Self::AiSetup,
        Self::Leads,
        Self::CommandView,
        Self::ChangeSets,
        Self::Ledger,
        Self::Relay,
        Self::CloudAccount,
        Self::FolderSecrets,
        Self::WasmApps,
        Self::PageGrid,
        Self::Marketplace,
        Self::Daw,
        Self::MediaIo,
        Self::Accessibility,
        Self::McpClient,
        Self::Routines,
    ];

    /// The `[experiments]` config key and the name used by `plexi experiments`.
    pub fn key(self) -> &'static str {
        match self {
            Self::Assistant => "assistant",
            Self::AiSetup => "ai_setup",
            Self::Leads => "leads",
            Self::CommandView => "command_view",
            Self::ChangeSets => "change_sets",
            Self::Ledger => "ledger",
            Self::Relay => "relay",
            Self::CloudAccount => "cloud_account",
            Self::FolderSecrets => "folder_secrets",
            Self::WasmApps => "wasm_apps",
            Self::PageGrid => "page_grid",
            Self::Marketplace => "marketplace",
            Self::Daw => "daw",
            Self::MediaIo => "media_io",
            Self::Accessibility => "accessibility",
            Self::McpClient => "mcp_client",
            Self::Routines => "routines",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|f| f.key() == key)
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Assistant => "Host AI assistant pane and the `assistant` commands",
            Self::AiSetup => "Guided AI backend setup (`ai setup`, `ai onboard`)",
            Self::Leads => "Named agent leads, runs, delegation and the assignment queue",
            Self::CommandView => "Leads dashboard pane and `command-view`",
            Self::ChangeSets => "Preview and accept agent edits (`changes`)",
            Self::Ledger => "Token and cost summary (`ledger`)",
            Self::Relay => "Phone pairing and turns through the Plexi relay",
            Self::CloudAccount => "Optional cloud account and history retention",
            Self::FolderSecrets => "Secrets injected per folder, with grants",
            Self::WasmApps => "Native Rust/WASM apps and the GPU canvas",
            Self::PageGrid => "2D page grid per context and the minimap",
            Self::Marketplace => "App marketplace: browse, search, publish",
            Self::Daw => "DAW engine proof of concept",
            Self::MediaIo => "Audio and video player panes",
            Self::Accessibility => "AccessKit accessibility tree",
            Self::McpClient => "MCP client: host MCP config and `app open --mcp`",
            Self::Routines => "Scheduled commands (`routine`)",
        }
    }

    /// True when the host reads this experiment at runtime, so a change only
    /// takes effect after `plexi host restart`. Pure CLI gates re-read the
    /// config on every invocation.
    pub fn requires_restart(self) -> bool {
        !matches!(self, Self::AiSetup | Self::Leads | Self::Ledger)
    }

    /// The clap paths this experiment owns. Empty for runtime-only surfaces
    /// that have no command of their own.
    pub fn owned_paths(self) -> &'static [GatePath] {
        use GatePath::{Arg, Sub};
        match self {
            Self::Assistant => &[Sub(&["assistant"])],
            Self::AiSetup => &[Sub(&["ai", "setup"]), Sub(&["ai", "onboard"])],
            Self::Leads => &[
                Sub(&["agent", "init"]),
                Sub(&["agent", "add"]),
                Sub(&["agent", "update"]),
                Sub(&["agent", "list"]),
                Sub(&["agent", "head"]),
                Sub(&["agent", "run"]),
                Sub(&["agent", "assign"]),
                Sub(&["agent", "cancel"]),
                Sub(&["agent", "conversation"]),
                Sub(&["agent", "delegate"]),
            ],
            Self::CommandView => &[Sub(&["command-view"])],
            Self::ChangeSets => &[Sub(&["changes"])],
            Self::Ledger => &[Sub(&["ledger"])],
            Self::Relay => &[Sub(&["relay"])],
            Self::CloudAccount => &[Sub(&["account"])],
            Self::FolderSecrets => &[
                Sub(&["secret", "rm"]),
                Sub(&["secret", "grant"]),
                Sub(&["secret", "read"]),
                Sub(&["secret", "exec"]),
                Arg(&["secret", "set"], "folder"),
            ],
            Self::Marketplace => &[
                Sub(&["app", "publish"]),
                Sub(&["app", "browse"]),
                Sub(&["app", "search"]),
            ],
            Self::McpClient => &[Sub(&["events", "mcp-config"]), Arg(&["app", "open"], "mcp")],
            Self::Routines => &[Sub(&["routine"])],
            Self::WasmApps | Self::PageGrid | Self::Daw | Self::MediaIo | Self::Accessibility => {
                &[]
            }
        }
    }
}

/// Parse the `[experiments]` table into the enabled set. An unknown name is an
/// error naming every valid key.
pub fn parse_experiments(
    table: &std::collections::BTreeMap<String, bool>,
) -> Result<BTreeSet<ReleaseFeature>, String> {
    let mut enabled = BTreeSet::new();
    for (name, on) in table {
        let Some(feature) = ReleaseFeature::from_key(name) else {
            let valid: Vec<&str> = ReleaseFeature::ALL.iter().map(|f| f.key()).collect();
            return Err(format!(
                "unknown experiment `{name}` in [experiments]. Valid names: {}. Fix it in config.toml or run `plexi experiments disable {name}`.",
                valid.join(", ")
            ));
        };
        if *on {
            enabled.insert(feature);
        }
    }
    Ok(enabled)
}

static EXPERIMENTS: OnceLock<Result<BTreeSet<ReleaseFeature>, String>> = OnceLock::new();

#[cfg(test)]
thread_local! {
    static TEST_EXPERIMENTS: std::cell::RefCell<Option<BTreeSet<ReleaseFeature>>> =
        const { std::cell::RefCell::new(None) };
}

/// RAII guard that clears the per-thread experiment override on drop.
#[cfg(test)]
pub struct TestExperimentsGuard;

#[cfg(test)]
impl Drop for TestExperimentsGuard {
    fn drop(&mut self) {
        TEST_EXPERIMENTS.with(|c| *c.borrow_mut() = None);
    }
}

/// Override the enabled experiments for the current test thread. The override
/// is thread-local, so a gate checked on a worker thread will not see it.
#[cfg(test)]
pub fn set_test_experiments(enabled: &[ReleaseFeature]) -> TestExperimentsGuard {
    TEST_EXPERIMENTS.with(|c| *c.borrow_mut() = Some(enabled.iter().copied().collect()));
    TestExperimentsGuard
}

/// The enabled set from the global config, cached for the life of the process.
/// A missing config means nothing is enabled.
pub fn load_experiments() -> &'static Result<BTreeSet<ReleaseFeature>, String> {
    EXPERIMENTS.get_or_init(crate::config::load_global_experiments)
}

/// An experiment-config error to surface before running a command, if any.
pub fn experiments_error() -> Option<&'static str> {
    load_experiments().as_ref().err().map(String::as_str)
}

pub fn feature_enabled(feature: ReleaseFeature) -> bool {
    #[cfg(test)]
    if let Some(set) = TEST_EXPERIMENTS.with(|c| c.borrow().clone()) {
        return set.contains(&feature);
    }
    // Tests never touch the process-wide cache or a real profile: an isolated
    // profile dir is read fresh, anything else means everything is off.
    #[cfg(test)]
    {
        return crate::config::test_profile_dir_override().is_some()
            && crate::config::load_global_experiments().is_ok_and(|s| s.contains(&feature));
    }
    #[cfg(not(test))]
    load_experiments()
        .as_ref()
        .is_ok_and(|set| set.contains(&feature))
}

pub fn log_feature_blocked(feature: ReleaseFeature) {
    log::info!(
        "release_gate: blocked access to experiment {}; it is off",
        feature.key()
    );
}

pub fn feature_unavailable_message(feature: ReleaseFeature) -> String {
    log_feature_blocked(feature);
    feature_unavailable_text(feature)
}

/// The user-facing unavailable text without the block-log side effect, for
/// static surfaces such as generated help.
pub fn feature_unavailable_text(feature: ReleaseFeature) -> String {
    format!(
        "{key} is an experiment. Turn it on with: plexi experiments enable {key}",
        key = feature.key()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    const CHANNELS: [Option<&str>; 6] = [
        None,
        Some("main"),
        Some("rc-010"),
        Some("beta"),
        Some("alpha"),
        Some("pr-2259"),
    ];

    #[test]
    fn every_experiment_is_off_by_default_on_every_channel() {
        let profile = tempfile::tempdir().unwrap();
        let _profile = crate::config::set_test_profile_dir(profile.path().to_path_buf());
        for channel in CHANNELS {
            let _channel = channel.map(crate::config::set_test_channel);
            for feature in ReleaseFeature::ALL {
                assert!(
                    !feature_enabled(*feature),
                    "{} must be off on {channel:?}",
                    feature.key()
                );
            }
        }
    }

    #[test]
    fn config_enable_turns_a_feature_on_for_any_channel() {
        let profile = tempfile::tempdir().unwrap();
        let _profile = crate::config::set_test_profile_dir(profile.path().to_path_buf());
        std::fs::write(
            profile.path().join("config.toml"),
            "[experiments]\nassistant = true\nrelay = false\n",
        )
        .unwrap();
        for channel in CHANNELS {
            let _channel = channel.map(crate::config::set_test_channel);
            assert!(feature_enabled(ReleaseFeature::Assistant), "{channel:?}");
            assert!(!feature_enabled(ReleaseFeature::Relay), "{channel:?}");
            assert!(!feature_enabled(ReleaseFeature::Ledger), "{channel:?}");
        }
    }

    #[test]
    fn unknown_experiment_name_errors_with_the_valid_list() {
        let table = BTreeMap::from([("assistent".to_string(), true)]);
        let err = parse_experiments(&table).unwrap_err();
        assert!(err.contains("unknown experiment `assistent`"), "{err}");
        for feature in ReleaseFeature::ALL {
            assert!(err.contains(feature.key()), "{err}");
        }
    }

    #[test]
    fn keys_are_unique_and_round_trip() {
        let mut seen = BTreeSet::new();
        for feature in ReleaseFeature::ALL {
            assert!(
                seen.insert(feature.key()),
                "duplicate key {}",
                feature.key()
            );
            assert_eq!(ReleaseFeature::from_key(feature.key()), Some(*feature));
            assert!(!feature.description().is_empty());
        }
    }

    #[test]
    fn unavailable_text_names_the_enable_command() {
        assert_eq!(
            feature_unavailable_text(ReleaseFeature::CommandView),
            "command_view is an experiment. Turn it on with: plexi experiments enable command_view"
        );
    }

    #[test]
    fn workspace_config_cannot_enable_experiments() {
        let profile = tempfile::tempdir().unwrap();
        let _profile = crate::config::set_test_profile_dir(profile.path().to_path_buf());
        let workspace = tempfile::tempdir().unwrap();
        let ws_cfg = crate::config::workspace_config_path(workspace.path());
        std::fs::create_dir_all(ws_cfg.parent().unwrap()).unwrap();
        std::fs::write(&ws_cfg, "[experiments]\nrelay = true\n").unwrap();
        let _cfg = crate::config::PlexiConfig::load_with_workspace(Some(workspace.path()));
        assert!(!feature_enabled(ReleaseFeature::Relay));
    }
}

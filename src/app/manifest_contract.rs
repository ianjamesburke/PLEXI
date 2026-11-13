//! Schema-2 manifests declare plugin identity, runtime, views and command metadata.
//! Package install, discovery and `plugin check` use the same admission contract.
//! A rejection names the plugin, field and reason, and emits an info log.
//!
//! ## Capabilities
//!
//! `[plugin.capabilities].capabilities` is a list of capability IDs. The registry
//! below owns the accepted IDs. Unknown IDs fail before activation. Reserved IDs
//! parse as known reservations but fail with "reserved, not yet available" while
//! no host projection serves them. A declaration requests access; it grants none.
//!
//! ## Commands
//!
//! Each `[[contributes.commands]]` table requires `id` and `handler` strings.
//! `id` begins with the complete plugin ID followed by a dot and a nonempty suffix.
//! Dot-separated segments contain ASCII letters, digits, underscores or dashes.
//! IDs are unique within the manifest. Core CLI namespaces, including hidden
//! commands and aliases, are reserved even when an experiment is disabled.
//! The first installed plugin owns a command ID; a different plugin cannot take
//! it during install. Updating the same owner retains its claim. Discovery also
//! refuses conflicting contributions across plugin scopes.
//!
//! `handler` starts with an ASCII letter or underscore and continues with ASCII
//! letters, digits, underscores or dashes. It identifies a host-dispatched typed
//! handler. Shell expressions, executable paths and arguments are rejected.
//! Declarations are retained by install and reported by `plugin check`; dispatch
//! and CLI namespace consent are unavailable in this schema reservation.
//!
//! ## Later contributions
//!
//! Contributions other than `views` and `commands`, including panels, settings,
//! subscriptions, undo, themes, agents, skills and file handlers, fail with a
//! named "not yet supported" error before activation. Unknown fields inside a
//! view or command declaration also fail validation.

use super::{Contributions, PluginManifest, PluginManifestWire};
use crate::app::permissions::{capability_ids, CapabilityId};
use clap::CommandFactory;
use std::collections::HashSet;
use std::path::Path;
use std::sync::LazyLock;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("plugin '{plugin}' field '{field}': {kind}")]
pub struct ManifestError {
    pub plugin: String,
    pub field: String,
    pub kind: ManifestErrorKind,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ManifestErrorKind {
    #[error("unknown capability ID '{0}'")]
    UnknownCapability(String),
    #[error("capability ID '{0}' is reserved, not yet available (no host projection serves it)")]
    ReservedCapability(String),
    #[error("duplicate command ID '{0}'")]
    DuplicateCommand(String),
    #[error("command ID '{0}' must be inside the plugin namespace")]
    CommandNamespace(String),
    #[error("command ID '{0}' collides with a reserved core CLI namespace")]
    CoreCommandNamespace(String),
    #[error(
        "invalid command handler '{0}': expected a symbolic identifier, never a shell command line"
    )]
    InvalidCommandHandler(String),
    #[error("contribution '{0}' is not yet supported")]
    UnsupportedContribution(String),
    #[error("command ID '{id}' is already owned by plugin '{owner}' (first install wins)")]
    CommandOwned { id: String, owner: String },
    #[error("{0}")]
    InvalidField(String),
    #[error("manifest I/O failed: {0}")]
    Io(String),
}

impl ManifestError {
    pub(super) fn reject(plugin: &str, field: &str, kind: ManifestErrorKind) -> Self {
        let error = Self {
            plugin: plugin.into(),
            field: field.into(),
            kind,
        };
        log::info!(
            "manifest: rejected plugin={} field={} reason={}",
            error.plugin,
            error.field,
            error.kind
        );
        error
    }
}

pub(super) fn validate_capabilities(plugin: &str, ids: &[String]) -> Result<(), ManifestError> {
    for id in ids {
        let kind = match capability_ids().find(|(known, _)| *known == id) {
            Some((_, CapabilityId::Available)) => continue,
            Some((_, CapabilityId::Reserved)) => ManifestErrorKind::ReservedCapability(id.clone()),
            None => ManifestErrorKind::UnknownCapability(id.clone()),
        };
        return Err(ManifestError::reject(
            plugin,
            "plugin.capabilities.capabilities",
            kind,
        ));
    }
    Ok(())
}

// Includes hidden/experimental namespaces and aliases, independent of channel gates.
static CORE_NAMESPACES: LazyLock<HashSet<String>> = LazyLock::new(|| {
    let cli = crate::cli::args::Cli::command();
    cli.get_subcommands()
        .flat_map(|command| {
            std::iter::once(command.get_name())
                .chain(command.get_all_aliases())
                .map(str::to_owned)
        })
        .chain(["help".into(), "plexi".into()])
        .collect()
});

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
}

impl Contributions {
    pub(super) fn validate(&self, plugin: &str) -> Result<(), ManifestError> {
        if let Some(name) = self.unsupported.keys().next() {
            return Err(ManifestError::reject(
                plugin,
                &format!("contributes.{name}"),
                ManifestErrorKind::UnsupportedContribution(name.clone()),
            ));
        }
        let prefix = format!("{plugin}.");
        let mut seen = HashSet::new();
        for command in &self.commands {
            let root = command.id.split('.').next().unwrap_or_default();
            let kind = if CORE_NAMESPACES.contains(root) {
                Some(ManifestErrorKind::CoreCommandNamespace(command.id.clone()))
            } else if !command.id.starts_with(&prefix) || !command.id.split('.').all(identifier) {
                Some(ManifestErrorKind::CommandNamespace(command.id.clone()))
            } else if !seen.insert(&command.id) {
                Some(ManifestErrorKind::DuplicateCommand(command.id.clone()))
            } else {
                None
            };
            if let Some(kind) = kind {
                return Err(ManifestError::reject(
                    plugin,
                    "contributes.commands.id",
                    kind,
                ));
            }
            if !identifier(&command.handler)
                || !command
                    .handler
                    .starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
            {
                return Err(ManifestError::reject(
                    plugin,
                    "contributes.commands.handler",
                    ManifestErrorKind::InvalidCommandHandler(command.handler.clone()),
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn check_command_owner(
        &self,
        plugin: &str,
        owner: &str,
        existing: &Self,
    ) -> Result<(), ManifestError> {
        if owner == plugin {
            return Ok(());
        }
        for command in &self.commands {
            if existing.commands.iter().any(|prior| prior.id == command.id) {
                return Err(ManifestError::reject(
                    plugin,
                    "contributes.commands.id",
                    ManifestErrorKind::CommandOwned {
                        id: command.id.clone(),
                        owner: owner.into(),
                    },
                ));
            }
        }
        Ok(())
    }
}

impl PluginManifest {
    /// The single manifest parse path. Typed contract errors survive; a legacy
    /// `[app]` table is never accepted and the error only says what to rename.
    pub fn parse(raw: &str) -> Result<Self, ManifestError> {
        let value: toml::Value = toml::from_str(raw).map_err(|e| {
            ManifestError::reject(
                "<unknown>",
                "manifest",
                ManifestErrorKind::InvalidField(e.to_string()),
            )
        })?;
        let plugin = value
            .get("plugin")
            .and_then(|p| p.get("id"))
            .and_then(toml::Value::as_str)
            .unwrap_or("<unknown>")
            .to_owned();
        let legacy = value.get("app").is_some();
        // Name the schema the file declares; a schema 2 file can still carry
        // a leftover `[app]` table.
        let declared = match value.get("schema_version") {
            Some(version) => format!("this manifest declares schema_version = {version}"),
            None => "this manifest declares no schema_version".to_string(),
        };
        let wire: PluginManifestWire = value.try_into().map_err(|e: toml::de::Error| {
            // toml's message ends in a newline; keep the error one line.
            let e = e.to_string();
            let e = e.trim_end();
            let message = if legacy {
                format!(
                    "`[app]` manifests are no longer accepted ({declared}); rename `[app]` to `[plugin]`, move the entry file to `[runtime]`, declare views under `[[contributes.views]]`, and set schema_version = 2 (parse error: {e})"
                )
            } else {
                e.to_owned()
            };
            ManifestError::reject(&plugin, "manifest", ManifestErrorKind::InvalidField(message))
        })?;
        Self::try_from(wire)
    }

    /// Hold this lock through installation. Directory and git/pack installers
    /// share it, so simultaneous first installs cannot both claim the same ID.
    pub(crate) fn lock_command_install(
        &self,
        target: &Path,
    ) -> Result<Option<std::fs::File>, ManifestError> {
        if self.contributes.commands.is_empty() {
            return Ok(None);
        }
        let io = |error: std::io::Error| {
            ManifestError::reject(
                &self.plugin.id,
                "contributes.commands",
                ManifestErrorKind::Io(format!("{}: {error}", target.display())),
            )
        };
        std::fs::create_dir_all(target).map_err(&io)?;
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(target.join(".command-install.lock"))
            .map_err(&io)?;
        lock.lock().map_err(&io)?;
        for entry in std::fs::read_dir(target).map_err(&io)? {
            let entry = entry.map_err(&io)?;
            if entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            let path = entry.path().join("manifest.toml");
            if !path.is_file() {
                continue;
            }
            let raw = std::fs::read_to_string(&path).map_err(&io)?;
            // The registry skips a manifest that does not parse, so it owns
            // no commands; `plexi plugin list` reports it as broken.
            let existing = match Self::parse(&raw) {
                Ok(existing) => existing,
                Err(error) => {
                    log::warn!(
                        "plugin_install: ignoring unparseable manifest {} for command ownership: {error}",
                        path.display()
                    );
                    continue;
                }
            };
            self.contributes.check_command_owner(
                &self.plugin.id,
                &existing.plugin.id,
                &existing.contributes,
            )?;
        }
        Ok(Some(lock))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::package::{self, PackageError};
    use crate::cli::plugin::{plugin_install_with_pin, InstallConfirm};
    use crate::cli::plugin_check::plugin_check_cli;

    const GOOD: &str = include_str!("../../tests/fixtures/schema2/good.toml");

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/schema2")
                .join(format!("{name}.toml")),
        )
        .unwrap()
    }

    fn stage(raw: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("manifest.toml"), raw).unwrap();
        std::fs::write(
            dir.path().join("main.wasm"),
            include_bytes!("../../tests/wasm-fixtures/counter.wasm"),
        )
        .unwrap();
        dir
    }

    #[test]
    fn schema2_malformed_fixtures_have_distinct_typed_errors_at_every_boundary() {
        use ManifestErrorKind::*;
        let profile = tempfile::tempdir().unwrap();
        let _profile = crate::config::set_test_profile_dir(profile.path().to_owned());
        let cases = [
            (
                "unknown-capability",
                "plugin.capabilities.capabilities",
                UnknownCapability("fs.typo".into()),
            ),
            (
                "reserved-read",
                "plugin.capabilities.capabilities",
                ReservedCapability("usage.read".into()),
            ),
            (
                "reserved-refresh",
                "plugin.capabilities.capabilities",
                ReservedCapability("usage.refresh".into()),
            ),
            (
                "duplicate-command",
                "contributes.commands.id",
                DuplicateCommand("sample.open".into()),
            ),
            (
                "foreign-command",
                "contributes.commands.id",
                CommandNamespace("foreign.open".into()),
            ),
            (
                "core-command",
                "contributes.commands.id",
                CoreCommandNamespace("pane.open".into()),
            ),
            (
                "shell-handler",
                "contributes.commands.handler",
                InvalidCommandHandler("sh -c echo".into()),
            ),
            (
                "unsupported-panel",
                "contributes.panels",
                UnsupportedContribution("panels".into()),
            ),
        ];
        for (name, field, kind) in cases {
            let raw = fixture(name);
            let error = PluginManifest::parse(&raw).unwrap_err();
            assert_eq!(
                error,
                ManifestError {
                    plugin: "sample".into(),
                    field: field.into(),
                    kind
                },
                "{name}"
            );
            let dir = stage(&raw);
            let PackageError::ManifestContract(package_error) =
                package::validate_dir(dir.path()).unwrap_err()
            else {
                panic!("{name}: lost typed contract error")
            };
            assert_eq!(package_error, error);
            let registry = crate::app::registry::PluginRegistry::empty();
            assert!(registry
                .load_plugin(&dir.path().to_owned())
                .unwrap_err()
                .contains(&error.to_string()));
            assert_eq!(
                plugin_check_cli(dir.path().to_str().unwrap(), &[], None),
                1,
                "{name}"
            );
            for confirm in [InstallConfirm::Interactive, InstallConfirm::PreApproved] {
                assert_eq!(
                    plugin_install_with_pin(dir.path().to_str().unwrap(), None, confirm, true),
                    1,
                    "{name}"
                );
            }
            assert!(!crate::app::registry::plugins_dir().join("sample").exists());
        }
    }

    #[test]
    fn schema2_commands_round_trip_package_install_and_check() {
        let profile = tempfile::tempdir().unwrap();
        let _profile = crate::config::set_test_profile_dir(profile.path().to_owned());
        let dir = stage(GOOD);
        let archive = dir
            .path()
            .parent()
            .unwrap()
            .join(format!("schema2-{}.plexipkg", uuid::Uuid::new_v4()));
        package::build_package(dir.path(), Some(&archive)).unwrap();
        assert_eq!(package::validate_package(&archive).unwrap().id, "sample");
        assert_eq!(
            crate::cli::plugin::plugin_install_package(
                archive.to_str().unwrap(),
                None,
                InstallConfirm::PreApproved,
                true
            ),
            0
        );
        std::fs::remove_file(archive).unwrap();
        let installed = crate::app::registry::plugins_dir().join("sample");
        let raw = std::fs::read_to_string(installed.join("manifest.toml")).unwrap();
        assert_eq!(raw, GOOD);
        let manifest = PluginManifest::parse(&raw).unwrap();
        assert_eq!(manifest.contributes.commands[0].id, "sample.open");
        assert_eq!(manifest.contributes.commands[0].handler, "open_picker");
        assert_eq!(plugin_check_cli(installed.to_str().unwrap(), &[], None), 0);
    }

    #[test]
    fn schema2_first_installed_command_owner_survives_collision_and_update() {
        let profile = tempfile::tempdir().unwrap();
        let _profile = crate::config::set_test_profile_dir(profile.path().to_owned());
        let first_raw = GOOD.replace("sample.open", "sample.tools.open");
        let second_raw = GOOD
            .replace("\"sample\"", "\"sample.tools\"")
            .replace("sample.main", "sample.tools.main")
            .replace("sample.open", "sample.tools.open");
        let first = stage(&first_raw);
        let second = stage(&second_raw);
        assert_eq!(
            plugin_install_with_pin(
                first.path().to_str().unwrap(),
                None,
                InstallConfirm::PreApproved,
                true
            ),
            0
        );
        let candidate = PluginManifest::parse(&second_raw).unwrap();
        assert_eq!(
            candidate
                .lock_command_install(&crate::app::registry::plugins_dir())
                .unwrap_err()
                .kind,
            ManifestErrorKind::CommandOwned {
                id: "sample.tools.open".into(),
                owner: "sample".into()
            }
        );
        assert_eq!(
            plugin_install_with_pin(
                second.path().to_str().unwrap(),
                None,
                InstallConfirm::PreApproved,
                true
            ),
            1
        );
        assert!(!crate::app::registry::plugins_dir()
            .join("sample.tools")
            .exists());
        assert_eq!(
            std::fs::read_to_string(
                crate::app::registry::plugins_dir().join("sample/manifest.toml")
            )
            .unwrap(),
            first_raw
        );
        assert_eq!(
            plugin_install_with_pin(
                first.path().to_str().unwrap(),
                None,
                InstallConfirm::PreApproved,
                true
            ),
            0,
            "owner can update"
        );
    }

    #[test]
    fn schema2_concurrent_git_installs_keep_one_command_owner() {
        use crate::app::packs::SourceSpec;
        use crate::cli::install_host::{install_one, test_support::MockCloner};
        let target = tempfile::tempdir().unwrap();
        let first = GOOD
            .replace("main.wasm", "run.sh")
            .replace("sample.open", "sample.tools.open");
        let second = first
            .replace("id = \"sample\"", "id = \"sample.tools\"")
            .replace("sample.main", "sample.tools.main");
        let cloner = MockCloner::new()
            .with_manifest("first", &first)
            .with_manifest("second", &second);
        let barrier = std::sync::Barrier::new(2);
        std::thread::scope(|scope| {
            let handles: Vec<_> = ["first", "second"]
                .into_iter()
                .map(|url| {
                    let cloner = &cloner;
                    let barrier = &barrier;
                    let target = target.path();
                    scope.spawn(move || {
                        barrier.wait();
                        install_one(cloner, &SourceSpec::Git(url.into()), None, target)
                    })
                })
                .collect();
            let results: Vec<_> = handles
                .into_iter()
                .map(|handle| handle.join().unwrap())
                .collect();
            assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
            let error = results.into_iter().find_map(Result::err).unwrap();
            assert!(
                error.contains("sample.tools.open") && error.contains("first install wins"),
                "{error}"
            );
        });
        assert_ne!(
            target.path().join("sample").exists(),
            target.path().join("sample.tools").exists()
        );
    }

    #[test]
    fn schema2_git_install_rejects_invalid_capability_before_placing_plugin() {
        use crate::app::packs::SourceSpec;
        use crate::cli::install_host::{install_one, test_support::MockCloner};
        let target = tempfile::tempdir().unwrap();
        let cloner = MockCloner::new().with_manifest("bad", &fixture("reserved-read"));
        let error =
            install_one(&cloner, &SourceSpec::Git("bad".into()), None, target.path()).unwrap_err();
        assert!(
            error.contains("sample")
                && error.contains("usage.read")
                && error.contains("reserved, not yet available"),
            "{error}"
        );
        assert!(!target.path().join("sample").exists());
    }

    #[test]
    fn schema2_rejects_command_paths_empty_segments_and_shell_fields() {
        for id in [
            "sample.",
            "sample..open",
            "sample./open",
            "sample.open;quit",
            "sample.open now",
        ] {
            let raw = GOOD.replace("sample.open", id);
            assert!(
                matches!(
                    PluginManifest::parse(&raw).unwrap_err().kind,
                    ManifestErrorKind::CommandNamespace(_)
                ),
                "{id}"
            );
        }
        for handler in [
            "",
            "/bin/sh",
            "$(touch /tmp/probe)",
            "open;quit",
            "open now",
            "9invalid",
        ] {
            let raw = GOOD.replace("open_picker", handler);
            assert!(
                matches!(
                    PluginManifest::parse(&raw).unwrap_err().kind,
                    ManifestErrorKind::InvalidCommandHandler(_)
                ),
                "{handler}"
            );
        }
        assert!(matches!(
            PluginManifest::parse(&format!("{GOOD}shell = 'echo nope'\n"))
                .unwrap_err()
                .kind,
            ManifestErrorKind::InvalidField(_)
        ));
    }

    #[test]
    fn schema2_reserves_all_core_namespaces_including_aliases() {
        for namespace in CORE_NAMESPACES.iter() {
            let raw = GOOD.replace("sample.open", &format!("{namespace}.open"));
            assert!(
                matches!(
                    PluginManifest::parse(&raw).unwrap_err().kind,
                    ManifestErrorKind::CoreCommandNamespace(_)
                ),
                "{namespace}"
            );
        }
    }

    /// Stint 1260: a sibling whose manifest no longer parses is skipped by
    /// the registry and owns no commands, so it must not block an unrelated
    /// install.
    #[test]
    fn command_install_lock_skips_a_sibling_whose_manifest_does_not_parse() {
        let target = tempfile::tempdir().unwrap();
        let broken = target.path().join("broken");
        std::fs::create_dir_all(&broken).unwrap();
        std::fs::write(
            broken.join("manifest.toml"),
            "schema_version = 1\n[app]\nid = 'broken'\nname = 'Broken'\n",
        )
        .unwrap();
        let candidate = PluginManifest::parse(GOOD).unwrap();
        let lock = candidate
            .lock_command_install(target.path())
            .expect("a broken sibling must not block the install");
        assert!(
            lock.is_some(),
            "a plugin with commands holds the install lock"
        );
    }

    #[test]
    fn schema2_names_every_later_contribution() {
        for name in [
            "panels",
            "settings",
            "subscriptions",
            "undo",
            "themes",
            "agents",
            "skills",
            "file_handlers",
        ] {
            let raw = format!("{GOOD}\n[[contributes.{name}]]\nid = 'sample.later'\n");
            let error = PluginManifest::parse(&raw).unwrap_err();
            assert_eq!(
                error.kind,
                ManifestErrorKind::UnsupportedContribution(name.into())
            );
            assert_eq!(error.field, format!("contributes.{name}"));
        }
    }

    #[test]
    #[ignore = "requires isolated process-global logger"]
    fn schema2_rejections_emit_info_with_plugin_field_and_reason() {
        let profile = tempfile::tempdir().unwrap();
        let _profile = crate::config::set_test_profile_dir(profile.path().to_owned());
        crate::platform::logging::init(log::LevelFilter::Info, 1, true);
        let names = [
            "unknown-capability",
            "reserved-read",
            "reserved-refresh",
            "duplicate-command",
            "foreign-command",
            "core-command",
            "shell-handler",
            "unsupported-panel",
        ];
        let errors: Vec<_> = names
            .iter()
            .map(|name| PluginManifest::parse(&fixture(name)).unwrap_err())
            .collect();
        log::logger().flush();
        let output = std::fs::read_to_string(profile.path().join("plexi.log")).unwrap();
        for error in errors {
            let expected = format!(
                "manifest: rejected plugin={} field={} reason={}",
                error.plugin, error.field, error.kind
            );
            assert!(
                output
                    .lines()
                    .any(|line| line.contains("[INFO]") && line.contains(&expected)),
                "missing info rejection: {expected}\n{output}"
            );
        }
        println!("{output}");
    }

    #[test]
    fn schema2_registry_preserves_runtime_ids_and_reservations_never_grant() {
        let mut seen = HashSet::new();
        for (id, availability) in capability_ids() {
            assert!(seen.insert(id), "duplicate registry ID {id}");
            match availability {
                CapabilityId::Available => {
                    crate::app::permissions::Capability::try_from(id).unwrap();
                    validate_capabilities("sample", &[id.into()]).unwrap();
                }
                CapabilityId::Reserved => {
                    assert!(
                        crate::app::permissions::parse_capability_strings(&[id.into()]).is_err()
                    );
                    assert!(matches!(
                        validate_capabilities("sample", &[id.into()])
                            .unwrap_err()
                            .kind,
                        ManifestErrorKind::ReservedCapability(_)
                    ));
                }
            }
        }
    }
}

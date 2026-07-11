//! Typed, layered settings for the host Assistant.

use std::path::{Path, PathBuf};

use crate::app_protocol::ModelTier;
use crate::broker::Decision;

pub fn model_tier_name(tier: ModelTier) -> &'static str {
    match tier {
        ModelTier::Low => "low",
        ModelTier::Medium => "medium",
        ModelTier::High => "high",
    }
}

/// One Assistant settings layer, in increasing precedence order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsScope {
    Default,
    User,
    Workspace,
    Local,
    Session,
}

impl std::fmt::Display for SettingsScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Default => "default",
            Self::User => "user",
            Self::Workspace => "workspace",
            Self::Local => "local",
            Self::Session => "session",
        })
    }
}

/// Where one resolved setting came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingSource {
    pub scope: SettingsScope,
    pub path: Option<PathBuf>,
}

impl SettingSource {
    fn defaults() -> Self {
        Self {
            scope: SettingsScope::Default,
            path: None,
        }
    }

    pub fn description(&self) -> String {
        match &self.path {
            Some(path) => format!("{} ({})", self.scope, path.display()),
            None => self.scope.to_string(),
        }
    }
}

/// A resolved value together with the scope that supplied it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sourced<T> {
    pub value: T,
    pub source: SettingSource,
}

/// Model settings after all active scopes have been applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelSettings {
    pub tier: Sourced<ModelTier>,
}

/// Tool ids enabled by the active settings scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolsSettings {
    pub enabled: Sourced<Vec<String>>,
}

/// Whether Assistant memory is enabled by the active settings scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemorySettings {
    pub enabled: Sourced<bool>,
}

/// Hook ids enabled by the active settings scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HooksSettings {
    pub enabled: Sourced<Vec<String>>,
}

/// Product-level default posture for Assistant permissions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssistantPermissionPosture {
    Review,
    Plan,
    Work,
    Locked,
}

impl AssistantPermissionPosture {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Review => "review",
            Self::Plan => "plan",
            Self::Work => "work",
            Self::Locked => "locked",
        }
    }

    fn broker_default(self) -> Decision {
        match self {
            Self::Review | Self::Plan | Self::Work => Decision::Ask,
            Self::Locked => Decision::Deny,
        }
    }
}

fn decision_precedence(decision: Decision) -> u8 {
    match decision {
        Decision::Allow => 0,
        Decision::Ask => 1,
        Decision::Deny => 2,
    }
}

/// One deduplicated permission rule after scope and decision merging.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPermissionRule {
    pub rule: String,
    pub decision: Decision,
    pub source: SettingSource,
}

/// Permission rules after all active scopes have been merged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionSettings {
    pub posture: Sourced<AssistantPermissionPosture>,
    pub rules: Vec<ResolvedPermissionRule>,
}

impl Default for PermissionSettings {
    fn default() -> Self {
        Self {
            posture: Sourced {
                value: AssistantPermissionPosture::Review,
                source: SettingSource::defaults(),
            },
            rules: Vec::new(),
        }
    }
}

impl PermissionSettings {
    pub fn broker_posture(&self) -> crate::broker::PermissionPosture {
        let mut posture = crate::broker::PermissionPosture {
            default_posture: self.posture.value.broker_default(),
            allow: Vec::new(),
            ask: Vec::new(),
            deny: Vec::new(),
        };
        for rule in &self.rules {
            match rule.decision {
                Decision::Allow => posture.allow.push(rule.rule.clone()),
                Decision::Ask => posture.ask.push(rule.rule.clone()),
                Decision::Deny => posture.deny.push(rule.rule.clone()),
            }
        }
        posture
    }

    fn merge(&mut self, overrides: &PermissionRuleOverrides, source: &SettingSource) {
        if let Some(default_posture) = overrides.default_posture {
            self.posture = Sourced {
                value: default_posture,
                source: source.clone(),
            };
        }
        self.merge_rules(&overrides.allow, Decision::Allow, source);
        self.merge_rules(&overrides.ask, Decision::Ask, source);
        self.merge_rules(&overrides.deny, Decision::Deny, source);
    }

    fn merge_rules(&mut self, rules: &[String], decision: Decision, source: &SettingSource) {
        for rule in rules {
            if let Some(existing) = self.rules.iter_mut().find(|entry| entry.rule == *rule) {
                if decision_precedence(decision) > decision_precedence(existing.decision) {
                    existing.decision = decision;
                    existing.source = source.clone();
                } else if decision == existing.decision {
                    existing.source = source.clone();
                }
            } else {
                self.rules.push(ResolvedPermissionRule {
                    rule: rule.clone(),
                    decision,
                    source: source.clone(),
                });
            }
        }
    }
}

/// Assistant settings after all active scopes have been applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssistantSettings {
    pub model: ModelSettings,
    pub tools: ToolsSettings,
    pub memory: MemorySettings,
    pub hooks: HooksSettings,
    pub permissions: PermissionSettings,
}

impl Default for AssistantSettings {
    fn default() -> Self {
        Self {
            model: ModelSettings {
                tier: Sourced {
                    value: ModelTier::Medium,
                    source: SettingSource::defaults(),
                },
            },
            tools: ToolsSettings {
                enabled: Sourced {
                    value: Vec::new(),
                    source: SettingSource::defaults(),
                },
            },
            memory: MemorySettings {
                enabled: Sourced {
                    value: false,
                    source: SettingSource::defaults(),
                },
            },
            hooks: HooksSettings {
                enabled: Sourced {
                    value: Vec::new(),
                    source: SettingSource::defaults(),
                },
            },
            permissions: PermissionSettings::default(),
        }
    }
}

/// Allow, ask, and deny rules contributed by one settings scope.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PermissionRuleOverrides {
    #[serde(default)]
    pub default_posture: Option<AssistantPermissionPosture>,
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub ask: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
}

impl PermissionRuleOverrides {
    fn is_empty(&self) -> bool {
        self.default_posture.is_none()
            && self.allow.is_empty()
            && self.ask.is_empty()
            && self.deny.is_empty()
    }
}

/// Temporary settings owned by one Assistant pane session.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionOverrides {
    pub model_tier: Option<ModelTier>,
    pub permissions: PermissionRuleOverrides,
}

/// One settings file that could not be read or parsed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SettingsLoadError {
    #[error("failed to read {scope} Assistant settings at {}: {cause}", path.display())]
    Read {
        scope: SettingsScope,
        path: PathBuf,
        cause: String,
    },
    #[error("failed to parse {scope} Assistant settings at {}: {cause}", path.display())]
    Parse {
        scope: SettingsScope,
        path: PathBuf,
        cause: String,
    },
}

/// Resolved settings plus every recoverable layer error encountered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsLoadReport {
    pub settings: AssistantSettings,
    pub errors: Vec<SettingsLoadError>,
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SettingsFile {
    #[serde(default)]
    model: ModelFile,
    #[serde(default)]
    tools: ToolsFile,
    #[serde(default)]
    memory: MemoryFile,
    #[serde(default)]
    hooks: HooksFile,
    #[serde(default)]
    permissions: PermissionRuleOverrides,
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelFile {
    tier: Option<ModelTier>,
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ToolsFile {
    enabled: Option<Vec<String>>,
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct MemoryFile {
    enabled: Option<bool>,
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct HooksFile {
    enabled: Option<Vec<String>>,
}

/// Resolves channel-scoped user, workspace, local, and session settings.
pub struct SettingsLoader {
    user_path: PathBuf,
    workspace_path: PathBuf,
    local_path: PathBuf,
}

impl SettingsLoader {
    pub fn new(profile_dir: &Path, workspace_root: &Path) -> Self {
        let workspace_agents_dir = workspace_root
            .join(crate::config::workspace_channel_dir())
            .join("agents");
        Self {
            user_path: profile_dir.join("agents/settings.toml"),
            workspace_path: workspace_agents_dir.join("settings.toml"),
            local_path: workspace_agents_dir.join("settings.local.toml"),
        }
    }

    pub fn load(&self, session: &SessionOverrides) -> SettingsLoadReport {
        let mut settings = AssistantSettings::default();
        let mut errors = Vec::new();
        Self::apply_scope(
            &mut settings,
            &mut errors,
            SettingsScope::User,
            &self.user_path,
        );
        Self::apply_scope(
            &mut settings,
            &mut errors,
            SettingsScope::Workspace,
            &self.workspace_path,
        );
        Self::apply_scope(
            &mut settings,
            &mut errors,
            SettingsScope::Local,
            &self.local_path,
        );
        if let Some(tier) = session.model_tier {
            settings.model.tier = Sourced {
                value: tier,
                source: SettingSource {
                    scope: SettingsScope::Session,
                    path: None,
                },
            };
            log::info!(
                "assistant settings: applied session model tier {}",
                model_tier_name(tier)
            );
        }
        if !session.permissions.is_empty() {
            settings.permissions.merge(
                &session.permissions,
                &SettingSource {
                    scope: SettingsScope::Session,
                    path: None,
                },
            );
            log::info!("assistant settings: applied session permission rules");
        }
        SettingsLoadReport { settings, errors }
    }

    fn apply_scope(
        settings: &mut AssistantSettings,
        errors: &mut Vec<SettingsLoadError>,
        scope: SettingsScope,
        path: &Path,
    ) {
        match Self::load_file(scope, path) {
            Ok(Some(file)) => {
                Self::apply_file(
                    settings,
                    file,
                    SettingSource {
                        scope,
                        path: Some(path.to_path_buf()),
                    },
                );
                log::info!(
                    "assistant settings: loaded {scope} scope from {}",
                    path.display()
                );
            }
            Ok(None) => {}
            Err(error) => {
                log::error!("assistant settings: {error}");
                errors.push(error);
            }
        }
    }

    fn load_file(
        scope: SettingsScope,
        path: &Path,
    ) -> Result<Option<SettingsFile>, SettingsLoadError> {
        let raw = match std::fs::read_to_string(path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(SettingsLoadError::Read {
                    scope,
                    path: path.to_path_buf(),
                    cause: error.to_string(),
                });
            }
        };
        toml::from_str(&raw)
            .map(Some)
            .map_err(|error| SettingsLoadError::Parse {
                scope,
                path: path.to_path_buf(),
                cause: error.to_string(),
            })
    }

    fn apply_file(settings: &mut AssistantSettings, file: SettingsFile, source: SettingSource) {
        if let Some(tier) = file.model.tier {
            settings.model.tier = Sourced {
                value: tier,
                source: source.clone(),
            };
        }
        if let Some(enabled) = file.tools.enabled {
            settings.tools.enabled = Sourced {
                value: enabled,
                source: source.clone(),
            };
        }
        if let Some(enabled) = file.memory.enabled {
            settings.memory.enabled = Sourced {
                value: enabled,
                source: source.clone(),
            };
        }
        if let Some(enabled) = file.hooks.enabled {
            settings.hooks.enabled = Sourced {
                value: enabled,
                source: source.clone(),
            };
        }
        settings.permissions.merge(&file.permissions, &source);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_settings(path: &std::path::Path, raw: &str) {
        std::fs::create_dir_all(path.parent().expect("settings path has a parent")).unwrap();
        std::fs::write(path, raw).unwrap();
    }

    #[test]
    fn defaults_use_medium_model_tier_with_default_source() {
        let settings = AssistantSettings::default();

        assert_eq!(
            settings.model.tier.value,
            crate::app_protocol::ModelTier::Medium
        );
        assert_eq!(settings.model.tier.source.scope, SettingsScope::Default);
        assert_eq!(settings.tools.enabled.value, Vec::<String>::new());
        assert_eq!(settings.tools.enabled.source.scope, SettingsScope::Default);
        assert!(!settings.memory.enabled.value);
        assert_eq!(settings.memory.enabled.source.scope, SettingsScope::Default);
        assert_eq!(settings.hooks.enabled.value, Vec::<String>::new());
        assert_eq!(settings.hooks.enabled.source.scope, SettingsScope::Default);
        assert_eq!(
            settings.permissions.posture,
            Sourced {
                value: AssistantPermissionPosture::Review,
                source: SettingSource {
                    scope: SettingsScope::Default,
                    path: None,
                },
            }
        );
    }

    #[test]
    fn default_permission_posture_is_review_and_maps_to_broker_ask() {
        let settings = AssistantSettings::default();

        assert_eq!(
            settings.permissions.posture.value,
            AssistantPermissionPosture::Review
        );
        assert_eq!(
            settings.permissions.broker_posture().default_posture,
            crate::broker::Decision::Ask
        );
    }

    #[test]
    fn permission_postures_map_to_unified_broker_defaults() {
        let cases = [
            (AssistantPermissionPosture::Review, Decision::Ask),
            (AssistantPermissionPosture::Plan, Decision::Ask),
            (AssistantPermissionPosture::Work, Decision::Ask),
            (AssistantPermissionPosture::Locked, Decision::Deny),
        ];

        for (value, expected) in cases {
            let permissions = PermissionSettings {
                posture: Sourced {
                    value,
                    source: SettingSource::defaults(),
                },
                rules: Vec::new(),
            };
            assert_eq!(permissions.broker_posture().default_posture, expected);
        }
    }

    #[test]
    fn user_model_tier_overrides_default_and_records_its_path() {
        let profile = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let loader = SettingsLoader::new(profile.path(), workspace.path());
        write_settings(&loader.user_path, "[model]\ntier = \"low\"\n");

        let report = loader.load(&SessionOverrides::default());

        assert_eq!(report.settings.model.tier.value, ModelTier::Low);
        assert_eq!(
            report.settings.model.tier.source,
            SettingSource {
                scope: SettingsScope::User,
                path: Some(loader.user_path.clone()),
            }
        );
    }

    #[test]
    fn workspace_model_tier_overrides_user_scope() {
        let profile = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let loader = SettingsLoader::new(profile.path(), workspace.path());
        let workspace_path = workspace
            .path()
            .join(crate::config::workspace_channel_dir())
            .join("agents/settings.toml");
        write_settings(&loader.user_path, "[model]\ntier = \"low\"\n");
        write_settings(&workspace_path, "[model]\ntier = \"high\"\n");

        let report = loader.load(&SessionOverrides::default());

        assert_eq!(
            report.settings.model.tier,
            Sourced {
                value: ModelTier::High,
                source: SettingSource {
                    scope: SettingsScope::Workspace,
                    path: Some(workspace_path),
                },
            }
        );
    }

    #[test]
    fn local_model_tier_overrides_workspace_scope() {
        let profile = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let loader = SettingsLoader::new(profile.path(), workspace.path());
        let agents_dir = workspace
            .path()
            .join(crate::config::workspace_channel_dir())
            .join("agents");
        let workspace_path = agents_dir.join("settings.toml");
        let local_path = agents_dir.join("settings.local.toml");
        write_settings(&workspace_path, "[model]\ntier = \"low\"\n");
        write_settings(&local_path, "[model]\ntier = \"high\"\n");

        let report = loader.load(&SessionOverrides::default());

        assert_eq!(
            report.settings.model.tier,
            Sourced {
                value: ModelTier::High,
                source: SettingSource {
                    scope: SettingsScope::Local,
                    path: Some(local_path),
                },
            }
        );
    }

    #[test]
    fn local_enabled_tools_replace_workspace_and_user_tools_with_their_source() {
        let profile = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let loader = SettingsLoader::new(profile.path(), workspace.path());
        write_settings(
            &loader.user_path,
            "[tools]\nenabled = [\"host.panes.read\"]\n",
        );
        write_settings(
            &loader.workspace_path,
            "[tools]\nenabled = [\"app.csv.describe\", \"app.csv.read\"]\n",
        );
        write_settings(
            &loader.local_path,
            "[tools]\nenabled = [\"host.files.read\"]\n",
        );

        let report = loader.load(&SessionOverrides::default());

        assert_eq!(
            report.settings.tools.enabled,
            Sourced {
                value: vec!["host.files.read".to_string()],
                source: SettingSource {
                    scope: SettingsScope::Local,
                    path: Some(loader.local_path.clone()),
                },
            }
        );
    }

    #[test]
    fn local_memory_setting_replaces_workspace_and_user_values_with_its_source() {
        let profile = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let loader = SettingsLoader::new(profile.path(), workspace.path());
        write_settings(&loader.user_path, "[memory]\nenabled = true\n");
        write_settings(&loader.workspace_path, "[memory]\nenabled = false\n");
        write_settings(&loader.local_path, "[memory]\nenabled = true\n");

        let report = loader.load(&SessionOverrides::default());

        assert_eq!(
            report.settings.memory.enabled,
            Sourced {
                value: true,
                source: SettingSource {
                    scope: SettingsScope::Local,
                    path: Some(loader.local_path.clone()),
                },
            }
        );
    }

    #[test]
    fn local_enabled_hooks_replace_workspace_and_user_hooks_with_their_source() {
        let profile = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let loader = SettingsLoader::new(profile.path(), workspace.path());
        write_settings(&loader.user_path, "[hooks]\nenabled = [\"turn.started\"]\n");
        write_settings(
            &loader.workspace_path,
            "[hooks]\nenabled = [\"turn.finished\"]\n",
        );
        write_settings(
            &loader.local_path,
            "[hooks]\nenabled = [\"permission.requested\"]\n",
        );

        let report = loader.load(&SessionOverrides::default());

        assert_eq!(
            report.settings.hooks.enabled,
            Sourced {
                value: vec!["permission.requested".to_string()],
                source: SettingSource {
                    scope: SettingsScope::Local,
                    path: Some(loader.local_path.clone()),
                },
            }
        );
    }

    #[test]
    fn session_model_tier_overrides_local_scope() {
        let profile = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let loader = SettingsLoader::new(profile.path(), workspace.path());
        write_settings(&loader.local_path, "[model]\ntier = \"low\"\n");
        let session = SessionOverrides {
            model_tier: Some(ModelTier::High),
            ..SessionOverrides::default()
        };

        let report = loader.load(&session);

        assert_eq!(
            report.settings.model.tier,
            Sourced {
                value: ModelTier::High,
                source: SettingSource {
                    scope: SettingsScope::Session,
                    path: None,
                },
            }
        );
    }

    #[test]
    fn invalid_toml_reports_its_scope_and_path_without_blocking_valid_layers() {
        let profile = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let loader = SettingsLoader::new(profile.path(), workspace.path());
        write_settings(&loader.user_path, "[model\ntier = nope");
        write_settings(&loader.workspace_path, "[model]\ntier = \"high\"\n");

        let report = loader.load(&SessionOverrides::default());

        assert_eq!(report.settings.model.tier.value, ModelTier::High);
        assert!(matches!(
            report.errors.as_slice(),
            [SettingsLoadError::Parse {
                scope: SettingsScope::User,
                path,
                ..
            }] if *path == loader.user_path
        ));
    }

    #[test]
    fn unsupported_settings_groups_are_reported_instead_of_silently_ignored() {
        let profile = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let loader = SettingsLoader::new(profile.path(), workspace.path());
        write_settings(&loader.user_path, "[quantum]\nenabled = true\n");

        let report = loader.load(&SessionOverrides::default());

        assert!(matches!(
            report.errors.as_slice(),
            [SettingsLoadError::Parse {
                scope: SettingsScope::User,
                path,
                ..
            }] if *path == loader.user_path
        ));
    }

    #[test]
    fn permission_rules_merge_across_scopes_and_dedupe() {
        let profile = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let loader = SettingsLoader::new(profile.path(), workspace.path());
        write_settings(
            &loader.user_path,
            "[permissions]\nallow = [\"host.panes.read\", \"app.csv.read\"]\n",
        );
        write_settings(
            &loader.workspace_path,
            "[permissions]\nallow = [\"app.csv.read\", \"app.csv.describe\"]\n",
        );

        let report = loader.load(&SessionOverrides::default());
        let rules: Vec<(&str, Decision, SettingsScope)> = report
            .settings
            .permissions
            .rules
            .iter()
            .map(|rule| (rule.rule.as_str(), rule.decision, rule.source.scope))
            .collect();

        assert_eq!(
            rules,
            vec![
                ("host.panes.read", Decision::Allow, SettingsScope::User,),
                ("app.csv.read", Decision::Allow, SettingsScope::Workspace,),
                (
                    "app.csv.describe",
                    Decision::Allow,
                    SettingsScope::Workspace,
                ),
            ]
        );
    }

    #[test]
    fn default_permission_posture_uses_normal_scope_precedence() {
        let profile = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let loader = SettingsLoader::new(profile.path(), workspace.path());
        write_settings(
            &loader.user_path,
            "[permissions]\ndefault_posture = \"locked\"\n",
        );
        write_settings(
            &loader.workspace_path,
            "[permissions]\ndefault_posture = \"work\"\n",
        );
        let session = SessionOverrides {
            permissions: PermissionRuleOverrides {
                default_posture: Some(AssistantPermissionPosture::Plan),
                ..PermissionRuleOverrides::default()
            },
            ..SessionOverrides::default()
        };

        let report = loader.load(&session);

        assert_eq!(
            report.settings.permissions.posture,
            Sourced {
                value: AssistantPermissionPosture::Plan,
                source: SettingSource {
                    scope: SettingsScope::Session,
                    path: None,
                },
            }
        );
    }

    #[test]
    fn permission_rules_enforce_deny_then_ask_precedence_across_scopes() {
        let profile = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let loader = SettingsLoader::new(profile.path(), workspace.path());
        write_settings(
            &loader.user_path,
            "[permissions]\ndeny = [\"host.secrets.read\"]\nask = [\"app.csv.write\"]\n",
        );
        write_settings(
            &loader.workspace_path,
            "[permissions]\nask = [\"host.secrets.read\"]\nallow = [\"app.csv.write\"]\n",
        );
        write_settings(
            &loader.local_path,
            "[permissions]\nallow = [\"host.secrets.read\", \"app.csv.write\"]\n",
        );
        let session = SessionOverrides {
            model_tier: None,
            permissions: PermissionRuleOverrides {
                allow: vec!["host.secrets.read".to_string(), "app.csv.write".to_string()],
                ..PermissionRuleOverrides::default()
            },
        };

        let report = loader.load(&session);
        let rules: Vec<(&str, Decision, SettingsScope)> = report
            .settings
            .permissions
            .rules
            .iter()
            .map(|rule| (rule.rule.as_str(), rule.decision, rule.source.scope))
            .collect();

        assert_eq!(
            rules,
            vec![
                ("app.csv.write", Decision::Ask, SettingsScope::User,),
                ("host.secrets.read", Decision::Deny, SettingsScope::User,),
            ]
        );
    }
}

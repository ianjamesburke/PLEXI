//! Freeze the host's selection before asking. Read values only after the claim.
use super::*;
use crate::cli::run::{resolve_command_target, ResolvedRunTarget};
use crate::protocol::{BundleDefinition, BundleValue, SecretInput};
use crate::workspace::secrets::{self, ResolveOutcome, WorkspaceConfig, WorkspaceSecrets};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub(super) struct Bundle {
    cwd: PathBuf,
    // A folder account shadows a workspace route with the same name.
    names: BTreeMap<String, Option<PathBuf>>,
    workspace: Option<(WorkspaceConfig, WorkspaceSecrets)>,
    definition: Option<BundleDefinition>,
    input: String,
    digest: String,
}

impl Bundle {
    pub(super) fn discover(
        cwd: PathBuf,
        command: Option<(String, Option<PathBuf>)>,
        pane_root: Option<&Path>,
    ) -> Result<Self, String> {
        let root = crate::cli::resolve_commands_workspace_root(&cwd);
        let (config, router) = match root.as_deref() {
            Some(path) => (WorkspaceConfig::load(path)?, WorkspaceSecrets::load(path)?),
            None => (None, None),
        };
        let mut names = BTreeMap::new();
        let mut input = json!({"scope":cwd});
        let definition = if let Some((id, requested_root)) = command {
            if requested_root != root
                || pane_root.is_some_and(|pane| root.as_deref().is_some_and(|r| r != pane))
            {
                return Err("Command workspace changed or is outside the pane context. Run from its workspace root.".into());
            }
            let Some(ResolvedRunTarget::TomlCommand {
                scope,
                run,
                steps,
                on_load,
                secrets: required,
            }) = resolve_command_target(root.as_deref(), &id)?
            else {
                return Err(format!("Command '{id}' has no secret-bearing definition. Check commands.toml and retry."));
            };
            if required.is_empty() {
                return Err(format!(
                    "Command '{id}' no longer requires secrets. Check commands.toml and retry."
                ));
            }
            let snapshot = json!([run, steps, on_load, required, scope]);
            input["definition"] = format!("{:x}", Sha256::digest(snapshot.to_string())).into();
            input["command"] = id.into();
            names.extend(required.into_iter().map(|name| (name, None)));
            Some(BundleDefinition {
                run,
                steps,
                on_load,
            })
        } else {
            if let Some(router) = &router {
                names.extend(
                    router
                        .terminal
                        .env
                        .inject
                        .iter()
                        .cloned()
                        .map(|name| (name, None)),
                );
            }
            for (name, folder) in secrets::folders_for_cwd(&cwd, pane_root, secrets::folder_store())
            {
                names.insert(name, Some(folder));
            }
            None
        };
        if let Some(name) = names
            .keys()
            .find(|name| !secrets::manager::valid_env_name(name))
        {
            return Err(format!("Invalid environment variable name '{name}'. Check the workspace secret configuration."));
        }
        input["names"] = json!(names.keys().collect::<Vec<_>>());
        let input = input.to_string();
        let digest = crate::broker::gate::bundle_digest(&input)?;
        Ok(Self {
            cwd,
            names,
            workspace: config.zip(router),
            definition,
            input,
            digest,
        })
    }

    fn fetch(&self) -> Result<(Vec<BundleValue>, Vec<String>), String> {
        let mut values = Vec::new();
        let mut missing = Vec::new();
        for (name, folder) in &self.names {
            let value = if let Some(folder) = folder {
                secrets::folder_store()
                    .get_interactive(&secrets::folder_account_name(folder, name))
                    .map_err(|e| e.to_string())?
            } else if let Some((config, router)) = &self.workspace {
                let consumer = if self.definition.is_some() {
                    crate::cli::PLUGIN_ID
                } else {
                    "terminal"
                };
                match secrets::resolve(&config.id, consumer, name, router, secrets::system_store())
                {
                    ResolveOutcome::Found(value) => Some(value),
                    ResolveOutcome::HardMissing { reason } => {
                        log::info!("secret.exec: missing {name}: {reason}");
                        None
                    }
                    ResolveOutcome::PromptUser => None,
                }
            } else {
                None
            };
            match value {
                Some(mut value) => values.push(BundleValue {
                    name: name.clone(),
                    value: SecretInput(std::mem::take(&mut *value)),
                }),
                None => missing.push(name.clone()),
            }
        }
        Ok((values, missing))
    }
}

impl PlexiApp {
    pub(in crate::app) fn serve_secret_bundle(
        &self,
        cwd: &Path,
        command: Option<(String, Option<PathBuf>)>,
        origin: &RequestOrigin,
        response_file: &str,
    ) {
        self.run_secret_request(
            "secret.exec",
            "environment",
            SecretAsk::Bundle {
                cwd: cwd.into(),
                command,
            },
            origin,
            response_file,
            |request, monitor| {
                if let Err(error) = request.read_and_release_bundle(&monitor) {
                    stream_reply(&request.reply_path, json!({"status":"error","error":error}));
                }
            },
        );
    }
}

impl SecretRequest {
    fn read_and_release_bundle(&self, monitor: &Arc<PermissionMonitor>) -> Result<(), String> {
        let SecretTarget::Bundle(bundle) = &self.target else {
            return Err("Missing environment bundle".into());
        };
        let principal = self.subject.principal();
        let call = uuid::Uuid::new_v4().to_string();
        let label = self.bundle_label();
        log::info!("secret.exec: request filed {label}");
        let Some(decision) = self.admit(
            monitor,
            AdmitRequest {
                call_id: &call,
                tool: "secret.exec",
                input_json: &bundle.input,
                principal: &principal,
                actor_scope: crate::broker::ActorScope::User,
                trust_origin: crate::broker::SECRET_SOCKET_ORIGIN,
                workspace_root: &bundle.cwd,
                context_id: 0,
                package_id: "",
                instance_id: 0,
                target_type: crate::broker::TargetType::Secret,
            },
        ) else {
            return Ok(());
        };
        let UseDecision::Allowed {
            approved,
            grant_id,
            fingerprint,
            resource_id,
        } = decision
        else {
            stream_reply(&self.reply_path, json!({"status":"denied"}));
            return Ok(());
        };
        let actor = principal.actor_id();
        let audit = format!(
            "{resource_id} names={}",
            bundle.names.keys().cloned().collect::<Vec<_>>().join(",")
        );
        monitor.note_use(actor, &call, &grant_id, &fingerprint, &audit, "")?;
        let result = bundle.fetch();
        let outcome = match &result {
            Ok((_, missing)) if missing.is_empty() => "ok",
            Ok(_) => "missing",
            Err(_) => "failed",
        };
        monitor.note_outcome(
            actor,
            &call,
            &grant_id,
            &fingerprint,
            &audit,
            "",
            outcome,
            "",
            "",
        )?;
        let (values, missing) = result?;
        if !missing.is_empty() {
            stream_reply(
                &self.reply_path,
                json!({"status":"missing","names":missing}),
            );
            return Ok(());
        }
        let target = format!("exec:{}", bundle.digest);
        let permit = match self.release_permit(monitor, &target, *approved) {
            Ok(permit) => permit,
            Err(error) => {
                log::info!("secret.exec: release withheld {label}: {error}");
                stream_reply(&self.reply_path, json!({"status":"denied"}));
                return Ok(());
            }
        };
        #[derive(serde::Serialize)]
        struct Reply<'a> {
            status: &'static str,
            values: Vec<BundleValue>,
            #[serde(flatten)]
            definition: Option<&'a BundleDefinition>,
        }
        release_secret(
            permit,
            &target,
            &self.reply_path,
            &format!("secret.exec {label}"),
            || Reply {
                status: "values",
                values,
                definition: bundle.definition.as_ref(),
            },
        );
        Ok(())
    }

    pub(super) fn bundle_label(&self) -> String {
        match &self.target {
            SecretTarget::Bundle(b) => format!(
                "caller={} scope={} names={} digest={}",
                self.subject.outside().unwrap_or("pane"),
                b.cwd.display(),
                b.names.len(),
                b.digest
            ),
            _ => String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_discovery_freezes_names_accounts_and_definition() {
        let dir = tempfile::tempdir().unwrap();
        let _profile = crate::config::set_test_profile_dir(dir.path().join("profile"));
        let ancestor = dir.path().canonicalize().unwrap();
        let cwd = ancestor.join("work");
        std::fs::create_dir(&cwd).unwrap();
        secrets::init_workspace(&cwd, &crate::config::workspace_channel_dir()).unwrap();
        secrets::write_terminal_env_inject(&cwd, "SHARED", true).unwrap();
        for (name, path, value) in [
            ("PARENT", &ancestor, "p"),
            ("SHARED", &ancestor, "a"),
            ("SHARED", &cwd, "nearest"),
        ] {
            secrets::set_folder_secret(secrets::folder_store(), name, path, value).unwrap();
        }
        let discover = |root| Bundle::discover(cwd.clone(), None, root).unwrap();
        let outside = discover(None);
        let pane = discover(Some(&cwd));
        assert_ne!(outside.digest, pane.digest);
        assert_eq!(pane.names.keys().collect::<Vec<_>>(), ["SHARED"]);
        let (values, missing) = outside.fetch().unwrap();
        assert!(missing.is_empty());
        assert!(values
            .iter()
            .any(|entry| entry.name == "PARENT" && entry.value.0 == "p"));
        assert!(values
            .iter()
            .any(|entry| entry.name == "SHARED" && entry.value.0 == "nearest"));

        let file = crate::cli::ensure_commands_file(Some(&cwd));
        let run = || {
            Bundle::discover(
                cwd.clone(),
                Some(("build".into(), Some(cwd.clone()))),
                Some(&cwd),
            )
            .unwrap()
        };
        let command = "[commands]\nbuild = { run = 'true', secrets = ['A'] }";
        std::fs::write(&file, command).unwrap();
        let first = run();
        std::fs::write(&file, command.replace("true", "false")).unwrap();
        assert_ne!(first.digest, run().digest);
        std::fs::write(&file, command.replace("['A']", "['B']")).unwrap();
        assert_ne!(first.digest, run().digest);
        assert_eq!(first.definition.unwrap().run, "true");
    }
}

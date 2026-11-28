//! Freeze the host's selection before asking. Read values only after the claim.
use super::*;
use crate::broker::principal::RecordOwner;
use crate::cli::run::{resolve_command_target, ResolvedRunTarget};
use plexi_protocol::{
    ApprovedBundle, BundleDefinition, BundleValue, SecretBundleReply, SecretInput,
};
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
    ) -> Result<Self, TargetError> {
        let root = crate::cli::resolve_commands_workspace_root(&cwd);
        if let (Some(pane), Some(workspace)) = (pane_root, root.as_deref()) {
            if workspace != pane {
                return Err(TargetError::OutsidePane {
                    workspace: workspace.into(),
                    pane: pane.into(),
                });
            }
        }
        let (config, router) = match root.as_deref() {
            Some(path) => (WorkspaceConfig::load(path)?, WorkspaceSecrets::load(path)?),
            None => (None, None),
        };
        let mut names = BTreeMap::new();
        let mut input = json!({"scope":cwd});
        let definition = if let Some((id, requested_root)) = command {
            if requested_root != root {
                return Err(TargetError::Unavailable(
                    "Command workspace changed or is outside the pane context. Run from its workspace root.".into(),
                ));
            }
            let Some(ResolvedRunTarget::TomlCommand {
                scope,
                run,
                steps,
                on_load,
                secrets: required,
            }) = resolve_command_target(root.as_deref(), &id)?
            else {
                return Err(format!("Command '{id}' has no secret-bearing definition. Check commands.toml and retry.").into());
            };
            if required.is_empty() {
                return Err(format!(
                    "Command '{id}' no longer requires secrets. Check commands.toml and retry."
                )
                .into());
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
            return Err(format!("Invalid environment variable name '{name}'. Check the workspace secret configuration.").into());
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

    fn fetch(
        &self,
        folder_store: &dyn NonDestructiveStore,
        workspace_store: &dyn NonDestructiveStore,
        connected: &(dyn Fn() -> bool + Sync),
    ) -> SecretBundleReply {
        let folders = LiveStore {
            inner: folder_store,
            connected,
        };
        let workspace = LiveStore {
            inner: workspace_store,
            connected,
        };
        let mut values = Vec::new();
        let mut missing = Vec::new();
        for (name, folder) in &self.names {
            let value = if let Some(folder) = folder {
                folders.get_interactive(&secrets::folder_account_name(folder, name))
            } else if let Some((config, router)) = &self.workspace {
                let consumer = if self.definition.is_some() {
                    crate::cli::PLUGIN_ID
                } else {
                    "terminal"
                };
                secrets::resolve_interactive(&config.id, consumer, name, router, &workspace).map(
                    |outcome| match outcome {
                        ResolveOutcome::Found(value) => Some(value),
                        ResolveOutcome::HardMissing { reason } => {
                            log::info!("secret.exec: missing {name}: {reason}");
                            None
                        }
                        ResolveOutcome::PromptUser => None,
                    },
                )
            } else {
                Ok(None)
            };
            match value {
                Ok(Some(mut value)) => values.push(BundleValue {
                    name: name.clone(),
                    value: SecretInput(std::mem::take(&mut *value)),
                }),
                Ok(None) => missing.push(name.clone()),
                Err(cause) => {
                    return SecretBundleReply::Error {
                        error: format!("Could not read secret '{name}': {cause}"),
                    }
                }
            }
        }
        if missing.is_empty() {
            SecretBundleReply::Values(ApprovedBundle {
                cwd: self.cwd.clone(),
                values,
                definition: self.definition.clone(),
            })
        } else {
            SecretBundleReply::Missing { names: missing }
        }
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
        let actor = RecordOwner::requester(&principal);
        let audit = format!(
            "{resource_id} names={}",
            bundle.names.keys().cloned().collect::<Vec<_>>().join(",")
        );
        monitor.note_use(&actor, &call, &grant_id, &fingerprint, &audit, "")?;
        let result = bundle.fetch(secrets::folder_store(), secrets::system_store(), &|| {
            self.client_connected()
        });
        let outcome = match &result {
            SecretBundleReply::Values(_) => "ok",
            SecretBundleReply::Missing { .. } => "missing",
            _ => "failed",
        };
        let run = monitor
            .records_for(&principal)
            .run(&call)
            .filter(|record| record.correlation_id.as_deref() == Some(call.as_str()))
            .map(|record| record.handle());
        monitor.note_outcome(
            run.as_ref(),
            &actor,
            &call,
            &grant_id,
            &fingerprint,
            &audit,
            "",
            outcome,
            "",
            "",
        )?;
        let values = match result {
            SecretBundleReply::Values(values) => values,
            failure @ (SecretBundleReply::Missing { .. }
            | SecretBundleReply::Error { .. }
            | SecretBundleReply::Denied { .. }) => {
                stream_reply(&self.reply_path, json!(failure));
                return Ok(());
            }
        };
        let target = format!("exec:{}", bundle.digest);
        let permit = match self.release_permit(monitor, &target, *approved) {
            Ok(permit) => permit,
            Err(error) => {
                log::info!("secret.exec: release withheld {label}: {error}");
                stream_reply(&self.reply_path, json!({"status":"denied"}));
                return Ok(());
            }
        };
        release_secret(
            permit,
            &target,
            &self.reply_path,
            &format!("secret.exec {label}"),
            || SecretBundleReply::Values(values),
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
    fn bundle_nested_pane_refuses_parent_before_admission() {
        let dir = tempfile::tempdir().unwrap();
        let _profile = crate::config::set_test_profile_dir(dir.path().join("profile"));
        let parent = dir.path().canonicalize().unwrap();
        let pane = parent.join("nested");
        std::fs::create_dir(&pane).unwrap();
        let config =
            secrets::init_workspace(&parent, &crate::config::workspace_channel_dir()).unwrap();
        let account = secrets::keychain_workspace_name(&config.id, "PARENT_ONLY");
        secrets::system_store()
            .set(&account, "parent-canary")
            .unwrap();
        secrets::write_terminal_env_inject(&parent, "PARENT_ONLY", true).unwrap();
        let outside = Bundle::discover(pane.clone(), None, None).unwrap();
        let SecretBundleReply::Values(approved) =
            outside.fetch(secrets::folder_store(), secrets::system_store(), &|| true)
        else {
            panic!("outside caller lost its parent workspace bundle");
        };
        assert!(approved
            .values
            .iter()
            .any(|v| v.name == "PARENT_ONLY" && v.value.0 == "parent-canary"));
        let monitor = PermissionMonitor::for_profile(&crate::config::config_dir()).unwrap();
        let ask = SecretAsk::Bundle {
            cwd: pane.clone(),
            command: None,
        };
        let principal = Principal::Pane(crate::broker::principal::PanePrincipal::for_test(7));
        let Err(reply) = ask.discover_target(
            "secret.exec",
            "environment".into(),
            Some(&pane),
            &principal,
            &monitor,
        ) else {
            panic!("nested pane reached bundle admission");
        };
        assert_eq!(reply["status"], "denied");
        assert_eq!(reply["error_code"], "permission_denied");
        assert!(reply["reason"]
            .as_str()
            .unwrap()
            .contains("outside the pane context"));
        assert!(reply.get("values").is_none());
        assert!(monitor.host_records().open().is_empty());
        assert!(monitor.host_records().audit().iter().any(|row| {
            row.actor == RecordOwner::requester(&principal)
                && row.call_id == "secret.exec"
                && row.decision == "context_scope_violation"
        }));
    }

    #[derive(Default)]
    struct RecordingStore {
        reads: std::sync::Mutex<Vec<String>>,
    }

    impl NonDestructiveStore for RecordingStore {
        fn get(&self, account: &str) -> Option<Zeroizing<String>> {
            self.reads.lock().unwrap().push(account.into());
            if account.starts_with("plexi:folder:") {
                Some(Zeroizing::new("folder-canary".into()))
            } else {
                account
                    .starts_with("plexi:user:")
                    .then(|| Zeroizing::new("global-canary".into()))
            }
        }

        fn get_interactive(&self, account: &str) -> Result<Option<Zeroizing<String>>, SecretError> {
            let value = self.get(account);
            if account == "plexi:ws:LOCKED" {
                return Err(SecretError::Backend("authentication denied".into()));
            }
            Ok(value)
        }

        fn add_new(&self, _: &str, _: &str) -> Result<(), SecretError> {
            unreachable!()
        }
        fn delete_if_value(&self, _: &str, _: &str) -> Result<(), SecretError> {
            unreachable!()
        }
        fn list_with_prefix(&self, _: &str) -> Vec<String> {
            unreachable!()
        }
        fn scan_accounts(&self) -> Result<Vec<String>, SecretError> {
            unreachable!()
        }
    }

    fn workspace_bundle() -> Bundle {
        Bundle {
            cwd: PathBuf::from("/workspace"),
            names: BTreeMap::new(),
            workspace: Some((
                WorkspaceConfig {
                    id: "ws".into(),
                    context: None,
                },
                WorkspaceSecrets::parse("fallback = true").unwrap(),
            )),
            definition: None,
            input: String::new(),
            digest: String::new(),
        }
    }

    #[test]
    fn bundle_disconnect_after_first_account_stops_reads() {
        let mut bundle = workspace_bundle();
        bundle
            .names
            .insert("SECOND".into(), Some(bundle.cwd.clone()));
        for first_folder in [None, Some(bundle.cwd.clone())] {
            let first_account = first_folder.as_ref().map_or_else(
                || secrets::keychain_workspace_name("ws", "FIRST"),
                |path| secrets::folder_account_name(path, "FIRST"),
            );
            bundle.names.insert("FIRST".into(), first_folder);
            let store = RecordingStore::default();
            let result = bundle.fetch(&store, &store, &|| store.reads.lock().unwrap().is_empty());
            assert_eq!(store.reads.lock().unwrap().as_slice(), [first_account]);
            assert!(
                matches!(result, SecretBundleReply::Error { .. }),
                "disconnected bundle produced values"
            );
        }
    }

    #[test]
    fn bundle_workspace_auth_error_stops_global_fallback() {
        let mut bundle = workspace_bundle();
        bundle.names.insert("LOCKED".into(), None);
        let store = RecordingStore::default();
        let reply = json!(bundle.fetch(&store, &store, &|| true));
        assert_eq!(
            reply["status"], "error",
            "workspace authentication failure must end the bundle"
        );
        let error = reply["error"].as_str().unwrap();
        assert!(error.contains("LOCKED") && error.contains("authentication denied"));
        assert!(reply.get("values").is_none());
        assert!(!reply.to_string().contains("global-canary"));
        assert_eq!(store.reads.lock().unwrap().as_slice(), ["plexi:ws:LOCKED"]);
    }

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
        let SecretBundleReply::Values(approved) =
            outside.fetch(secrets::folder_store(), secrets::system_store(), &|| true)
        else {
            panic!("outside caller should receive both folder values");
        };
        assert_eq!(approved.cwd, cwd);
        let values = approved.values;
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

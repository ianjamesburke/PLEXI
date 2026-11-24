//! Admitted mutations on the host's secret worker.
use super::*;
use crate::broker::gate::{AdmitRequest, UseDecision};
use crate::protocol::SecretInput;
use crate::workspace::secrets::manager::{mutate_secret, Mutation, SecretTarget};

impl PlexiApp {
    pub(in crate::app) fn serve_secret_mutation(
        &self,
        name: &str,
        ask: SecretAsk,
        origin: &RequestOrigin,
        response_file: &str,
        set: Option<(SecretInput, Option<String>)>,
    ) {
        let tool = if set.is_some() {
            "secret.set"
        } else {
            "secret.delete"
        };
        self.run_secret_request(
            tool,
            name,
            ask,
            origin,
            response_file,
            move |request, monitor| {
                let body = request
                    .mutate(&monitor, tool, set.as_ref())
                    .unwrap_or_else(|error| {
                        Some(serde_json::json!({"status":"error","error":error}))
                    });
                if let Some(body) = body {
                    stream_reply(&request.reply_path, body);
                }
            },
        );
    }
}

impl SecretRequest {
    fn mutate(
        &self,
        monitor: &PermissionMonitor,
        tool: &str,
        set: Option<&(SecretInput, Option<String>)>,
    ) -> Result<Option<serde_json::Value>, String> {
        let (target, root) = match &self.scope {
            SecretScope::Workspace { id, root } => (
                SecretTarget::Workspace {
                    id: id.clone(),
                    root: root.clone(),
                },
                root,
            ),
            SecretScope::Folder { path } => (SecretTarget::Folder(path.clone()), path),
            SecretScope::Global => (SecretTarget::Global, &self.profile),
        };
        if matches!(self.scope, SecretScope::Folder { .. })
            && set.is_some_and(|(value, _)| value.0.is_empty())
        {
            return Err("empty value, nothing stored".into());
        }
        let mut input = self.scope.input(tool, &self.name, self.subject.outside());
        let mutation = match set {
            Some((value, Some(canonical))) => {
                input["route_name"] = canonical.as_str().into();
                Mutation::SetRouted {
                    canonical,
                    value: &value.0,
                }
            }
            Some((value, None)) => Mutation::Set(&value.0),
            None => Mutation::Delete,
        };
        let principal = self.subject.principal();
        let call = format!("{tool}-{}", uuid::Uuid::new_v4());
        let Some(admission) = self.admit(
            monitor,
            AdmitRequest {
                call_id: &call,
                tool,
                input_json: &input.to_string(),
                principal: &principal,
                actor_scope: crate::broker::ActorScope::User,
                trust_origin: "host",
                workspace_root: root,
                context_id: 0,
                package_id: "",
                instance_id: 0,
                target_type: crate::broker::TargetType::Secret,
            },
        ) else {
            return Ok(None);
        };
        let UseDecision::Allowed {
            approved,
            grant_id,
            fingerprint,
            resource_id,
        } = admission
        else {
            return Ok(Some(serde_json::json!({"status":"denied"})));
        };
        monitor.note_use(
            &crate::broker::principal::RecordOwner::requester(&principal),
            &call,
            &grant_id,
            &fingerprint,
            &resource_id,
            "",
        )?;
        // Recheck the claimed grant and pane together before handing the
        // mutation to the worker's store. Store I/O can block.
        {
            let _commit = monitor.commit(&approved)?;
            self.subject.current()?;
        }
        let result = mutate_secret(self.scope.store(), monitor, &target, &self.name, mutation);
        monitor
            .note_outcome(
                None,
                &crate::broker::principal::RecordOwner::requester(&principal),
                &call,
                &grant_id,
                &fingerprint,
                &resource_id,
                "",
                if result.is_ok() { "ok" } else { "failed" },
                "",
                "",
            )
            .map_err(|_| "Could not audit the mutation outcome. Refresh before retrying.")?;
        result?;
        let mut reply = serde_json::json!({"status":"ok", "target":target});
        if set.is_some() && matches!(target, SecretTarget::Folder(_)) {
            reply["backend"] = crate::workspace::secrets::backend_label().into();
        }
        Ok(Some(reply))
    }
}

use std::path::PathBuf;
use std::time::Duration;

use crate::protocol::AppRequest;

/// How long a `pane new --agent` spawn waits for the agent's first idle
/// self-report when the caller does not say. Generous on purpose: a cold agent
/// TUI can spend tens of seconds loading before it runs its `SessionStart` hook.
pub(crate) const DEFAULT_AGENT_BOOT_TIMEOUT: Duration = Duration::from_secs(60);

/// A `pane new --agent` request folded into the launch spec.
///
/// The command is a shell expression Plexi does not parse or validate — the
/// caller names a tier alias (`c-large`, `codex-small`) and Plexi types it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AgentBootSpec {
    pub(crate) command: String,
    pub(crate) timeout: Duration,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PaneLaunchTarget {
    Terminal,
    AppId(String),
    Path(PathBuf),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PaneLaunchSpec {
    pub(crate) target: PaneLaunchTarget,
    pub(crate) layout: Option<String>,
    pub(crate) args: Vec<String>,
    pub(crate) ephemeral: bool,
    pub(crate) response_file: Option<String>,
    pub(crate) from_pane_id: Option<u64>,
    pub(crate) cwd: Option<PathBuf>,
    pub(crate) no_focus: bool,
    pub(crate) workspace_root: Option<PathBuf>,
    pub(crate) target_context: Option<u64>,
    pub(crate) context_name: Option<String>,
    pub(crate) name: Option<String>,
    pub(crate) agent: Option<AgentBootSpec>,
    pub(crate) worktree: Option<String>,
    pub(crate) result_schema: Option<serde_json::Value>,
    /// Host-stamped. See `AppRequest::SpawnPane::peer_ancestry`.
    pub(crate) peer_ancestry: Option<Vec<u32>>,
}

impl PaneLaunchSpec {
    pub(crate) fn path(path: impl Into<PathBuf>, args: Vec<String>) -> Result<Self, String> {
        let path = path.into();
        if path.as_os_str().is_empty() {
            return Err("path launch target is empty".to_string());
        }
        Ok(Self {
            target: PaneLaunchTarget::Path(path),
            layout: None,
            args,
            ephemeral: false,
            response_file: None,
            from_pane_id: None,
            cwd: None,
            no_focus: false,
            workspace_root: None,
            target_context: None,
            context_name: None,
            name: None,
            agent: None,
            worktree: None,
            result_schema: None,
            peer_ancestry: None,
        })
    }

    pub(crate) fn from_spawn_pane(request: &AppRequest) -> Result<Self, String> {
        let AppRequest::SpawnPane {
            type_id,
            layout,
            args,
            response_file,
            ephemeral,
            from_pane_id,
            cwd,
            no_focus,
            path,
            workspace_root,
            target_context,
            context_name,
            name,
            agent_cmd,
            boot_timeout_secs,
            worktree,
            result_schema,
            peer_ancestry,
            ..
        } = request
        else {
            return Err("expected spawn_pane request".to_string());
        };

        if context_name.as_deref().is_some_and(|c| !c.is_empty()) && type_id != "terminal" {
            return Err("context_name is only supported for terminal spawns".to_string());
        }

        let worktree = match worktree.as_deref().map(str::trim).filter(|w| !w.is_empty()) {
            Some(branch) => {
                if type_id != "terminal" {
                    return Err("worktree is only supported for terminal spawns".to_string());
                }
                if branch.starts_with('-') {
                    return Err(format!("invalid worktree branch name: {branch}"));
                }
                Some(branch.to_string())
            }
            None => {
                if worktree.is_some() {
                    return Err("worktree branch name cannot be empty".to_string());
                }
                None
            }
        };

        let result_schema = match result_schema {
            Some(schema_val) => {
                let base_dir = cwd.as_deref().map(std::path::Path::new);
                Some(crate::host::pane_result::normalize_schema_value(
                    schema_val, base_dir,
                )?)
            }
            None => None,
        };

        let agent = match agent_cmd.as_deref().filter(|cmd| !cmd.is_empty()) {
            Some(command) => {
                if type_id != "terminal" {
                    return Err("agent_cmd is only supported for terminal spawns".to_string());
                }
                let timeout = match boot_timeout_secs {
                    // Bounded above MAX_WAIT_TIMEOUT_SECS: Duration::from_secs_f64
                    // panics on finite values beyond Duration's range.
                    Some(secs)
                        if !secs.is_finite()
                            || *secs <= 0.0
                            || *secs > crate::app::pane_wait::MAX_WAIT_TIMEOUT_SECS =>
                    {
                        return Err(format!(
                            "boot_timeout_secs must be a positive number of seconds at most {}, got {secs}",
                            crate::app::pane_wait::MAX_WAIT_TIMEOUT_SECS
                        ));
                    }
                    Some(secs) => Duration::from_secs_f64(*secs),
                    None => DEFAULT_AGENT_BOOT_TIMEOUT,
                };
                Some(AgentBootSpec {
                    command: command.to_string(),
                    timeout,
                })
            }
            None => {
                if boot_timeout_secs.is_some() {
                    return Err("boot_timeout_secs requires agent_cmd".to_string());
                }
                None
            }
        };

        let target = match (type_id.as_str(), path.as_deref()) {
            ("terminal", None) => PaneLaunchTarget::Terminal,
            ("", Some(path)) => PaneLaunchTarget::Path(PathBuf::from(path)),
            (type_id, None) if !type_id.is_empty() => PaneLaunchTarget::AppId(type_id.to_string()),
            ("terminal", Some(_)) => {
                return Err("spawn_pane cannot set both terminal and path target".to_string());
            }
            (type_id, Some(_)) if !type_id.is_empty() => {
                return Err("spawn_pane cannot set both app id and path target".to_string());
            }
            ("", None) => return Err("spawn_pane target is empty".to_string()),
            _ => return Err("invalid spawn_pane target".to_string()),
        };

        Ok(Self {
            target,
            layout: layout.clone(),
            args: args.clone(),
            ephemeral: *ephemeral,
            response_file: response_file.clone(),
            from_pane_id: *from_pane_id,
            cwd: cwd.as_deref().map(PathBuf::from),
            no_focus: *no_focus,
            workspace_root: workspace_root.as_deref().map(PathBuf::from),
            target_context: *target_context,
            context_name: context_name.clone(),
            name: name.clone(),
            agent,
            worktree,
            result_schema,
            peer_ancestry: peer_ancestry.clone(),
        })
    }

    pub(crate) fn with_layout(mut self, layout: Option<String>) -> Self {
        self.layout = layout;
        self
    }

    pub(crate) fn with_from_pane_id(mut self, from_pane_id: Option<u64>) -> Self {
        self.from_pane_id = from_pane_id;
        self
    }

    pub(crate) fn with_response_file(mut self, response_file: Option<String>) -> Self {
        self.response_file = response_file;
        self
    }

    pub(crate) fn to_spawn_pane_request(&self) -> AppRequest {
        let (type_id, path) = match &self.target {
            PaneLaunchTarget::Terminal => ("terminal".to_string(), None),
            PaneLaunchTarget::AppId(type_id) => (type_id.clone(), None),
            PaneLaunchTarget::Path(path) => {
                (String::new(), Some(path.to_string_lossy().into_owned()))
            }
        };

        AppRequest::SpawnPane {
            type_id,
            layout: self.layout.clone(),
            args: self.args.clone(),
            from_pane_id: self.from_pane_id,
            request_id: None,
            response_file: self.response_file.clone(),
            ephemeral: self.ephemeral,
            cwd: self.cwd.as_ref().map(|p| p.to_string_lossy().into_owned()),
            no_focus: self.no_focus,
            path,
            workspace_root: self
                .workspace_root
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned()),
            target_context: self.target_context,
            context_name: self.context_name.clone(),
            name: self.name.clone(),
            agent_cmd: self.agent.as_ref().map(|agent| agent.command.clone()),
            boot_timeout_secs: self.agent.as_ref().map(|agent| agent.timeout.as_secs_f64()),
            worktree: self.worktree.clone(),
            result_schema: self.result_schema.clone(),
            peer_ancestry: self.peer_ancestry.clone(),
        }
    }

    pub(crate) fn target_for_log(&self) -> String {
        match &self.target {
            PaneLaunchTarget::Terminal => "terminal".to_string(),
            PaneLaunchTarget::AppId(type_id) => format!("app:{type_id}"),
            PaneLaunchTarget::Path(path) => format!("path:{}", path.display()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{PaneLaunchSpec, PaneLaunchTarget, DEFAULT_AGENT_BOOT_TIMEOUT};
    use crate::protocol::AppRequest;

    fn spawn_pane(type_id: &str, path: Option<&str>, args: &[&str]) -> AppRequest {
        AppRequest::SpawnPane {
            type_id: type_id.to_string(),
            layout: Some("split_v".to_string()),
            args: args.iter().map(|arg| arg.to_string()).collect(),
            from_pane_id: Some(7),
            request_id: None,
            response_file: Some("/tmp/response.json".to_string()),
            ephemeral: false,
            cwd: None,
            no_focus: false,
            path: path.map(str::to_string),
            workspace_root: None,
            target_context: None,
            context_name: None,
            name: None,
            agent_cmd: None,
            boot_timeout_secs: None,
            worktree: None,
            result_schema: None,
            peer_ancestry: None,
        }
    }

    fn agent_spawn(type_id: &str, agent_cmd: Option<&str>, timeout: Option<f64>) -> AppRequest {
        let mut request = spawn_pane(type_id, None, &[]);
        if let AppRequest::SpawnPane {
            agent_cmd: slot,
            boot_timeout_secs,
            ..
        } = &mut request
        {
            *slot = agent_cmd.map(str::to_string);
            *boot_timeout_secs = timeout;
        }
        request
    }

    #[test]
    fn from_spawn_pane_defaults_agent_boot_timeout() {
        let spec = PaneLaunchSpec::from_spawn_pane(&agent_spawn("terminal", Some("c-large"), None))
            .expect("valid launch spec");

        let agent = spec.agent.expect("agent boot spec");
        assert_eq!(agent.command, "c-large");
        assert_eq!(agent.timeout, DEFAULT_AGENT_BOOT_TIMEOUT);
    }

    #[test]
    fn from_spawn_pane_rejects_agent_on_app_target() {
        let err = PaneLaunchSpec::from_spawn_pane(&agent_spawn("snake", Some("c-large"), None))
            .unwrap_err();

        assert_eq!(err, "agent_cmd is only supported for terminal spawns");
    }

    #[test]
    fn from_spawn_pane_rejects_boot_timeout_without_agent() {
        let err = PaneLaunchSpec::from_spawn_pane(&agent_spawn("terminal", None, Some(30.0)))
            .unwrap_err();

        assert_eq!(err, "boot_timeout_secs requires agent_cmd");
    }

    #[test]
    fn from_spawn_pane_rejects_non_positive_boot_timeout() {
        let err =
            PaneLaunchSpec::from_spawn_pane(&agent_spawn("terminal", Some("c-large"), Some(0.0)))
                .unwrap_err();

        assert!(
            err.contains("must be a positive number of seconds"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn from_spawn_pane_rejects_oversized_boot_timeout() {
        // Finite values beyond Duration's range would panic in
        // Duration::from_secs_f64 — must be a validation error instead.
        let err =
            PaneLaunchSpec::from_spawn_pane(&agent_spawn("terminal", Some("c-large"), Some(1e308)))
                .unwrap_err();

        assert!(
            err.contains("at most"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn from_spawn_pane_preserves_path_args() {
        let request = spawn_pane("", Some("/tmp/app.wasm"), &["--sample", "96"]);

        let spec = PaneLaunchSpec::from_spawn_pane(&request).expect("valid launch spec");

        assert_eq!(spec.target, PaneLaunchTarget::Path("/tmp/app.wasm".into()));
        assert_eq!(spec.args, ["--sample", "96"]);
    }

    #[test]
    fn from_spawn_pane_rejects_empty_target() {
        let request = spawn_pane("", None, &[]);

        let err = PaneLaunchSpec::from_spawn_pane(&request).unwrap_err();

        assert_eq!(err, "spawn_pane target is empty");
    }

    #[test]
    fn from_spawn_pane_rejects_app_id_and_path_conflict() {
        let request = spawn_pane("demo", Some("/tmp/app.wasm"), &[]);

        let err = PaneLaunchSpec::from_spawn_pane(&request).unwrap_err();

        assert_eq!(err, "spawn_pane cannot set both app id and path target");
    }

    #[test]
    fn from_spawn_pane_accepts_worktree_on_terminal() {
        let mut request = spawn_pane("terminal", None, &[]);
        if let AppRequest::SpawnPane { worktree, .. } = &mut request {
            *worktree = Some("feature/0668-test".to_string());
        }
        let spec = PaneLaunchSpec::from_spawn_pane(&request).expect("valid launch spec");
        assert_eq!(spec.worktree.as_deref(), Some("feature/0668-test"));
        assert_eq!(
            PaneLaunchSpec::from_spawn_pane(&spec.to_spawn_pane_request()).expect("round trip"),
            spec
        );
    }

    #[test]
    fn from_spawn_pane_rejects_worktree_on_app_target() {
        let mut request = spawn_pane("snake", None, &[]);
        if let AppRequest::SpawnPane { worktree, .. } = &mut request {
            *worktree = Some("feature/0668-test".to_string());
        }
        let err = PaneLaunchSpec::from_spawn_pane(&request).unwrap_err();
        assert_eq!(err, "worktree is only supported for terminal spawns");
    }

    #[test]
    fn from_spawn_pane_rejects_flag_like_worktree_branch() {
        let mut request = spawn_pane("terminal", None, &[]);
        if let AppRequest::SpawnPane { worktree, .. } = &mut request {
            *worktree = Some("-b".to_string());
        }
        let err = PaneLaunchSpec::from_spawn_pane(&request).unwrap_err();
        assert!(err.contains("invalid worktree branch name"));
    }

    #[test]
    fn from_spawn_pane_accepts_and_validates_result_schema() {
        let mut request = spawn_pane("terminal", None, &[]);
        let schema = serde_json::json!({
            "type": "object",
            "required": ["verdict"],
            "properties": {
                "verdict": { "type": "string", "enum": ["PASS", "FAIL"] }
            }
        });
        if let AppRequest::SpawnPane { result_schema, .. } = &mut request {
            *result_schema = Some(schema.clone());
        }
        let spec = PaneLaunchSpec::from_spawn_pane(&request).expect("valid launch spec");
        assert_eq!(spec.result_schema, Some(schema));
        assert_eq!(
            PaneLaunchSpec::from_spawn_pane(&spec.to_spawn_pane_request()).expect("round trip"),
            spec
        );

        let mut bad_request = spawn_pane("terminal", None, &[]);
        if let AppRequest::SpawnPane { result_schema, .. } = &mut bad_request {
            *result_schema = Some(serde_json::json!({"type": "not_a_valid_type"}));
        }
        let err = PaneLaunchSpec::from_spawn_pane(&bad_request).unwrap_err();
        assert!(err.contains("unknown JSON Schema type"), "unexpected error: {err}");
    }
}

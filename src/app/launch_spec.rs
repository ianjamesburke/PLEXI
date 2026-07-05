use std::path::PathBuf;

use crate::app_protocol::AppRequest;

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
    pub(crate) name: Option<String>,
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
            name: None,
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
            name,
            ..
        } = request
        else {
            return Err("expected spawn_pane request".to_string());
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
            name: name.clone(),
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
            name: self.name.clone(),
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
    use super::{PaneLaunchSpec, PaneLaunchTarget};
    use crate::app_protocol::AppRequest;

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
            name: None,
        }
    }

    #[test]
    fn from_spawn_pane_preserves_path_args() {
        let request = spawn_pane("", Some("/tmp/app.wasm"), &["--sample", "96"]);

        let spec = PaneLaunchSpec::from_spawn_pane(&request).expect("valid launch spec");

        assert_eq!(
            spec.target,
            PaneLaunchTarget::Path("/tmp/app.wasm".into())
        );
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
}

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PaneId(pub u64);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FrameId(pub u64);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AppId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RequestId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RunId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PipeId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SecretKey(pub String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceRoot(PathBuf);

impl WorkspaceRoot {
    pub fn new(path: impl Into<PathBuf>) -> Result<Self, WorkspaceRootError> {
        let path = path.into();
        if !path.is_absolute() {
            return Err(WorkspaceRootError::RelativePath(path));
        }
        Ok(Self(path))
    }

    pub fn as_path(&self) -> &Path {
        self.0.as_path()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum WorkspaceRootError {
    #[error("workspace root must be absolute: {0}")]
    RelativePath(PathBuf),
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::protocol::schema::WorkspaceRoot;

    #[test]
    fn workspace_root_rejects_relative_path() {
        let err = WorkspaceRoot::new(PathBuf::from("relative/path")).expect_err("must reject");
        assert!(err.to_string().contains("must be absolute"));
    }
}

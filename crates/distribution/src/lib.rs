//! Package validation and channel-scoped installation shared by every installer.
// Helpers start through bounded::Helper; tests may start processes freely.
#![deny(clippy::disallowed_methods)]
#![cfg_attr(test, allow(clippy::disallowed_methods))]
pub mod bounded;
pub mod host_presence;
pub mod package;
pub mod profile_target;
pub mod release;
pub mod strays;
pub mod transaction;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error("{0}")]
    Invalid(String),
    #[error("{action}: {source}")]
    Io {
        action: String,
        source: std::io::Error,
    },
    #[error("{0}")]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Helper(#[from] bounded::RunError),
}
pub type Result<T> = std::result::Result<T, Error>;

pub(crate) trait IoContext<T> {
    fn context(self, action: impl Into<String>) -> Result<T>;
}
impl<T> IoContext<T> for std::io::Result<T> {
    fn context(self, action: impl Into<String>) -> Result<T> {
        self.map_err(|source| Error::Io {
            action: action.into(),
            source,
        })
    }
}

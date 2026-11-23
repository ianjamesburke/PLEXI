//! Package validation and channel-scoped installation shared by every installer.
// Helpers start through bounded::Helper; tests may start processes freely.
#![deny(clippy::disallowed_methods)]
#![cfg_attr(test, allow(clippy::disallowed_methods))]
pub mod bounded;
pub mod host_presence;
pub mod package;
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
    /// GitHub kept failing with a server error, timeout or reset after the
    /// bounded retries. The text is for people; `detail` (status or transport
    /// error, URL and attempt count) is already in the log.
    #[error("GitHub is slow, try again")]
    GithubSlow { detail: String },
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

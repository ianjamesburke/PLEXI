//! Package validation and channel-scoped installation shared by every installer.
pub mod package;
pub mod release;
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

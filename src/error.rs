use thiserror::Error;

#[derive(Error, Debug)]
pub enum PlexiError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Protocol error: {0}")]
    Protocol(String),

    #[error("Permission denied: {0}")]
    Permission(String),

    #[error("App launch failed: {0}")]
    AppLaunch(String),

    #[error("Media error: {0}")]
    Media(String),

    #[error("Pipe error: {0}")]
    Pipe(String),

    #[error("Registry error: {0}")]
    Registry(String),

    #[error("Not implemented")]
    NotImplemented,
}

pub type PlexiResult<T> = Result<T, PlexiError>;

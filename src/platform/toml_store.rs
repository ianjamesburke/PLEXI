//! A TOML file that holds one serializable payload.
//!
//! Permission stores keep their payload here. Sealed load and save live in
//! `broker::seal`, because a MAC has to be checked before the body is parsed.

use std::path::PathBuf;

/// `label` prefixes every log line this store emits, so the messages stay
/// attributable to the owning subsystem (`grant_store`, `permission_store`).
#[derive(Debug)]
pub struct TomlStore<T> {
    pub data: T,
    pub path: PathBuf,
    label: &'static str,
}

impl<T: Default> TomlStore<T> {
    /// An in-memory store with no backing file — this is the shape tests
    /// construct when they only exercise the payload rules.
    pub fn detached(label: &'static str) -> Self {
        Self {
            data: T::default(),
            path: PathBuf::new(),
            label,
        }
    }
}

impl<T> TomlStore<T> {
    /// A store whose payload was already checked by the caller.
    pub fn at(path: PathBuf, label: &'static str, data: T) -> Self {
        Self { data, path, label }
    }

    pub fn label(&self) -> &'static str {
        self.label
    }
}

//! A TOML file that holds one serializable payload, loads to a default when
//! absent, backs itself up when corrupt, and saves atomically.
//!
//! Both permission stores (`app::permissions::PermissionStore` over
//! `permissions.toml` and `broker::GrantStore` over `grants.toml`) were the
//! same file-handling code written twice, down to the log wording. That
//! handling lives here now; each store keeps only its own payload and rules.

use serde::Serialize;
use serde::de::DeserializeOwned;
use std::path::{Path, PathBuf};

/// `label` prefixes every log line this store emits, so the messages stay
/// attributable to the owning subsystem (`grant_store`, `permission_store`).
#[derive(Debug)]
pub struct TomlStore<T> {
    pub data: T,
    pub path: PathBuf,
    label: &'static str,
}

impl<T: Default> TomlStore<T> {
    /// An in-memory store with no backing file. `save` is a no-op — this is
    /// the shape tests construct when they only exercise the payload rules.
    pub fn detached(label: &'static str) -> Self {
        Self {
            data: T::default(),
            path: PathBuf::new(),
            label,
        }
    }
}

impl<T: Default + DeserializeOwned + Serialize> TomlStore<T> {
    /// Load `dir/file_name`.
    ///
    /// - Absent file → default payload, no logging (first run).
    /// - Parse failure → rename the file to `<file_name>.corrupt-<unix_secs>`
    ///   and return the default payload. Fail open to empty, never crash the
    ///   host; the operator keeps the original for recovery.
    /// - Success → `on_loaded` is called with the payload so the owning
    ///   module can log its own count in its own vocabulary.
    pub fn load_or_default(
        dir: &Path,
        file_name: &str,
        label: &'static str,
        on_loaded: impl FnOnce(&T, &Path),
    ) -> Self {
        let path = dir.join(file_name);
        let Ok(raw) = std::fs::read_to_string(&path) else {
            return Self {
                data: T::default(),
                path,
                label,
            };
        };
        match toml::from_str::<T>(&raw) {
            Ok(data) => {
                on_loaded(&data, &path);
                Self { data, path, label }
            }
            Err(e) => {
                let backup = path.with_file_name(format!(
                    "{file_name}.corrupt-{}",
                    crate::platform::clock::now_secs()
                ));
                log::error!(
                    "{label}: failed to parse {}: {e} — backing up to {}",
                    path.display(),
                    backup.display()
                );
                if let Err(rename_err) = std::fs::rename(&path, &backup) {
                    log::error!(
                        "{label}: could not rename corrupt file to {}: {rename_err}",
                        backup.display()
                    );
                }
                Self {
                    data: T::default(),
                    path,
                    label,
                }
            }
        }
    }

    /// Serialize and write atomically. Best-effort: a failure is logged, not
    /// propagated, because every caller is a UI-thread mutation that has
    /// already taken effect in memory. No-op for a [`detached`] store.
    ///
    /// [`detached`]: TomlStore::detached
    pub fn save(&self) {
        if self.path.as_os_str().is_empty() {
            return;
        }
        let label = self.label;
        match toml::to_string_pretty(&self.data) {
            Ok(s) => match crate::platform::fs::atomic_write(&self.path, s.as_bytes()) {
                Ok(()) => log::info!("{label}: saved {}", self.path.display()),
                Err(e) => log::error!("{label}: failed to save {}: {e}", self.path.display()),
            },
            Err(e) => log::error!("{label}: serialize error: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Default, Serialize, Deserialize, PartialEq)]
    struct Payload {
        #[serde(default)]
        items: Vec<String>,
    }

    #[test]
    fn missing_file_loads_default_without_touching_disk() {
        let dir = tempfile::tempdir().expect("dir");
        let store: TomlStore<Payload> =
            TomlStore::load_or_default(dir.path(), "grants.toml", "test_store", |_, _| {
                panic!("on_loaded must not fire for an absent file")
            });
        assert_eq!(store.data, Payload::default());
        assert!(!dir.path().join("grants.toml").exists());
    }

    #[test]
    fn round_trips_through_save_and_load() {
        let dir = tempfile::tempdir().expect("dir");
        let mut store: TomlStore<Payload> =
            TomlStore::load_or_default(dir.path(), "grants.toml", "test_store", |_, _| {});
        store.data.items.push("one".to_string());
        store.save();

        let mut loaded = false;
        let reread: TomlStore<Payload> =
            TomlStore::load_or_default(dir.path(), "grants.toml", "test_store", |_, _| {
                loaded = true;
            });
        assert!(loaded, "on_loaded fires for a parseable file");
        assert_eq!(reread.data.items, vec!["one".to_string()]);
    }

    #[test]
    fn corrupt_file_is_backed_up_and_store_falls_open_to_empty() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("grants.toml");
        std::fs::write(&path, "this is not = = toml").expect("seed corrupt file");

        let store: TomlStore<Payload> =
            TomlStore::load_or_default(dir.path(), "grants.toml", "test_store", |_, _| {
                panic!("on_loaded must not fire for a corrupt file")
            });
        assert_eq!(store.data, Payload::default());
        assert!(!path.exists(), "corrupt file is moved aside, not left");
        let backups: Vec<_> = std::fs::read_dir(dir.path())
            .expect("read dir")
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with("grants.toml.corrupt-"))
            .collect();
        assert_eq!(backups.len(), 1, "one backup, got {backups:?}");
    }

    #[test]
    fn detached_store_save_is_a_no_op() {
        let mut store: TomlStore<Payload> = TomlStore::detached("test_store");
        store.data.items.push("kept in memory".to_string());
        store.save();
        assert_eq!(store.data.items.len(), 1);
    }
}

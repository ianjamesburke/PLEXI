//! `plexi experiments list|enable|disable`: the per-feature switches in the
//! `[experiments]` table of the global config.toml.

use std::path::Path;

use crate::release::ReleaseFeature;

const TABLE: &str = "experiments";

/// Rows for `list`: feature plus whether the config has it on.
fn rows(path: &Path) -> Result<Vec<(ReleaseFeature, bool)>, String> {
    let enabled = crate::config::experiments_from_path(path)?;
    Ok(ReleaseFeature::ALL
        .iter()
        .map(|f| (*f, enabled.contains(f)))
        .collect())
}

pub fn experiments_list(json: bool) -> i32 {
    list_at(&crate::config::config_path(), json)
}

fn list_at(path: &Path, json: bool) -> i32 {
    let rows = match rows(path) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: experiments list: {e}");
            return 1;
        }
    };
    if json {
        let items: Vec<serde_json::Value> = rows
            .iter()
            .map(|(f, on)| {
                serde_json::json!({
                    "name": f.key(),
                    "enabled": on,
                    "requires_restart": f.requires_restart(),
                    "description": f.description(),
                })
            })
            .collect();
        println!("{}", serde_json::Value::Array(items));
        return 0;
    }
    let width = rows.iter().map(|(f, _)| f.key().len()).max().unwrap_or(0);
    println!("{:<width$}  STATE  RESTART  DESCRIPTION", "NAME");
    for (f, on) in rows {
        println!(
            "{:<width$}  {:<5}  {:<7}  {}",
            f.key(),
            if on { "on" } else { "off" },
            if f.requires_restart() { "yes" } else { "no" },
            f.description()
        );
    }
    0
}

pub fn experiments_set(name: &str, enable: bool) -> i32 {
    set_at(&crate::config::config_path(), name, enable)
}

fn set_at(path: &Path, name: &str, enable: bool) -> i32 {
    let verb = if enable { "enable" } else { "disable" };
    let Some(feature) = ReleaseFeature::from_key(name) else {
        let valid: Vec<&str> = ReleaseFeature::ALL.iter().map(|f| f.key()).collect();
        eprintln!(
            "error: experiments {verb}: unknown experiment `{name}`. Valid names: {}",
            valid.join(", ")
        );
        return 1;
    };
    let result = crate::config::edit::edit_config_file(path, |doc| {
        if enable {
            crate::config::edit::set_value(doc, &[TABLE, name], true.into())
        } else {
            crate::config::edit::remove_value(doc, &[TABLE, name]);
            Ok(())
        }
    });
    if let Err(e) = result {
        eprintln!("error: experiments {verb} {name}: {e}");
        return 1;
    }
    log::info!("experiments: {verb} name={name} path={}", path.display());
    println!("{name}: {}", if enable { "on" } else { "off" });
    if feature.requires_restart() {
        println!("restart required: run `plexi host restart`");
    }
    println!("Rerun `plexi skill install` and `plexi completions` to refresh their snapshots.");
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = "# my config\nfont_size = 14.0 # big\n\n[ai.local]\n# local backend\nurl = \"http://127.0.0.1:3456\"\n\n[experiments]\n# keep me\nledger = true\n";

    fn fixture() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, FIXTURE).unwrap();
        (dir, path)
    }

    #[test]
    fn enable_preserves_comments_and_unrelated_keys() {
        let (_d, path) = fixture();
        assert_eq!(set_at(&path, "assistant", true), 0);
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            text,
            FIXTURE.replace("ledger = true\n", "ledger = true\nassistant = true\n")
        );
    }

    #[test]
    fn disable_removes_only_its_key() {
        let (_d, path) = fixture();
        assert_eq!(set_at(&path, "ledger", false), 0);
        let text = std::fs::read_to_string(&path).unwrap();
        // A comment directly above a key belongs to that key and goes with it.
        assert_eq!(text, FIXTURE.replace("# keep me\nledger = true\n", ""));
    }

    #[test]
    fn disable_of_an_absent_key_is_a_noop() {
        let (_d, path) = fixture();
        assert_eq!(set_at(&path, "relay", false), 0);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), FIXTURE);
    }

    #[test]
    fn enable_unknown_name_fails_and_writes_nothing() {
        let (_d, path) = fixture();
        assert_eq!(set_at(&path, "assistent", true), 1);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), FIXTURE);
    }

    #[test]
    fn enable_creates_the_table_in_a_missing_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        assert_eq!(set_at(&path, "relay", true), 0);
        let enabled = crate::config::experiments_from_path(&path).unwrap();
        assert!(enabled.contains(&ReleaseFeature::Relay));
        assert_eq!(enabled.len(), 1);
    }

    #[test]
    fn list_reports_state_and_restart_flag() {
        let (_d, path) = fixture();
        let rows = rows(&path).unwrap();
        assert_eq!(rows.len(), ReleaseFeature::ALL.len());
        let ledger = rows
            .iter()
            .find(|(f, _)| *f == ReleaseFeature::Ledger)
            .unwrap();
        assert!(ledger.1);
        assert!(!ReleaseFeature::Ledger.requires_restart());
        assert!(ReleaseFeature::Relay.requires_restart());
        assert!(rows.iter().filter(|(_, on)| *on).count() == 1);
    }
}

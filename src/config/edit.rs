//! Comment-preserving edits to a `config.toml`, shared by every command that
//! writes config (`config set`, `experiments enable|disable`).
//!
//! Edits go through `toml_edit`, so comments, ordering and keys the caller does
//! not own survive. Callers must not round-trip through a `toml::Value`.

use std::path::Path;

/// Read `path` (seeding from the default template when absent), apply `edit`,
/// and write the result back. Errors name the failing operation and path.
pub fn edit_config_file(
    path: &Path,
    edit: impl FnOnce(&mut toml_edit::DocumentMut) -> Result<(), String>,
) -> Result<(), String> {
    let raw = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => super::CONFIG_TEMPLATE.to_string(),
        Err(e) => return Err(format!("could not read {}: {e}", path.display())),
    };
    let mut doc: toml_edit::DocumentMut = raw
        .parse()
        .map_err(|e| format!("could not parse {}: {e}", path.display()))?;
    edit(&mut doc)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("could not create config dir {}: {e}", parent.display()))?;
    }
    std::fs::write(path, doc.to_string())
        .map_err(|e| format!("could not write {}: {e}", path.display()))
}

/// Navigate/create nested tables in a toml_edit document and set the leaf value.
pub fn set_value(
    doc: &mut toml_edit::DocumentMut,
    segments: &[&str],
    value: toml_edit::Value,
) -> Result<(), String> {
    let (leaf, parents) = segments
        .split_last()
        .ok_or_else(|| "empty key".to_string())?;
    let mut table: &mut toml_edit::Table = doc.as_table_mut();
    for seg in parents {
        if !table.contains_key(seg) {
            table.insert(seg, toml_edit::Item::Table(toml_edit::Table::new()));
        }
        table = table
            .get_mut(seg)
            .and_then(|i| i.as_table_mut())
            .ok_or_else(|| format!("{seg} is not a table"))?;
    }
    table.insert(leaf, toml_edit::Item::Value(value));
    Ok(())
}

/// Remove the leaf at `segments`; absent keys are a no-op. Returns whether a
/// key was removed.
pub fn remove_value(doc: &mut toml_edit::DocumentMut, segments: &[&str]) -> bool {
    let Some((leaf, parents)) = segments.split_last() else {
        return false;
    };
    let mut table: &mut toml_edit::Table = doc.as_table_mut();
    for seg in parents {
        match table.get_mut(seg).and_then(|i| i.as_table_mut()) {
            Some(t) => table = t,
            None => return false,
        }
    }
    table.remove(leaf).is_some()
}

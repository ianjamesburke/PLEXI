//! Old copies of a channel's Mac app bundle outside the managed installation.
//!
//! Discovery is pure filesystem inspection so it runs on any platform under
//! test; only the installer and the macOS host call it for real. Moving a
//! stray is always a rename into the Trash, never a delete.
use crate::{Error, IoContext, Result, transaction};
use std::{
    fs,
    path::{Path, PathBuf},
};

/// `CFBundleIdentifier` the package builder assigns to a channel's bundle.
pub fn bundle_identifier(channel: &str) -> String {
    match channel {
        "stable" | "main" => "com.ianjamesburke.plexi".into(),
        other => format!("com.ianjamesburke.plexi-{other}"),
    }
}

/// Directories a user or an older installer may have left a copy in.
/// `~/.Trash` is deliberately absent.
pub fn candidate_dirs(home: &Path) -> Vec<PathBuf> {
    vec![
        PathBuf::from("/Applications"),
        home.join("Applications"),
        home.join("Downloads"),
        home.join("Desktop"),
    ]
}

/// The identifier recorded in a bundle's `Contents/Info.plist`, when readable.
pub fn read_bundle_identifier(bundle: &Path) -> Option<String> {
    let plist = bundle.join("Contents/Info.plist");
    let bytes = fs::read(&plist).ok()?;
    if bytes.starts_with(b"bplist") {
        return binary_plist_identifier(&plist);
    }
    let text = String::from_utf8_lossy(&bytes);
    let after_key = text.split("<key>CFBundleIdentifier</key>").nth(1)?;
    let value = after_key.trim_start().strip_prefix("<string>")?;
    Some(value.split("</string>").next()?.trim().to_string())
}

#[cfg(target_os = "macos")]
fn binary_plist_identifier(plist: &Path) -> Option<String> {
    let output = std::process::Command::new("/usr/bin/plutil")
        .args(["-extract", "CFBundleIdentifier", "raw", "-o", "-"])
        .arg(plist)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}
#[cfg(not(target_os = "macos"))]
fn binary_plist_identifier(_plist: &Path) -> Option<String> {
    None
}

/// Whether `name` looks like a copy of the channel's bundle: the exact name,
/// or Finder/browser duplicate names such as `Plexi Beta 2.app`.
fn name_matches(channel: &str, name: &str) -> bool {
    let display = transaction::display_name(channel);
    let Some(stem) = name.strip_suffix(".app") else {
        return false;
    };
    stem == display
        || stem
            .strip_prefix(&display)
            .is_some_and(|rest| rest.starts_with([' ', '-', '(']))
}

/// App bundles of `channel` in `dirs` that are not the managed installation.
///
/// A bundle counts when its identifier is the channel's, or when its
/// identifier is unreadable and its name matches. A readable foreign
/// identifier always wins over the name, so stable's `Plexi*.app` pattern
/// never claims `Plexi Beta.app`. Paths under `exclude` (the managed root and
/// owned integrations) and the user's Trash are skipped; aliases are reported
/// once.
pub fn find_strays(
    channel: &str,
    home: &Path,
    dirs: &[PathBuf],
    exclude: &[PathBuf],
) -> Vec<PathBuf> {
    let identifier = bundle_identifier(channel);
    let resolve = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    let excluded: Vec<PathBuf> = exclude
        .iter()
        .map(|p| resolve(p))
        .chain([resolve(&home.join(".Trash"))])
        .collect();
    let mut seen = Vec::new();
    let mut strays = Vec::new();
    for dir in dirs {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        let mut paths: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
        paths.sort();
        for path in paths {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !name.ends_with(".app") || !path.is_dir() {
                continue;
            }
            let ours = match read_bundle_identifier(&path) {
                Some(found) => found == identifier,
                None => name_matches(channel, name),
            };
            let real = resolve(&path);
            if !ours
                || excluded
                    .iter()
                    .any(|e| real.starts_with(e) || path.starts_with(e))
                || seen.contains(&real)
            {
                continue;
            }
            seen.push(real);
            strays.push(path);
        }
    }
    strays
}

/// Strays for `channel` in `dirs`, excluding everything the receipt owns.
pub fn find_for_installation(
    channel: &str,
    home: &Path,
    receipt: Option<&transaction::Receipt>,
    dirs: &[PathBuf],
) -> Vec<PathBuf> {
    let exclude: Vec<PathBuf> = receipt
        .map(|r| {
            std::iter::once(r.root.clone())
                .chain(r.integrations.iter().map(|o| o.path.clone()))
                .collect()
        })
        .unwrap_or_default();
    find_strays(channel, home, dirs, &exclude)
}

/// First free name for `name` in `trash`: `Plexi Beta.app`, then
/// `Plexi Beta 2.app`, `Plexi Beta 3.app`, ...
pub fn trash_destination(trash: &Path, name: &str) -> PathBuf {
    let (stem, extension) = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem, format!(".{ext}")),
        _ => (name, String::new()),
    };
    let mut candidate = trash.join(name);
    let mut n = 2;
    while candidate.exists() || candidate.is_symlink() {
        candidate = trash.join(format!("{stem} {n}{extension}"));
        n += 1;
    }
    candidate
}

/// Move `path` into `~/.Trash` by rename. Fails rather than copying or
/// deleting when the rename is not possible (another volume, no permission).
pub fn move_to_trash(path: &Path, home: &Path) -> Result<PathBuf> {
    let trash = home.join(".Trash");
    fs::create_dir_all(&trash).context("create Trash directory")?;
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| Error::Invalid(format!("cannot name {}", path.display())))?;
    let destination = trash_destination(&trash, name);
    fs::rename(path, &destination).context(format!(
        "move {} to {}",
        path.display(),
        destination.display()
    ))?;
    log::info!(
        "distribution: moved stray bundle {} to {}",
        path.display(),
        destination.display()
    );
    Ok(destination)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bundle(dir: &Path, name: &str, identifier: Option<&str>) -> PathBuf {
        let path = dir.join(name);
        fs::create_dir_all(path.join("Contents")).unwrap();
        if let Some(id) = identifier {
            fs::write(
                path.join("Contents/Info.plist"),
                format!("<plist><dict><key>CFBundleName</key><string>x</string><key>CFBundleIdentifier</key>\n  <string>{id}</string></dict></plist>"),
            )
            .unwrap();
        }
        path
    }

    #[test]
    fn strays_match_identifier_or_name_and_skip_trash_and_managed() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        let system = home.join("system-Applications");
        let user = home.join("Applications");
        let downloads = home.join("Downloads");
        let root = home.join(".local/share/plexi/beta");
        let beta = "com.ianjamesburke.plexi-beta";
        let old_system = bundle(&system, "Plexi Beta.app", Some(beta));
        let managed = bundle(&user, "Plexi Beta.app", Some(beta));
        let download = bundle(&downloads, "Plexi Beta 2.app", None);
        let renamed = bundle(&downloads, "Old Copy.app", Some(beta));
        bundle(
            &downloads,
            "Plexi Beta notes.txt.app",
            Some("com.example.other"),
        );
        bundle(
            &downloads,
            "Plexi Alpha.app",
            Some("com.ianjamesburke.plexi-alpha"),
        );
        bundle(&home.join(".Trash"), "Plexi Beta.app", Some(beta));
        bundle(&root.join("generations/abc"), "Plexi Beta.app", Some(beta));
        let dirs = vec![
            system.clone(),
            user.clone(),
            downloads.clone(),
            home.join("Desktop"),
            home.join(".Trash"),
            root.join("generations/abc"),
        ];
        let strays = find_strays("beta", home, &dirs, &[root.clone(), managed.clone()]);
        assert_eq!(strays, vec![old_system, renamed, download]);
    }

    #[test]
    fn stable_name_pattern_never_claims_other_channels() {
        let home = tempfile::tempdir().unwrap();
        let apps = home.path().join("Applications");
        bundle(
            &apps,
            "Plexi Beta.app",
            Some("com.ianjamesburke.plexi-beta"),
        );
        let stable = bundle(&apps, "Plexi.app", Some("com.ianjamesburke.plexi"));
        let unlabeled = bundle(&apps, "Plexi (1).app", None);
        bundle(&apps, "Plexify.app", None);
        assert_eq!(
            find_strays("stable", home.path(), &[apps], &[]),
            vec![unlabeled, stable]
        );
    }

    #[cfg(unix)]
    #[test]
    fn aliased_directories_report_a_bundle_once() {
        let home = tempfile::tempdir().unwrap();
        let apps = home.path().join("Applications");
        let only = bundle(
            &apps,
            "Plexi Beta.app",
            Some("com.ianjamesburke.plexi-beta"),
        );
        std::os::unix::fs::symlink(&apps, home.path().join("Apps")).unwrap();
        let dirs = vec![apps, home.path().join("Apps")];
        assert_eq!(find_strays("beta", home.path(), &dirs, &[]), vec![only]);
    }

    #[test]
    fn trash_moves_rename_with_collision_suffix() {
        let home = tempfile::tempdir().unwrap();
        let trash = home.path().join(".Trash");
        bundle(&trash, "Plexi Beta.app", None);
        bundle(&trash, "Plexi Beta 2.app", None);
        let stray = bundle(&home.path().join("Downloads"), "Plexi Beta.app", None);
        fs::write(stray.join("Contents/marker"), b"old").unwrap();
        let moved = move_to_trash(&stray, home.path()).unwrap();
        assert_eq!(moved, trash.join("Plexi Beta 3.app"));
        assert!(!stray.exists());
        assert_eq!(fs::read(moved.join("Contents/marker")).unwrap(), b"old");
        assert_eq!(trash_destination(&trash, "README"), trash.join("README"));
    }
}

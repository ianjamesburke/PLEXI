//! Test fixture: a real compiled CLI that records each launch in a marker file.

use std::collections::HashMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

const SOURCE: &str = include_str!("fixture_cli.rs");

struct Cache {
    _dir: tempfile::TempDir,
    built: HashMap<String, PathBuf>,
}

fn cache() -> &'static Mutex<Option<Cache>> {
    static CACHE: OnceLock<Mutex<Option<Cache>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

/// Install a fixture build with `version` baked in at `dest` (mode 0755), and
/// return `dest`. Compilation happens once per version per test process.
pub fn install(version: &str, dest: &Path) -> PathBuf {
    let mut guard = cache().lock().unwrap_or_else(|e| e.into_inner());
    let state = guard.get_or_insert_with(|| Cache {
        _dir: tempfile::tempdir().expect("fixture cache dir"),
        built: HashMap::new(),
    });
    let built = match state.built.get(version) {
        Some(path) => path.clone(),
        None => {
            let src = state._dir.path().join("fixture_cli.rs");
            std::fs::write(&src, SOURCE).expect("write fixture source");
            let out = state._dir.path().join(format!("fixture-{version}"));
            let output = std::process::Command::new("rustc")
                .args(["--edition", "2021", "-C", "opt-level=0", "-o"])
                .arg(&out)
                .arg(&src)
                .env("FIXTURE_VERSION", version)
                .output()
                .expect("rustc must be on PATH to build the fixture CLI");
            assert!(
                output.status.success(),
                "fixture CLI failed to compile: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            state.built.insert(version.to_string(), out.clone());
            out
        }
    };
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).expect("fixture dest dir");
    }
    // Write through a temporary name and rename, as a package manager would.
    let staging = dest.with_extension("tmp-install");
    std::fs::copy(&built, &staging).expect("copy fixture");
    std::fs::set_permissions(&staging, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::rename(&staging, dest).expect("install fixture");
    dest.to_path_buf()
}

/// Launch lines the fixture recorded in `workspace`, one per start.
pub fn launches(workspace: &Path) -> Vec<String> {
    std::fs::read_to_string(workspace.join(".fixture-launched"))
        .map(|text| text.lines().map(str::to_string).collect())
        .unwrap_or_default()
}

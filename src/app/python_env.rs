use plexi_distribution::bounded::{Helper, RunError};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub(crate) const PYTHON_APP_VENV_VERSION: &str = "3.12";

#[derive(Debug, Clone)]
pub(crate) struct PythonRuntime {
    pub(crate) executable: OsString,
    pub(crate) label: String,
    pub(crate) version: String,
}

/// Limits for the uv and Python helpers. `uv venv` may download Python and
/// `uv pip install` may build packages, so they get minutes; the version
/// probes answer at once when healthy.
struct UvLimits {
    probe: Duration,
    venv: Duration,
    install: Duration,
}

const UV_LIMITS: UvLimits = UvLimits {
    probe: Duration::from_secs(30),
    venv: Duration::from_secs(600),
    install: Duration::from_secs(900),
};

/// Why a plugin's Python environment could not be prepared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum VenvError {
    /// uv or Python is missing, too old, or refused: the plugin is installed
    /// but will not launch until its environment is fixed.
    Unavailable(String),
    /// A uv or Python helper timed out, could not be stopped, or its output
    /// was cut: the environment is in an unknown state and may still be
    /// changing.
    Fatal(String),
}

impl std::fmt::Display for VenvError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(message) | Self::Fatal(message) => f.write_str(message),
        }
    }
}

impl From<RunError> for VenvError {
    fn from(error: RunError) -> Self {
        if error.is_tool_failure() {
            Self::Unavailable(error.to_string())
        } else {
            Self::Fatal(error.to_string())
        }
    }
}

pub(crate) fn ensure_plugin_venv(
    plugin_id: &str,
    plugin_dir: &Path,
    dependencies: &[String],
) -> Result<PathBuf, VenvError> {
    ensure_plugin_venv_using(
        OsStr::new("uv"),
        plugin_id,
        plugin_dir,
        dependencies,
        &UV_LIMITS,
    )
}

fn ensure_plugin_venv_using(
    uv: &OsStr,
    plugin_id: &str,
    plugin_dir: &Path,
    dependencies: &[String],
    limits: &UvLimits,
) -> Result<PathBuf, VenvError> {
    let venv_path = plugin_dir.join(".venv");
    let python = venv_path.join("bin").join("python");

    if python.exists() {
        let version = check_python_version(&python, limits.probe)?;
        log::info!(
            "python_env[{plugin_id}]: using existing venv {} ({version})",
            venv_path.display()
        );
        install_dependencies(uv, plugin_id, &python, dependencies, limits)?;
        return Ok(python);
    }

    if venv_path.exists() {
        return Err(VenvError::Unavailable(format!(
            "{} exists but {} is missing; remove the broken venv and retry",
            venv_path.display(),
            python.display()
        )));
    }

    Helper::new(uv)
        .arg("--version")
        .run("`uv --version`", limits.probe)
        .map_err(|e| {
            let message = format!(
                "`uv` is required to create a Python {PYTHON_APP_VENV_VERSION} plugin venv but did not run: {e}"
            );
            match VenvError::from(e) {
                VenvError::Unavailable(_) => VenvError::Unavailable(message),
                VenvError::Fatal(_) => VenvError::Fatal(message),
            }
        })?;

    Helper::new(uv)
        .args(["venv", "--python", PYTHON_APP_VENV_VERSION])
        .arg(&venv_path)
        .run(
            &format!(
                "`uv venv --python {PYTHON_APP_VENV_VERSION} {}`",
                venv_path.display()
            ),
            limits.venv,
        )?;

    let version = check_python_version(&python, limits.probe)?;
    log::info!(
        "python_env[{plugin_id}]: created venv at {} ({version})",
        venv_path.display()
    );
    install_dependencies(uv, plugin_id, &python, dependencies, limits)?;
    Ok(python)
}

pub(crate) fn resolve_python_runtime(
    plugin_id: &str,
    entry_path: &Path,
    ensure_venv: bool,
    dependencies: &[String],
) -> Result<PythonRuntime, String> {
    let plugin_dir = entry_path.parent().ok_or_else(|| {
        format!(
            "could not determine plugin directory for Python entry {}",
            entry_path.display()
        )
    })?;

    if ensure_venv {
        let python =
            ensure_plugin_venv(plugin_id, plugin_dir, dependencies).map_err(|e| e.to_string())?;
        return runtime_from_path("venv", python);
    }

    let venv_python = plugin_dir.join(".venv").join("bin").join("python");
    if venv_python.exists() {
        return runtime_from_path("venv", venv_python);
    }

    if let Some(bundle_python) = bundled_python() {
        return runtime_from_path("bundled", bundle_python);
    }

    runtime_from_path("system", PathBuf::from("python3"))
}

pub(crate) fn bundled_python_bin_dir() -> Option<PathBuf> {
    bundle_contents_dir().map(|c| {
        c.join("Resources")
            .join("assets")
            .join("python")
            .join("bin")
    })
}

fn runtime_from_path(source: &str, python: PathBuf) -> Result<PythonRuntime, String> {
    let version = check_python_version(&python, UV_LIMITS.probe).map_err(|e| e.to_string())?;
    Ok(PythonRuntime {
        executable: OsString::from(&python),
        label: format!("{source}: {}", python.display()),
        version,
    })
}

fn install_dependencies(
    uv: &OsStr,
    plugin_id: &str,
    python: &Path,
    dependencies: &[String],
    limits: &UvLimits,
) -> Result<(), VenvError> {
    if dependencies.is_empty() {
        return Ok(());
    }
    Helper::new(uv)
        .args(["pip", "install", "--python"])
        .arg(python)
        .args(dependencies)
        .run(
            &format!("`uv pip install --python {}`", python.display()),
            limits.install,
        )?;
    log::info!(
        "python_env[{plugin_id}]: installed {} Python dep(s): {}",
        dependencies.len(),
        dependencies.join(", ")
    );
    Ok(())
}

fn check_python_version(python: &Path, limit: Duration) -> Result<String, VenvError> {
    let exited = Helper::new(python)
        .arg("-c")
        .arg(
            "import sys; print(f'{sys.version_info.major}.{sys.version_info.minor}.{sys.version_info.micro}'); sys.exit(0 if sys.version_info >= (3, 11) else 42)",
        )
        .run_any_exit(&format!("Python interpreter {}", python.display()), limit)?;
    let version = String::from_utf8_lossy(&exited.output.stdout)
        .trim()
        .to_string();
    if exited.status.success() {
        return Ok(version);
    }
    let rendered_version = if version.is_empty() {
        "unknown".to_string()
    } else {
        version
    };
    Err(VenvError::Unavailable(format!(
        "Python interpreter {} is {rendered_version}; Plexi SDK v3 requires Python >= 3.11",
        python.display()
    )))
}

fn bundled_python() -> Option<PathBuf> {
    bundled_python_bin_dir()
        .map(|bin| bin.join("python3"))
        .filter(|python| python.exists())
}

fn bundle_contents_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().and_then(|p| p.parent()).map(Path::to_path_buf))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Stint 1610: a hung uv is stopped at its limit and the venv step
    /// fails instead of holding the install.
    #[cfg(unix)]
    #[test]
    fn a_hung_uv_fails_the_venv_inside_its_limit() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let uv = dir.path().join("uv");
        std::fs::write(&uv, "#!/bin/sh\nexec sleep 600\n").unwrap();
        std::fs::set_permissions(&uv, std::fs::Permissions::from_mode(0o755)).unwrap();
        let plugin = dir.path().join("plugin");
        std::fs::create_dir(&plugin).unwrap();
        let limits = UvLimits {
            probe: Duration::from_millis(300),
            venv: Duration::from_millis(300),
            install: Duration::from_millis(300),
        };
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(ensure_plugin_venv_using(
                uv.as_os_str(),
                "test",
                &plugin,
                &[],
                &limits,
            ));
        });
        let result = rx
            .recv_timeout(Duration::from_secs(20))
            .expect("ensure_plugin_venv returned inside its limit");
        let error = result.unwrap_err();
        assert!(
            matches!(&error, VenvError::Fatal(message) if message.contains("did not finish within")),
            "a hung uv must be fatal: {error:?}"
        );
    }
}

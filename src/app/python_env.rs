use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

pub(crate) const PYTHON_APP_VENV_VERSION: &str = "3.12";

#[derive(Debug, Clone)]
pub(crate) struct PythonRuntime {
    pub(crate) executable: OsString,
    pub(crate) label: String,
    pub(crate) version: String,
}

pub(crate) fn ensure_app_venv(
    app_id: &str,
    app_dir: &Path,
    dependencies: &[String],
) -> Result<PathBuf, String> {
    let venv_path = app_dir.join(".venv");
    let python = venv_path.join("bin").join("python");

    if python.exists() {
        let version = check_python_version(&python)?;
        log::info!(
            "python_env[{app_id}]: using existing venv {} ({version})",
            venv_path.display()
        );
        install_dependencies(app_id, &python, dependencies)?;
        return Ok(python);
    }

    if venv_path.exists() {
        return Err(format!(
            "{} exists but {} is missing; remove the broken venv and retry",
            venv_path.display(),
            python.display()
        ));
    }

    let uv = Command::new("uv").arg("--version").output().map_err(|e| {
        format!(
            "`uv` is required to create a Python {PYTHON_APP_VENV_VERSION} app venv but could not be started: {e}"
        )
    })?;
    if !uv.status.success() {
        return Err(format!(
            "`uv --version` failed while preparing Python app venv: {}",
            command_output_summary(&uv)
        ));
    }

    let output = Command::new("uv")
        .args(["venv", "--python", PYTHON_APP_VENV_VERSION])
        .arg(&venv_path)
        .output()
        .map_err(|e| format!("failed to run `uv venv`: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "`uv venv --python {PYTHON_APP_VENV_VERSION} {}` failed: {}",
            venv_path.display(),
            command_output_summary(&output)
        ));
    }

    let version = check_python_version(&python)?;
    log::info!(
        "python_env[{app_id}]: created venv at {} ({version})",
        venv_path.display()
    );
    install_dependencies(app_id, &python, dependencies)?;
    Ok(python)
}

pub(crate) fn resolve_python_runtime(
    app_id: &str,
    entry_path: &Path,
    ensure_venv: bool,
    dependencies: &[String],
) -> Result<PythonRuntime, String> {
    let app_dir = entry_path.parent().ok_or_else(|| {
        format!(
            "could not determine app directory for Python entry {}",
            entry_path.display()
        )
    })?;

    if ensure_venv {
        let python = ensure_app_venv(app_id, app_dir, dependencies)?;
        return runtime_from_path("venv", python);
    }

    let venv_python = app_dir.join(".venv").join("bin").join("python");
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
    let version = check_python_version(&python)?;
    Ok(PythonRuntime {
        executable: OsString::from(&python),
        label: format!("{source}: {}", python.display()),
        version,
    })
}

fn install_dependencies(
    app_id: &str,
    python: &Path,
    dependencies: &[String],
) -> Result<(), String> {
    if dependencies.is_empty() {
        return Ok(());
    }

    let output = Command::new("uv")
        .args(["pip", "install", "--python"])
        .arg(python)
        .args(dependencies)
        .output()
        .map_err(|e| format!("failed to run `uv pip install`: {e}"))?;
    if output.status.success() {
        log::info!(
            "python_env[{app_id}]: installed {} Python dep(s): {}",
            dependencies.len(),
            dependencies.join(", ")
        );
        Ok(())
    } else {
        Err(format!(
            "`uv pip install --python {}` failed: {}",
            python.display(),
            command_output_summary(&output)
        ))
    }
}

fn check_python_version(python: &Path) -> Result<String, String> {
    let output = Command::new(python)
        .arg("-c")
        .arg(
            "import sys; print(f'{sys.version_info.major}.{sys.version_info.minor}.{sys.version_info.micro}'); sys.exit(0 if sys.version_info >= (3, 11) else 42)",
        )
        .output()
        .map_err(|e| format!("could not run Python interpreter {}: {e}", python.display()))?;
    let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if output.status.success() {
        return Ok(version);
    }
    let rendered_version = if version.is_empty() {
        "unknown".to_string()
    } else {
        version
    };
    Err(format!(
        "Python interpreter {} is {rendered_version}; Plexi SDK v3 requires Python >= 3.11",
        python.display()
    ))
}

fn command_output_summary(output: &std::process::Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    match (stdout.is_empty(), stderr.is_empty()) {
        (true, true) => format!("exit {}", output.status.code().unwrap_or(-1)),
        (false, true) => stdout,
        (true, false) => stderr,
        (false, false) => format!("{stdout}\n{stderr}"),
    }
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

    #[test]
    fn command_output_summary_prefers_stderr_when_stdout_empty() {
        let output = Command::new("sh")
            .args(["-c", "echo nope >&2; exit 3"])
            .output()
            .unwrap();

        assert_eq!(command_output_summary(&output), "nope");
    }
}

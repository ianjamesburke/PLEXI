pub fn validate_cli(path: &str) -> i32 {
    let app_dir = std::path::Path::new(path);
    if !app_dir.exists() {
        eprintln!("validate: path does not exist: {path}");
        return 1;
    }
    // A .plexipkg file gets the full fail-closed package validation.
    if app_dir.is_file() {
        if app_dir.extension().and_then(|e| e.to_str())
            == Some(crate::app::package::PACKAGE_EXTENSION)
        {
            return validate_package_cli(app_dir);
        }
        eprintln!(
            "validate: path is not a directory or .{} file: {path}",
            crate::app::package::PACKAGE_EXTENSION
        );
        return 1;
    }
    if !app_dir.is_dir() {
        eprintln!("validate: path is not a directory: {path}");
        return 1;
    }

    let manifest_path = app_dir.join("manifest.toml");
    if !manifest_path.exists() {
        eprintln!("✗ manifest.toml not found in {path}");
        return 1;
    }

    let raw = match std::fs::read_to_string(&manifest_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("✗ cannot read manifest.toml: {e}");
            return 1;
        }
    };

    let toml_val: toml::Value = match raw.parse() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("✗ manifest.toml parse error: {e}");
            return 1;
        }
    };

    let mut errors: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    // Required fields
    let app_section = toml_val.get("app");
    let required_fields = ["id", "name", "version", "entry"];
    for field in &required_fields {
        let val = app_section
            .and_then(|a| a.get(field))
            .and_then(|v| v.as_str());
        if val.is_none() || val == Some("") {
            errors.push(format!("  [app].{field} is missing or empty"));
        }
    }

    // description is recommended but not required
    let has_desc = app_section
        .and_then(|a| a.get("description"))
        .and_then(|v| v.as_str())
        .map(|s| !s.is_empty())
        .unwrap_or(false);
    if !has_desc {
        warnings.push("  [app].description is missing (recommended)".to_string());
    }

    // Check entry file
    let entry = app_section
        .and_then(|a| a.get("entry"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if !entry.is_empty() {
        let entry_path = app_dir.join(entry);
        if !entry_path.exists() {
            errors.push(format!("  entry file not found: {}", entry_path.display()));
        } else if entry.ends_with(".py") {
            // Python syntax check via AST parse (no import, no SDK needed)
            let py_check = std::process::Command::new("python3")
                .arg("-c")
                .arg("import ast, sys; ast.parse(open(sys.argv[1]).read())")
                .arg(&entry_path)
                .output();
            match py_check {
                Ok(out) if out.status.success() => {}
                Ok(out) => {
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    errors.push(format!(
                        "  Python syntax error in {entry}: {}",
                        stderr.trim()
                    ));
                }
                Err(e) => {
                    warnings.push(format!("  python3 not found — skipping syntax check: {e}"));
                }
            }
        }
    }

    // capabilities validation — checked against the real Capability enum.
    // Unknown capability strings are hard errors (fail closed): the host would
    // refuse to install/launch the app, so validate must too.
    if let Some(caps) = app_section
        .and_then(|a| a.get("capabilities"))
        .and_then(|c| c.get("capabilities"))
        .and_then(|v| v.as_array())
    {
        use std::convert::TryFrom;
        for cap in caps {
            if let Some(s) = cap.as_str() {
                if crate::app::permissions::Capability::try_from(s).is_err() {
                    errors.push(format!(
                        "  unknown capability: {s:?} — valid capabilities: {}",
                        crate::app::permissions::Capability::all_str_values().join(", ")
                    ));
                }
            }
        }
    }

    // Print results
    let id = app_section
        .and_then(|a| a.get("id"))
        .and_then(|v| v.as_str())
        .unwrap_or(path);

    if errors.is_empty() && warnings.is_empty() {
        println!("✓ {id} — all checks passed");
        log::info!("validate: {} passed", id);
        return 0;
    }

    if !errors.is_empty() {
        println!("✗ {id} — {} error(s):", errors.len());
        for e in &errors {
            println!("{e}");
        }
    }
    if !warnings.is_empty() {
        println!("⚠ {id} — {} warning(s):", warnings.len());
        for w in &warnings {
            println!("{w}");
        }
    }
    log::warn!(
        "validate: {} — {} errors, {} warnings",
        id,
        errors.len(),
        warnings.len()
    );

    if errors.is_empty() {
        0
    } else {
        1
    }
}

/// Validate a `.plexipkg` file and print the report (fail-closed; stint 0015).
fn validate_package_cli(file: &std::path::Path) -> i32 {
    log::info!("validate: package file {}", file.display());
    match crate::app::package::validate_package(file) {
        Ok(report) => {
            println!("✓ {} — package valid", report.id);
            println!("  name:         {}", report.name);
            println!("  version:      {}", report.version);
            println!("  runtime:      {}", report.runtime.as_str());
            println!("  entry:        {}", report.entry);
            println!(
                "  capabilities: {}",
                if report.capabilities.is_empty() {
                    "(none)".to_string()
                } else {
                    report
                        .capabilities
                        .iter()
                        .map(|c| c.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                }
            );
            println!(
                "  files:        {} ({} bytes)",
                report.file_count, report.total_size
            );
            0
        }
        Err(e) => {
            eprintln!("✗ {} — package validation failed:", file.display());
            eprintln!("  {e}");
            1
        }
    }
}

/// Resolve `path` argument (canonicalize if given, else use CWD).
pub(super) fn resolve_path(path: Option<&str>) -> Result<std::path::PathBuf, String> {
    match path {
        Some(p) => std::fs::canonicalize(p)
            .map_err(|e| format!("error: could not resolve path {p:?}: {e}")),
        None => std::env::current_dir()
            .map_err(|e| format!("error: could not get current directory: {e}")),
    }
}

#[cfg(test)]
mod validate_tests {
    use tempfile::TempDir;

    fn write_valid_manifest(dir: &std::path::Path) {
        std::fs::write(
            dir.join("manifest.toml"),
            "schema_version = 1\n\n\
             [app]\n\
             id = \"test-app\"\n\
             type = \"app\"\n\
             name = \"Test App\"\n\
             entry = \"main.py\"\n\
             version = \"0.1.0\"\n\
             description = \"A test app\"\n",
        )
        .unwrap();
        std::fs::write(dir.join("main.py"), "# stub\n").unwrap();
    }

    /// `plexi app validate <path>` must succeed from a bare directory with no
    /// `.plexi/` workspace ancestor. This is the core invariant: workspace is
    /// irrelevant for stateless path operations.
    #[test]
    fn validate_passes_without_workspace() {
        let dir = TempDir::new().unwrap();
        write_valid_manifest(dir.path());
        let path = dir.path().to_string_lossy().to_string();
        // The temp dir has no .plexi/ ancestor — must still return 0.
        let code = super::validate_cli(&path);
        assert_eq!(code, 0, "validate_cli must succeed without a workspace");
    }

    #[test]
    fn validate_fails_on_missing_manifest() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().to_string_lossy().to_string();
        let code = super::validate_cli(&path);
        assert_eq!(code, 1, "missing manifest.toml must return 1");
    }

    #[test]
    fn validate_fails_on_unknown_capability() {
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("manifest.toml"),
            "schema_version = 1\n\n\
             [app]\n\
             id = \"test-app\"\n\
             type = \"app\"\n\
             name = \"Test App\"\n\
             entry = \"main.py\"\n\
             version = \"0.1.0\"\n\
             description = \"A test app\"\n\n\
             [app.capabilities]\n\
             capabilities = [\"net.dns\"]\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("main.py"), "# stub\n").unwrap();
        let path = dir.path().to_string_lossy().to_string();
        let code = super::validate_cli(&path);
        assert_eq!(code, 1, "unknown capability must be a hard error");
    }

    #[test]
    fn validate_routes_plexipkg_file_to_package_validation() {
        let dir = TempDir::new().unwrap();
        let app_dir = dir.path().join("app");
        std::fs::create_dir(&app_dir).unwrap();
        std::fs::write(
            app_dir.join("manifest.toml"),
            "schema_version = 1\n\n\
             [app]\n\
             id = \"pkg-cli-test\"\n\
             type = \"app\"\n\
             name = \"Pkg CLI Test\"\n\
             entry = \"main.py\"\n\
             version = \"0.1.0\"\n\
             description = \"A test app\"\n",
        )
        .unwrap();
        std::fs::write(app_dir.join("main.py"), "# stub\n").unwrap();
        let pkg = dir.path().join("pkg-cli-test-0.1.0.plexipkg");
        crate::app::package::build_package(&app_dir, Some(&pkg)).unwrap();

        let code = super::validate_cli(&pkg.to_string_lossy());
        assert_eq!(code, 0, "a valid .plexipkg must pass validate_cli");

        // A non-package file must be rejected, not treated as a directory.
        let bogus = dir.path().join("not-a-package.txt");
        std::fs::write(&bogus, "x").unwrap();
        let code = super::validate_cli(&bogus.to_string_lossy());
        assert_eq!(code, 1, "non-.plexipkg file must fail validate_cli");
    }

    #[test]
    fn validate_fails_on_missing_required_field() {
        let dir = TempDir::new().unwrap();
        // id is missing
        std::fs::write(
            dir.path().join("manifest.toml"),
            "schema_version = 1\n\n\
             [app]\n\
             type = \"app\"\n\
             name = \"Test App\"\n\
             entry = \"main.py\"\n\
             version = \"0.1.0\"\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("main.py"), "# stub\n").unwrap();
        let path = dir.path().to_string_lossy().to_string();
        let code = super::validate_cli(&path);
        assert_eq!(code, 1, "missing [app].id must return 1");
    }
}

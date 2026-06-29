use crate::app::registry::AppManifest;
use serde::Deserialize;
use std::path::{Path, PathBuf};

const DEFAULT_CHECK_SIZES: &[(u32, u32)] = &[(320, 240), (480, 320), (800, 600), (1200, 800)];

#[derive(Debug, Default)]
struct SdkAnalysis {
    has_init: bool,
    has_update: bool,
    has_view: bool,
    legacy_app_classes: Vec<String>,
    errors: Vec<String>,
    warnings: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct PythonAstReport {
    functions: Vec<String>,
    legacy_app_classes: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ScaffoldMetadata {
    schema_version: u32,
    plexi_cli_version: String,
    sdk_version: String,
    manifest_schema_version: u32,
    python_runtime_version: String,
    template_version: u32,
    channel: String,
    profile_dir: String,
}

pub fn app_check_cli(path: &str, sizes: &[String], png_dir: Option<&str>) -> i32 {
    let app_dir = Path::new(path);
    log::info!(
        "app_check: path={} sizes={sizes:?} png_dir={png_dir:?}",
        app_dir.display()
    );

    let render_sizes = match check_sizes(sizes) {
        Ok(sizes) => sizes,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };

    let png_dir = match png_dir {
        Some(raw) => {
            let dir = PathBuf::from(raw);
            if let Err(e) = std::fs::create_dir_all(&dir) {
                eprintln!(
                    "error: could not create PNG output directory {}: {e}",
                    dir.display()
                );
                return 1;
            }
            Some(dir)
        }
        None => None,
    };

    let mut errors = Vec::new();
    let mut warnings = Vec::new();

    let (manifest, entry_path) = match load_local_app(app_dir) {
        Ok(app) => app,
        Err(e) => {
            println!("✗ manifest — {e}");
            return 1;
        }
    };

    println!("✓ manifest — {} ({})", manifest.app.id, manifest.app.name);
    warnings.extend(scaffold_metadata_warnings(app_dir));

    if entry_path.extension().and_then(|ext| ext.to_str()) == Some("py") {
        match analyze_python_entry(&manifest.app.id, &entry_path, &manifest.app.dependencies) {
            Ok(analysis) => {
                if analysis.has_init && analysis.has_update && analysis.has_view {
                    println!("✓ SDK — module-level init/update/view");
                }
                if !analysis.legacy_app_classes.is_empty() {
                    for class in &analysis.legacy_app_classes {
                        errors.push(format!(
                            "SDK — {class} subclasses legacy App; SDK v3 apps use module-level init/update/view functions"
                        ));
                    }
                }
                errors.extend(analysis.errors);
                warnings.extend(analysis.warnings);
            }
            Err(e) => errors.push(format!("SDK — {e}")),
        }
    } else {
        warnings.push(format!(
            "SDK — {} is not a Python entry; skipping Python SDK checks",
            entry_path.display()
        ));
    }

    for (width, height) in render_sizes {
        let label = format!("{width}x{height}");
        match crate::render::app_render::render_app_to_json(
            &manifest.app.id,
            &entry_path,
            width,
            height,
            None,
            &manifest.app.dependencies,
        ) {
            Ok(json) => {
                let frame: serde_json::Value = match serde_json::from_str(&json) {
                    Ok(value) => value,
                    Err(e) => {
                        errors.push(format!("render {label} — invalid JSON frame: {e}"));
                        continue;
                    }
                };
                let command_count = frame.as_array().map(Vec::len).unwrap_or(0);
                if command_count == 0 {
                    errors.push(format!("render {label} — app emitted an empty frame"));
                    continue;
                }
                let bounds = obvious_bounds_errors(&frame, width, height);
                if bounds.is_empty() {
                    println!("✓ render {label} — {command_count} command(s)");
                } else {
                    for issue in bounds {
                        errors.push(format!("render {label} — {issue}"));
                    }
                }
                if let Some(dir) = &png_dir {
                    let png_path = dir.join(format!("{}-{label}.png", manifest.app.id));
                    match crate::render::app_render::render_app_to_png(
                        &manifest.app.id,
                        &entry_path,
                        width,
                        height,
                        None,
                        &manifest.app.dependencies,
                    ) {
                        Ok(bytes) => match std::fs::write(&png_path, bytes) {
                            Ok(()) => println!("✓ png {label} — {}", png_path.display()),
                            Err(e) => errors.push(format!(
                                "png {label} — could not write {}: {e}",
                                png_path.display()
                            )),
                        },
                        Err(e) => errors.push(format!("png {label} — {e}")),
                    }
                }
            }
            Err(e) => errors.push(format!("render {label} — {e}")),
        }
    }

    for warning in &warnings {
        println!("warning: {warning}");
    }
    for error in &errors {
        println!("error: {error}");
    }

    if errors.is_empty() {
        println!("✓ app check passed");
        log::info!(
            "app_check[{}]: passed with {} warning(s)",
            manifest.app.id,
            warnings.len()
        );
        0
    } else {
        println!(
            "✗ app check failed — {} error(s), {} warning(s)",
            errors.len(),
            warnings.len()
        );
        log::warn!(
            "app_check[{}]: failed with {} error(s), {} warning(s)",
            manifest.app.id,
            errors.len(),
            warnings.len()
        );
        1
    }
}

fn check_sizes(raw_sizes: &[String]) -> Result<Vec<(u32, u32)>, String> {
    if raw_sizes.is_empty() {
        return Ok(DEFAULT_CHECK_SIZES.to_vec());
    }

    let mut parsed = Vec::with_capacity(raw_sizes.len());
    for raw in raw_sizes {
        let Some((w, h)) = parse_size(raw) else {
            return Err(format!(
                "invalid --size '{raw}' — expected WxH, for example 800x600"
            ));
        };
        if !parsed.contains(&(w, h)) {
            parsed.push((w, h));
        }
    }
    Ok(parsed)
}

fn parse_size(raw: &str) -> Option<(u32, u32)> {
    let (w, h) = raw.split_once('x')?;
    let w = w.parse::<u32>().ok()?;
    let h = h.parse::<u32>().ok()?;
    if w == 0 || h == 0 {
        return None;
    }
    Some((w, h))
}

fn load_local_app(app_dir: &Path) -> Result<(AppManifest, PathBuf), String> {
    if !app_dir.exists() {
        return Err(format!("path does not exist: {}", app_dir.display()));
    }
    if !app_dir.is_dir() {
        return Err(format!("path is not a directory: {}", app_dir.display()));
    }

    let manifest_path = app_dir.join("manifest.toml");
    let manifest_raw = std::fs::read_to_string(&manifest_path)
        .map_err(|e| format!("could not read {}: {e}", manifest_path.display()))?;
    let manifest: AppManifest =
        toml::from_str(&manifest_raw).map_err(|e| format!("invalid manifest.toml: {e}"))?;

    if manifest.schema_version > crate::app::registry::MANIFEST_SCHEMA_VERSION {
        return Err(format!(
            "manifest schema_version {} is newer than this Plexi build supports ({})",
            manifest.schema_version,
            crate::app::registry::MANIFEST_SCHEMA_VERSION
        ));
    }

    let entry_path = app_dir.join(&manifest.app.entry);
    if !entry_path.exists() {
        return Err(format!(
            "entry file '{}' not found in {}",
            manifest.app.entry,
            app_dir.display()
        ));
    }

    Ok((manifest, entry_path))
}

fn scaffold_metadata_warnings(app_dir: &Path) -> Vec<String> {
    let metadata_path = app_dir.join(crate::cli::app::SCAFFOLD_METADATA_FILE);
    let mut warnings = Vec::new();

    let raw = match std::fs::read_to_string(&metadata_path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            warnings.push(format!(
                "scaffold metadata — missing {}; old scaffolds are still supported, but regenerate with `plexi app init` or add metadata before relying on drift checks",
                crate::cli::app::SCAFFOLD_METADATA_FILE
            ));
            log::warn!(
                "app_check: missing scaffold metadata at {}",
                metadata_path.display()
            );
            return warnings;
        }
        Err(e) => {
            warnings.push(format!(
                "scaffold metadata — could not read {}: {e}",
                metadata_path.display()
            ));
            log::warn!(
                "app_check: could not read scaffold metadata at {}: {e}",
                metadata_path.display()
            );
            return warnings;
        }
    };

    let metadata: ScaffoldMetadata = match toml::from_str(&raw) {
        Ok(metadata) => metadata,
        Err(e) => {
            warnings.push(format!(
                "scaffold metadata — invalid {}: {e}",
                metadata_path.display()
            ));
            log::warn!(
                "app_check: invalid scaffold metadata at {}: {e}",
                metadata_path.display()
            );
            return warnings;
        }
    };

    let current_cli = env!("CARGO_PKG_VERSION");
    let current_sdk = crate::cli::app::python_sdk_version();
    let current_manifest_schema = crate::app::registry::MANIFEST_SCHEMA_VERSION;
    let current_runtime = crate::app::python_env::PYTHON_APP_VENV_VERSION;
    let current_template = crate::cli::app::PYTHON_SCAFFOLD_TEMPLATE_VERSION;
    let (current_channel, current_profile_dir) = crate::cli::app::current_scaffold_channel();

    if metadata.schema_version != crate::cli::app::SCAFFOLD_METADATA_SCHEMA_VERSION {
        warnings.push(format!(
            "scaffold metadata — schema_version {} differs from supported {}; regenerate or update {}",
            metadata.schema_version,
            crate::cli::app::SCAFFOLD_METADATA_SCHEMA_VERSION,
            crate::cli::app::SCAFFOLD_METADATA_FILE
        ));
    }
    if metadata.plexi_cli_version != current_cli {
        warnings.push(format!(
            "scaffold metadata — generated by Plexi CLI {}, checking with {}; run `plexi app check` on the intended build or regenerate the scaffold",
            metadata.plexi_cli_version, current_cli
        ));
    }
    if metadata.sdk_version != current_sdk {
        warnings.push(format!(
            "scaffold metadata — generated for Python SDK {}, current SDK is {}; rerun tests/checks with the intended channel or refresh the scaffold",
            metadata.sdk_version, current_sdk
        ));
    }
    if metadata.manifest_schema_version != current_manifest_schema {
        warnings.push(format!(
            "scaffold metadata — manifest schema {} differs from current {}; update manifest.toml or regenerate",
            metadata.manifest_schema_version, current_manifest_schema
        ));
    }
    if metadata.python_runtime_version != current_runtime {
        warnings.push(format!(
            "scaffold metadata — generated for Python {}, current app venv runtime is {}; recreate the venv or regenerate",
            metadata.python_runtime_version, current_runtime
        ));
    }
    if metadata.template_version != current_template {
        warnings.push(format!(
            "scaffold metadata — generated from template {}, current template is {}; regenerate to pick up scaffold guidance",
            metadata.template_version, current_template
        ));
    }
    if metadata.channel != current_channel || metadata.profile_dir != current_profile_dir {
        warnings.push(format!(
            "scaffold metadata — generated for channel/profile {}/{}, checking under {}/{}; use an explicit matching PLEXI_CHANNEL or regenerate under this profile",
            metadata.channel, metadata.profile_dir, current_channel, current_profile_dir
        ));
    }

    if warnings.is_empty() {
        println!(
            "✓ scaffold metadata — {} template {}",
            metadata.sdk_version, metadata.template_version
        );
        log::info!(
            "app_check: scaffold metadata current at {}",
            metadata_path.display()
        );
    } else {
        log::warn!(
            "app_check: scaffold metadata produced {} warning(s) at {}",
            warnings.len(),
            metadata_path.display()
        );
    }

    warnings
}

fn analyze_python_entry(
    app_id: &str,
    entry_path: &Path,
    python_dependencies: &[String],
) -> Result<SdkAnalysis, String> {
    let runtime = crate::app::python_env::resolve_python_runtime(
        app_id,
        entry_path,
        true,
        python_dependencies,
    )?;
    analyze_python_entry_with(&entry_path, Path::new(&runtime.executable))
}

fn analyze_python_entry_with(entry_path: &Path, python: &Path) -> Result<SdkAnalysis, String> {
    let script = r#"
import ast
import json
import sys

path = sys.argv[1]
with open(path, "r", encoding="utf-8") as fh:
    tree = ast.parse(fh.read(), filename=path)

def base_name(node):
    if isinstance(node, ast.Name):
        return node.id
    if isinstance(node, ast.Attribute):
        return node.attr
    if isinstance(node, ast.Subscript):
        return base_name(node.value)
    if isinstance(node, ast.Call):
        return base_name(node.func)
    return ""

functions = []
legacy_app_classes = []
for node in tree.body:
    if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
        functions.append(node.name)
    if isinstance(node, ast.ClassDef):
        if any(base_name(base) == "App" for base in node.bases):
            legacy_app_classes.append(node.name)

json.dump({"functions": functions, "legacy_app_classes": legacy_app_classes}, sys.stdout)
"#;

    let output = std::process::Command::new(python)
        .arg("-c")
        .arg(script)
        .arg(entry_path)
        .output()
        .map_err(|e| format!("could not run {} for AST analysis: {e}", python.display()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "Python syntax or AST error in {}: {}",
            entry_path.display(),
            stderr.trim()
        ));
    }

    let report: PythonAstReport = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("could not parse AST report: {e}"))?;

    let mut analysis = SdkAnalysis::default();
    analysis.has_init = report.functions.iter().any(|name| name == "init");
    analysis.has_update = report.functions.iter().any(|name| name == "update");
    analysis.has_view = report.functions.iter().any(|name| name == "view");
    analysis.legacy_app_classes = report.legacy_app_classes;

    for required in ["init", "update", "view"] {
        if !report.functions.iter().any(|name| name == required) {
            analysis.errors.push(format!(
                "SDK — missing module-level {required}(); SDK v3 apps define init(size, args), update(event), and view()"
            ));
        }
    }

    Ok(analysis)
}

fn obvious_bounds_errors(frame: &serde_json::Value, width: u32, height: u32) -> Vec<String> {
    let Some(commands) = frame.as_array() else {
        return vec!["frame is not a command array".to_string()];
    };

    let mut errors = Vec::new();
    for (idx, command) in commands.iter().enumerate() {
        collect_bounds_errors(
            command,
            width as f64,
            height as f64,
            &format!("command {idx}"),
            &mut errors,
        );
    }
    errors
}

fn collect_bounds_errors(
    value: &serde_json::Value,
    viewport_w: f64,
    viewport_h: f64,
    label: &str,
    errors: &mut Vec<String>,
) {
    let Some(obj) = value.as_object() else {
        return;
    };

    let type_name = obj
        .get("type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("node");

    let x = obj.get("x").and_then(serde_json::Value::as_f64);
    let y = obj.get("y").and_then(serde_json::Value::as_f64);
    let w = obj.get("w").and_then(serde_json::Value::as_f64);
    let h = obj.get("h").and_then(serde_json::Value::as_f64);

    if let (Some(x), Some(y), Some(w), Some(h)) = (x, y, w, h) {
        if w < 0.0 || h < 0.0 {
            errors.push(format!("{label} {type_name} has negative size {w}x{h}"));
        }
        if x + w < 0.0 || y + h < 0.0 || x > viewport_w || y > viewport_h {
            errors.push(format!(
                "{label} {type_name} is outside the viewport at {x},{y} {w}x{h}"
            ));
        }
        if w > viewport_w * 4.0 || h > viewport_h * 4.0 {
            errors.push(format!(
                "{label} {type_name} is far larger than the viewport: {w}x{h}"
            ));
        }
    } else if let (Some(x), Some(y)) = (x, y) {
        if x < -viewport_w || y < -viewport_h || x > viewport_w * 2.0 || y > viewport_h * 2.0 {
            errors.push(format!(
                "{label} {type_name} is far outside the viewport at {x},{y}"
            ));
        }
    }

    for key in ["root", "children", "items", "tiers", "command"] {
        if let Some(child) = obj.get(key) {
            match child {
                serde_json::Value::Array(values) => {
                    for (idx, nested) in values.iter().enumerate() {
                        collect_bounds_errors(
                            nested,
                            viewport_w,
                            viewport_h,
                            &format!("{label}.{key}[{idx}]"),
                            errors,
                        );
                    }
                }
                serde_json::Value::Object(_) => {
                    collect_bounds_errors(
                        child,
                        viewport_w,
                        viewport_h,
                        &format!("{label}.{key}"),
                        errors,
                    );
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod app_check_tests {
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn default_sizes_cover_small_and_normal_panes() {
        let sizes = super::check_sizes(&[]).expect("default sizes should parse");

        assert!(sizes.contains(&(320, 240)));
        assert!(sizes.contains(&(480, 320)));
        assert!(sizes.contains(&(800, 600)));
    }

    #[test]
    fn sdk_analysis_accepts_module_level_v3_app() {
        let dir = TempDir::new().unwrap();
        let entry = dir.path().join("main.py");
        fs::write(
            &entry,
            r#"
def init(size, args):
    return []

def update(event):
    return []

def view():
    return None
"#,
        )
        .unwrap();

        let analysis = super::analyze_python_entry_with(&entry, std::path::Path::new("python3"))
            .expect("analysis should run");
        assert!(analysis.errors.is_empty(), "{:?}", analysis.errors);
    }

    #[test]
    fn sdk_analysis_rejects_legacy_app_subclass() {
        let dir = TempDir::new().unwrap();
        let entry = dir.path().join("main.py");
        fs::write(
            &entry,
            r#"
from plexi_sdk import App

class Bad(App):
    pass

def init(size, args):
    return []

def update(event):
    return []

def view():
    return None
"#,
        )
        .unwrap();

        let analysis = super::analyze_python_entry_with(&entry, std::path::Path::new("python3"))
            .expect("analysis should run");
        assert!(
            analysis
                .legacy_app_classes
                .iter()
                .any(|class| class == "Bad"),
            "{:?}",
            analysis.legacy_app_classes
        );
    }

    #[test]
    fn sdk_analysis_rejects_missing_update() {
        let dir = TempDir::new().unwrap();
        let entry = dir.path().join("main.py");
        fs::write(
            &entry,
            r#"
def init(size, args):
    return []

def view():
    return None
"#,
        )
        .unwrap();

        let analysis = super::analyze_python_entry_with(&entry, std::path::Path::new("python3"))
            .expect("analysis should run");
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.contains("missing module-level update()")),
            "{:?}",
            analysis.errors
        );
    }

    #[test]
    fn scaffold_metadata_missing_warns_without_failing_legacy_apps() {
        let dir = TempDir::new().unwrap();

        let warnings = super::scaffold_metadata_warnings(dir.path());

        assert!(
            warnings
                .iter()
                .any(|warning| warning.contains("missing plexi.scaffold.toml")),
            "{warnings:?}"
        );
    }

    #[test]
    fn scaffold_metadata_stale_values_warn() {
        let _channel = crate::config::set_test_channel("alpha");
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join(crate::cli::app::SCAFFOLD_METADATA_FILE),
            r#"schema_version = 1
generated_by = "plexi app init"
plexi_cli_version = "0.0.0"
sdk_version = "0.0.0"
manifest_schema_version = 0
python_runtime_version = "3.10"
template_version = 0
channel = "beta"
profile_dir = ".plexi-beta"
"#,
        )
        .unwrap();

        let warnings = super::scaffold_metadata_warnings(dir.path());

        assert!(
            warnings
                .iter()
                .any(|warning| warning.contains("generated by Plexi CLI 0.0.0")),
            "{warnings:?}"
        );
        assert!(
            warnings
                .iter()
                .any(|warning| warning.contains("generated for Python SDK 0.0.0")),
            "{warnings:?}"
        );
        assert!(
            warnings
                .iter()
                .any(|warning| warning.contains("generated from template 0")),
            "{warnings:?}"
        );
        assert!(
            warnings
                .iter()
                .any(|warning| warning.contains("generated for channel/profile beta/.plexi-beta")),
            "{warnings:?}"
        );
    }
}

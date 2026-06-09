use crate::app::registry::AppManifest;
use serde::Deserialize;
use std::path::{Path, PathBuf};

const DEFAULT_CHECK_SIZES: &[(u32, u32)] = &[(320, 240), (480, 320), (800, 600), (1200, 800)];

#[derive(Debug, Default)]
struct SdkAnalysis {
    app_classes: Vec<PythonAppClass>,
    errors: Vec<String>,
    warnings: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct PythonAstReport {
    classes: Vec<PythonClassReport>,
}

#[derive(Debug, Deserialize)]
struct PythonClassReport {
    name: String,
    bases: Vec<String>,
    methods: Vec<String>,
}

#[derive(Debug, Clone)]
struct PythonAppClass {
    name: String,
    has_view: bool,
    has_on_render: bool,
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

    if entry_path.extension().and_then(|ext| ext.to_str()) == Some("py") {
        match analyze_python_entry(&entry_path) {
            Ok(analysis) => {
                if analysis.app_classes.is_empty() {
                    errors.push(format!(
                        "SDK — {} does not define a class that subclasses App",
                        entry_path.display()
                    ));
                } else {
                    for class in analysis.app_classes {
                        if class.has_view {
                            println!("✓ SDK — {} uses view()", class.name);
                        } else if class.has_on_render {
                            warnings.push(format!(
                                "SDK — {} uses on_render(ctx); use this only for canvas, games, animation, or visual tools",
                                class.name
                            ));
                        }
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

fn analyze_python_entry(entry_path: &Path) -> Result<SdkAnalysis, String> {
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

classes = []
for node in ast.walk(tree):
    if isinstance(node, ast.ClassDef):
        methods = [
            child.name
            for child in node.body
            if isinstance(child, (ast.FunctionDef, ast.AsyncFunctionDef))
        ]
        classes.append({
            "name": node.name,
            "bases": [base_name(base) for base in node.bases],
            "methods": methods,
        })

json.dump({"classes": classes}, sys.stdout)
"#;

    let output = std::process::Command::new("python3")
        .arg("-c")
        .arg(script)
        .arg(entry_path)
        .output()
        .map_err(|e| format!("could not run python3 for AST analysis: {e}"))?;

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
    for class in report.classes {
        if !class.bases.iter().any(|base| base == "App") {
            continue;
        }
        let has_view = class.methods.iter().any(|method| method == "view");
        let has_on_render = class.methods.iter().any(|method| method == "on_render");
        if has_view && has_on_render {
            analysis.errors.push(format!(
                "SDK — {} defines both view() and on_render(ctx); pick one render path",
                class.name
            ));
        }
        if !has_view && !has_on_render {
            analysis.errors.push(format!(
                "SDK — {} defines neither view() nor on_render(ctx)",
                class.name
            ));
        }
        analysis.app_classes.push(PythonAppClass {
            name: class.name,
            has_view,
            has_on_render,
        });
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
    fn sdk_analysis_accepts_view_only_app() {
        let dir = TempDir::new().unwrap();
        let entry = dir.path().join("main.py");
        fs::write(
            &entry,
            r#"
from plexi_sdk import App

class Good(App):
    def view(self):
        return None
"#,
        )
        .unwrap();

        let analysis = super::analyze_python_entry(&entry).expect("analysis should run");
        assert!(analysis.errors.is_empty(), "{:?}", analysis.errors);
    }

    #[test]
    fn sdk_analysis_rejects_app_that_overrides_both_render_paths() {
        let dir = TempDir::new().unwrap();
        let entry = dir.path().join("main.py");
        fs::write(
            &entry,
            r#"
from plexi_sdk import App

class Bad(App):
    def view(self):
        return None

    def on_render(self, ctx):
        pass
"#,
        )
        .unwrap();

        let analysis = super::analyze_python_entry(&entry).expect("analysis should run");
        assert!(
            analysis
                .errors
                .iter()
                .any(|error| error.contains("defines both view() and on_render(ctx)")),
            "{:?}",
            analysis.errors
        );
    }
}

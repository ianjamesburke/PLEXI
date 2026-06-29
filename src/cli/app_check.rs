use crate::app::registry::AppManifest;
use crate::app_protocol::{RenderCommand, UiNode};
use serde::Deserialize;
use std::path::{Path, PathBuf};

const DEFAULT_CHECK_SIZES: &[(u32, u32)] = &[(320, 240), (480, 320), (800, 600), (1200, 800)];
const SEEDED_PROBE_SIZE: (u32, u32) = (480, 320);
const RECOGNIZED_ACTION_HANDLERS: &[&str] = &["counter-increment"];

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

#[derive(Debug)]
struct SeedFixture {
    path: PathBuf,
    state: serde_json::Value,
    signals: Vec<String>,
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
    let require_semantic_chrome = scaffold_requires_semantic_chrome(app_dir);
    let mut checked_semantic_chrome = false;

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
                if require_semantic_chrome && !checked_semantic_chrome {
                    let chrome_errors = semantic_scaffold_chrome_errors(&frame);
                    if chrome_errors.is_empty() {
                        println!("✓ semantic chrome — app bar, action bar, pinned footer keys");
                        log::info!(
                            "app_check[{}]: semantic scaffold chrome present",
                            manifest.app.id
                        );
                    } else {
                        for issue in chrome_errors {
                            errors.push(format!("semantic chrome — {issue}"));
                        }
                    }
                    checked_semantic_chrome = true;
                }
                if require_semantic_chrome {
                    let shell_errors = scaffold_shell_layout_errors(&frame, width, height);
                    if shell_errors.is_empty() {
                        println!("✓ shell layout {label} — body/action/footer slots resolved");
                        log::info!(
                            "app_check[{}]: shell layout resolved at {label}",
                            manifest.app.id
                        );
                    } else {
                        for issue in shell_errors {
                            errors.push(format!("shell layout {label} — {issue}"));
                        }
                    }
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
                        Ok(bytes) => {
                            if require_semantic_chrome {
                                let png_errors = semantic_png_chrome_errors(&bytes, width, height);
                                if png_errors.is_empty() {
                                    println!("✓ png chrome {label} — full-bleed, unclipped footer");
                                } else {
                                    for issue in png_errors {
                                        errors.push(format!("png chrome {label} — {issue}"));
                                    }
                                }
                            }
                            match std::fs::write(&png_path, bytes) {
                                Ok(()) => println!("✓ png {label} — {}", png_path.display()),
                                Err(e) => errors.push(format!(
                                    "png {label} — could not write {}: {e}",
                                    png_path.display()
                                )),
                            }
                        }
                        Err(e) => errors.push(format!("png {label} — {e}")),
                    }
                }
            }
            Err(e) => errors.push(format!("render {label} — {e}")),
        }
    }

    match load_seed_fixture(app_dir) {
        Ok(Some(fixture)) => {
            println!(
                "✓ state fixture — loaded {}",
                fixture
                    .path
                    .strip_prefix(app_dir)
                    .unwrap_or(&fixture.path)
                    .display()
            );
            run_seeded_state_and_action_probe(
                &manifest,
                &entry_path,
                &fixture,
                require_semantic_chrome,
                &mut errors,
                &mut warnings,
            );
        }
        Ok(None) => {
            println!("skip state fixture — no fixtures/state.json; seeded probes skipped");
            log::info!(
                "app_check[{}]: no fixtures/state.json; seeded probes skipped",
                manifest.app.id
            );
        }
        Err(e) => errors.push(format!("state fixture — {e}")),
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

fn load_seed_fixture(app_dir: &Path) -> Result<Option<SeedFixture>, String> {
    let path = app_dir.join("fixtures").join("state.json");
    if !path.exists() {
        return Ok(None);
    }

    let raw = std::fs::read_to_string(&path)
        .map_err(|e| format!("could not read {}: {e}", path.display()))?;
    let state: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|e| format!("invalid JSON in {}: {e}", path.display()))?;
    if !state.is_object() {
        return Err(format!(
            "{} must be a plain JSON object, for example {{\"count\": 3}}",
            path.display()
        ));
    }

    let signals = seed_state_signals(&state);
    Ok(Some(SeedFixture {
        path,
        state,
        signals,
    }))
}

fn run_seeded_state_and_action_probe(
    manifest: &AppManifest,
    entry_path: &Path,
    fixture: &SeedFixture,
    probe_semantic_actions: bool,
    errors: &mut Vec<String>,
    warnings: &mut Vec<String>,
) {
    let (width, height) = SEEDED_PROBE_SIZE;
    let label = format!("{width}x{height}");
    if fixture.signals.is_empty() {
        warnings.push(format!(
            "seeded render {label} — fixtures/state.json has no scalar values to verify"
        ));
        return;
    }

    let frame = match crate::render::app_render::render_app_to_json(
        &manifest.app.id,
        entry_path,
        width,
        height,
        Some(fixture.state.clone()),
        &manifest.app.dependencies,
    ) {
        Ok(json) => match serde_json::from_str::<serde_json::Value>(&json) {
            Ok(frame) => frame,
            Err(e) => {
                errors.push(format!("seeded render {label} — invalid JSON frame: {e}"));
                return;
            }
        },
        Err(e) => {
            errors.push(format!("seeded render {label} — {e}"));
            return;
        }
    };

    let matched = matched_seed_signals(&frame, &fixture.signals);
    if matched.is_empty() {
        errors.push(format!(
            "seeded render {label} — rendered frame did not include any scalar fixture value ({})",
            fixture.signals.join(", ")
        ));
        return;
    }
    println!(
        "✓ seeded render {label} — reflected fixture value(s): {}",
        matched.join(", ")
    );
    log::info!(
        "app_check[{}]: seeded render {label} reflected {:?}",
        manifest.app.id,
        matched
    );

    let Some(handler_id) = recognized_action_handler(&frame, probe_semantic_actions) else {
        println!("skip action probe — no recognized action handler in seeded render");
        log::info!(
            "app_check[{}]: no recognized action handler in seeded render",
            manifest.app.id
        );
        return;
    };

    let action_label = format!("action probe {handler_id}");
    let expected_after = expected_action_signal(&fixture.state, &handler_id);
    match crate::render::app_render::render_app_ui_action_round_trip(
        &manifest.app.id,
        entry_path,
        width,
        height,
        Some(fixture.state.clone()),
        &handler_id,
        &manifest.app.dependencies,
    ) {
        Ok((before, after)) => {
            let before_frame = render_commands_value(&before);
            let after_frame = render_commands_value(&after);
            if let Some(expected) = expected_after {
                if frame_contains_scalar(&after_frame, &expected) {
                    println!("✓ {action_label} — rendered expected state value {expected}");
                    log::info!(
                        "app_check[{}]: {action_label} rendered expected value {expected}",
                        manifest.app.id
                    );
                } else {
                    errors.push(format!(
                        "{action_label} — expected rendered state value {expected} after action"
                    ));
                }
            } else if after_frame != before_frame {
                println!("✓ {action_label} — rendered frame changed after action");
                log::info!(
                    "app_check[{}]: {action_label} changed rendered frame",
                    manifest.app.id
                );
            } else {
                warnings.push(format!(
                    "{action_label} — action ran, but no generic state expectation was available and the frame did not change"
                ));
            }
        }
        Err(e) => errors.push(format!("{action_label} — {e}")),
    }
}

fn render_commands_value(commands: &[RenderCommand]) -> serde_json::Value {
    serde_json::to_value(commands).unwrap_or_else(|_| serde_json::Value::Null)
}

fn seed_state_signals(state: &serde_json::Value) -> Vec<String> {
    let mut signals = Vec::new();
    collect_scalar_signals(state, &mut signals);
    signals.sort();
    signals.dedup();
    signals
}

fn collect_scalar_signals(value: &serde_json::Value, signals: &mut Vec<String>) {
    match value {
        serde_json::Value::String(s) if !s.is_empty() => signals.push(s.clone()),
        serde_json::Value::Number(n) => signals.push(n.to_string()),
        serde_json::Value::Array(values) => {
            for value in values {
                collect_scalar_signals(value, signals);
            }
        }
        serde_json::Value::Object(map) => {
            for value in map.values() {
                collect_scalar_signals(value, signals);
            }
        }
        _ => {}
    }
}

fn matched_seed_signals(frame: &serde_json::Value, signals: &[String]) -> Vec<String> {
    signals
        .iter()
        .filter(|signal| frame_contains_scalar(frame, signal))
        .cloned()
        .collect()
}

fn frame_contains_scalar(frame: &serde_json::Value, expected: &str) -> bool {
    match frame {
        serde_json::Value::String(s) => s == expected,
        serde_json::Value::Number(n) => n.to_string() == expected,
        serde_json::Value::Bool(b) => b.to_string() == expected,
        serde_json::Value::Array(values) => values
            .iter()
            .any(|value| frame_contains_scalar(value, expected)),
        serde_json::Value::Object(map) => map
            .values()
            .any(|value| frame_contains_scalar(value, expected)),
        _ => false,
    }
}

fn recognized_action_handler(
    frame: &serde_json::Value,
    probe_semantic_actions: bool,
) -> Option<String> {
    for handler in RECOGNIZED_ACTION_HANDLERS {
        if frame_contains_keyed_string(frame, &["node_id", "handler_id"], handler) {
            return Some((*handler).to_string());
        }
    }
    if probe_semantic_actions {
        return first_semantic_action_handler(frame);
    }
    None
}

fn first_semantic_action_handler(frame: &serde_json::Value) -> Option<String> {
    match frame {
        serde_json::Value::Object(map) => {
            if map.get("type").and_then(serde_json::Value::as_str) == Some("action_bar") {
                if let Some(actions) = map.get("actions").and_then(serde_json::Value::as_array) {
                    for action in actions {
                        if let Some(handler) = action
                            .as_object()
                            .and_then(|obj| {
                                obj.get("node_id")
                                    .or_else(|| obj.get("handler_id"))
                                    .and_then(serde_json::Value::as_str)
                            })
                            .filter(|handler| !handler.is_empty())
                        {
                            return Some(handler.to_string());
                        }
                    }
                }
            }
            map.values().find_map(first_semantic_action_handler)
        }
        serde_json::Value::Array(values) => values.iter().find_map(first_semantic_action_handler),
        _ => None,
    }
}

fn frame_contains_keyed_string(frame: &serde_json::Value, keys: &[&str], expected: &str) -> bool {
    match frame {
        serde_json::Value::Object(map) => {
            if keys.iter().any(|key| {
                map.get(*key)
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|value| value == expected)
            }) {
                return true;
            }
            map.values()
                .any(|value| frame_contains_keyed_string(value, keys, expected))
        }
        serde_json::Value::Array(values) => values
            .iter()
            .any(|value| frame_contains_keyed_string(value, keys, expected)),
        _ => false,
    }
}

fn expected_action_signal(state: &serde_json::Value, handler_id: &str) -> Option<String> {
    if handler_id == "counter-increment" {
        return state
            .get("count")
            .and_then(serde_json::Value::as_i64)
            .map(|count| (count + 1).to_string());
    }
    None
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

fn scaffold_requires_semantic_chrome(app_dir: &Path) -> bool {
    let metadata_path = app_dir.join(crate::cli::app::SCAFFOLD_METADATA_FILE);
    let Ok(raw) = std::fs::read_to_string(&metadata_path) else {
        return false;
    };
    let Ok(metadata) = toml::from_str::<ScaffoldMetadata>(&raw) else {
        return false;
    };
    metadata.template_version >= crate::cli::app::PYTHON_SCAFFOLD_TEMPLATE_VERSION
}

fn semantic_scaffold_chrome_errors(frame: &serde_json::Value) -> Vec<String> {
    let Some(root) = first_component_tree_root(frame) else {
        return vec!["missing component_tree root".to_string()];
    };
    let Some(root_obj) = root.as_object() else {
        return vec!["component_tree root is not an object".to_string()];
    };
    if root_obj.get("type").and_then(serde_json::Value::as_str) != Some("column") {
        return vec!["root must be a semantic column".to_string()];
    }
    let padding = root_obj
        .get("padding")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(crate::ui::style::SPACE_XL as f64);
    if padding < crate::ui::style::SPACE_MD as f64 {
        return vec![format!(
            "root semantic column padding must be at least {:.0}px; remove padding=0 or set padding >= {:.0}",
            crate::ui::style::SPACE_MD,
            crate::ui::style::SPACE_MD
        )];
    }
    let Some(children) = root_obj
        .get("children")
        .and_then(serde_json::Value::as_array)
    else {
        return vec!["column root must contain children".to_string()];
    };
    if children.is_empty() {
        return vec!["column root has no children".to_string()];
    }

    let mut errors = Vec::new();
    if node_type(&children[0]) != Some("app_bar") {
        errors.push("first column child must be app_bar".to_string());
    }

    let action_idx = children
        .iter()
        .position(|child| node_type(child) == Some("action_bar"));
    let footer_idx = children.iter().position(is_pinned_footer_keys);

    let Some(action_idx) = action_idx else {
        errors.push("missing semantic action_bar child".to_string());
        return errors;
    };
    let Some(footer_idx) = footer_idx else {
        errors.push("missing bottom-pinned footer_keys child".to_string());
        return errors;
    };

    if action_idx == 0 {
        errors.push("action_bar must follow app body content".to_string());
    }
    if action_idx > footer_idx {
        errors.push("action_bar must appear before pinned footer_keys".to_string());
    }
    if footer_idx + 1 != children.len() {
        errors.push("pinned footer_keys must be the final column child".to_string());
    }
    if action_idx < footer_idx
        && children[action_idx + 1..footer_idx].iter().any(|child| {
            node_type(child) == Some("spacer") && node_bool(child, "grow") == Some(true)
        })
    {
        errors
            .push("grow spacer must not sit between action_bar and pinned footer_keys".to_string());
    }

    if !action_bar_has_buttons(&children[action_idx]) {
        errors.push("action_bar must contain button actions".to_string());
    }
    if !contains_node_type(root, "card") {
        errors.push("current scaffold must include a semantic card surface".to_string());
    }
    if !contains_node_type(root, "text_edit") {
        errors.push("current scaffold must include a host-rendered text_edit".to_string());
    }
    for (node_type, label) in [
        ("section", "semantic section header"),
        ("badge", "semantic badge"),
        ("divider", "semantic divider"),
        ("select_list", "semantic select_list"),
    ] {
        if !contains_node_type(root, node_type) {
            errors.push(format!("current scaffold must include a {label}"));
        }
    }

    errors
}

fn scaffold_shell_layout_errors(frame: &serde_json::Value, width: u32, height: u32) -> Vec<String> {
    let Some(root) = first_component_tree_root(frame) else {
        return vec!["missing component_tree root".to_string()];
    };
    let root: UiNode = match serde_json::from_value(root.clone()) {
        Ok(root) => root,
        Err(e) => {
            return vec![format!(
                "component_tree root does not match UiNode schema: {e}"
            )]
        }
    };
    crate::render::components::validate_shell_layout(&root, width as f32, height as f32)
}

fn first_component_tree_root(frame: &serde_json::Value) -> Option<&serde_json::Value> {
    frame.as_array()?.iter().find_map(|command| {
        let obj = command.as_object()?;
        if obj.get("type").and_then(serde_json::Value::as_str) == Some("component_tree") {
            obj.get("root")
        } else {
            None
        }
    })
}

fn node_type(value: &serde_json::Value) -> Option<&str> {
    value.as_object()?.get("type")?.as_str()
}

fn node_bool(value: &serde_json::Value, key: &str) -> Option<bool> {
    value.as_object()?.get(key)?.as_bool()
}

fn is_pinned_footer_keys(value: &serde_json::Value) -> bool {
    let Some(obj) = value.as_object() else {
        return false;
    };
    obj.get("type").and_then(serde_json::Value::as_str) == Some("pinned")
        && obj.get("edge").and_then(serde_json::Value::as_str) == Some("bottom")
        && obj.get("child").and_then(node_type) == Some("footer_keys")
}

fn action_bar_has_buttons(value: &serde_json::Value) -> bool {
    let Some(actions) = value
        .as_object()
        .and_then(|obj| obj.get("actions"))
        .and_then(serde_json::Value::as_array)
    else {
        return false;
    };
    !actions.is_empty()
        && actions
            .iter()
            .all(|action| node_type(action) == Some("button"))
}

fn contains_node_type(value: &serde_json::Value, expected_type: &str) -> bool {
    match value {
        serde_json::Value::Object(map) => {
            if map
                .get("type")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|ty| ty == expected_type)
            {
                return true;
            }
            map.values()
                .any(|child| contains_node_type(child, expected_type))
        }
        serde_json::Value::Array(values) => values
            .iter()
            .any(|child| contains_node_type(child, expected_type)),
        _ => false,
    }
}

fn semantic_png_chrome_errors(bytes: &[u8], width: u32, height: u32) -> Vec<String> {
    let mut errors = Vec::new();
    let Ok(img) = image::load_from_memory(bytes) else {
        return vec!["could not decode PNG bytes".to_string()];
    };
    let rgba = img.to_rgba8();
    if rgba.width() != width || rgba.height() != height {
        errors.push(format!(
            "decoded size {}x{} did not match requested {width}x{height}",
            rgba.width(),
            rgba.height()
        ));
        return errors;
    }
    if width < 8 || height < 8 {
        errors.push(format!(
            "viewport too small for chrome pixel check: {width}x{height}"
        ));
        return errors;
    }

    let top_left = *rgba.get_pixel(0, 0);
    let top_mid = *rgba.get_pixel(width / 2, 0);
    let top_right = *rgba.get_pixel(width - 1, 0);
    if top_left != top_mid || top_right != top_mid {
        errors.push("app bar is not full-bleed across the top edge".to_string());
    }

    let footer_bg = *rgba.get_pixel(0, height - 1);
    let bottom_mid = *rgba.get_pixel(width / 2, height - 1);
    let bottom_right = *rgba.get_pixel(width - 1, height - 1);
    if bottom_mid != footer_bg || bottom_right != footer_bg {
        errors.push("footer is clipped or not full-bleed on the bottom edge".to_string());
    }

    let clean_rows = 6.min(height);
    let sample_step = 8.max(width / 40).max(1);
    for y in height - clean_rows..height {
        let mut x = 0;
        while x < width {
            if *rgba.get_pixel(x, y) != footer_bg {
                errors.push(format!(
                    "footer content reaches bottom padding at pixel ({x},{y})"
                ));
                return errors;
            }
            x = x.saturating_add(sample_step);
        }
        if *rgba.get_pixel(width - 1, y) != footer_bg {
            errors.push(format!(
                "footer content reaches bottom padding at pixel ({},{y})",
                width - 1
            ));
            return errors;
        }
    }

    errors
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
    use std::io::Cursor;
    use tempfile::TempDir;

    fn test_png_with_footer_pixel(bad_footer_pixel: Option<(u32, u32)>) -> Vec<u8> {
        let chrome = image::Rgba([24, 24, 37, 255]);
        let body = image::Rgba([0, 0, 0, 255]);
        let text = image::Rgba([166, 173, 200, 255]);
        let mut img = image::RgbaImage::from_pixel(32, 24, body);
        for x in 0..32 {
            img.put_pixel(x, 0, chrome);
        }
        for y in 18..24 {
            for x in 0..32 {
                img.put_pixel(x, y, chrome);
            }
        }
        if let Some((x, y)) = bad_footer_pixel {
            img.put_pixel(x, y, text);
        }
        let mut bytes = Vec::new();
        img.write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        bytes
    }

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

    #[test]
    fn seed_state_signals_collect_scalar_fixture_values() {
        let state = serde_json::json!({
            "count": 3,
            "mode": "demo",
            "nested": {"items": [7]},
        });

        let signals = super::seed_state_signals(&state);

        assert!(signals.contains(&"3".to_string()), "{signals:?}");
        assert!(signals.contains(&"demo".to_string()), "{signals:?}");
        assert!(signals.contains(&"7".to_string()), "{signals:?}");
    }

    #[test]
    fn seeded_frame_match_uses_exact_scalar_values_not_substrings() {
        let frame = serde_json::json!([
            {
                "type": "component_tree",
                "root": {
                    "type": "column",
                    "padding": 24.0,
                    "children": [
                        {"type": "text", "text": "13"},
                        {"type": "text", "text": "Runtime state counter"}
                    ]
                }
            }
        ]);

        assert!(!super::frame_contains_scalar(&frame, "3"));
        assert!(super::frame_contains_scalar(&frame, "13"));
    }

    #[test]
    fn recognized_action_handler_finds_counter_increment_button() {
        let frame = serde_json::json!([
            {
                "type": "component_tree",
                "root": {
                    "type": "column",
                    "children": [
                        {
                            "type": "button",
                            "node_id": "counter-increment",
                            "label": "Increment"
                        }
                    ]
                }
            }
        ]);

        assert_eq!(
            super::recognized_action_handler(&frame, false),
            Some("counter-increment".to_string())
        );
    }

    #[test]
    fn recognized_action_handler_uses_semantic_action_bar_when_enabled() {
        let frame = serde_json::json!([
            {
                "type": "component_tree",
                "root": {
                    "type": "column",
                    "children": [
                        {
                            "type": "action_bar",
                            "actions": [
                                {"type": "button", "node_id": "focus-add-5", "label": "+5"}
                            ]
                        }
                    ]
                }
            }
        ]);

        assert_eq!(super::recognized_action_handler(&frame, false), None);
        assert_eq!(
            super::recognized_action_handler(&frame, true),
            Some("focus-add-5".to_string())
        );
    }

    fn scaffold_proof_body() -> serde_json::Value {
        serde_json::json!({
            "type": "scroll",
            "child": {
                "type": "column",
                "padding": 0.0,
                "gap": 8.0,
                "children": [
                    {
                        "type": "card",
                        "padding": 8.0,
                        "children": [
                            {
                                "type": "stack",
                                "direction": "horizontal",
                                "gap": 8.0,
                                "children": [
                                    {"type": "text", "text": "3", "size": 24.0, "bold": true},
                                    {"type": "badge", "text": "host", "color": "accent"},
                                    {"type": "badge", "text": "semantic", "color": "neutral"}
                                ]
                            },
                            {"type": "divider"},
                            {"type": "text_edit", "node_id": "draft-note", "placeholder": "Working note", "value": "Draft something small"}
                        ]
                    },
                    {"type": "spacer", "size": 12.0},
                    {"type": "section", "title": "List"},
                    {
                        "type": "sized",
                        "height": 96.0,
                        "child": {
                            "type": "select_list",
                            "selected_idx": 0,
                            "items": [
                                {"name": "AppBar", "description": "full-bleed host chrome"},
                                {"name": "Card", "description": "themed surface"},
                                {"name": "TextEdit", "description": "host input chrome"}
                            ]
                        }
                    }
                ]
            }
        })
    }

    #[test]
    fn semantic_chrome_accepts_scaffold_tree() {
        let frame = serde_json::json!([
            {
                "type": "component_tree",
                "root": {
                    "type": "column",
                    "children": [
                        {"type": "app_bar", "title": "Counter"},
                        scaffold_proof_body(),
                        {
                            "type": "action_bar",
                            "actions": [
                                {"type": "button", "node_id": "counter-increment", "label": "Increment"}
                            ]
                        },
                        {
                            "type": "pinned",
                            "edge": "bottom",
                            "child": {"type": "footer_keys", "entries": []}
                        }
                    ]
                }
            }
        ]);

        assert!(super::semantic_scaffold_chrome_errors(&frame).is_empty());
    }

    #[test]
    fn semantic_chrome_rejects_zero_padding_shell() {
        let frame = serde_json::json!([
            {
                "type": "component_tree",
                "root": {
                    "type": "column",
                    "padding": 0.0,
                    "children": [
                        {"type": "app_bar", "title": "Counter"},
                        scaffold_proof_body(),
                        {
                            "type": "action_bar",
                            "actions": [
                                {"type": "button", "node_id": "counter-increment", "label": "Increment"}
                            ]
                        },
                        {
                            "type": "pinned",
                            "edge": "bottom",
                            "child": {"type": "footer_keys", "entries": []}
                        }
                    ]
                }
            }
        ]);

        assert!(
            super::semantic_scaffold_chrome_errors(&frame)
                .iter()
                .any(|error| error.contains("padding must be at least")),
            "{:?}",
            super::semantic_scaffold_chrome_errors(&frame)
        );
    }

    #[test]
    fn semantic_png_chrome_accepts_full_bleed_unclipped_footer() {
        let bytes = test_png_with_footer_pixel(None);

        assert!(super::semantic_png_chrome_errors(&bytes, 32, 24).is_empty());
    }

    #[test]
    fn semantic_png_chrome_rejects_footer_content_on_bottom_edge() {
        let bytes = test_png_with_footer_pixel(Some((16, 23)));

        assert!(
            super::semantic_png_chrome_errors(&bytes, 32, 24)
                .iter()
                .any(|error| error.contains("footer content reaches bottom padding")),
            "{:?}",
            super::semantic_png_chrome_errors(&bytes, 32, 24)
        );
    }

    #[test]
    fn semantic_chrome_rejects_generic_action_stack() {
        let frame = serde_json::json!([
            {
                "type": "component_tree",
                "root": {
                    "type": "column",
                    "padding": 24.0,
                    "children": [
                        {"type": "app_bar", "title": "Counter"},
                        scaffold_proof_body(),
                        {
                            "type": "stack",
                            "direction": "horizontal",
                            "children": [
                                {"type": "button", "node_id": "counter-increment", "label": "Increment"}
                            ]
                        },
                        {
                            "type": "pinned",
                            "edge": "bottom",
                            "child": {"type": "footer_keys", "entries": []}
                        }
                    ]
                }
            }
        ]);

        assert!(
            super::semantic_scaffold_chrome_errors(&frame)
                .iter()
                .any(|error| error.contains("missing semantic action_bar")),
            "{:?}",
            super::semantic_scaffold_chrome_errors(&frame)
        );
    }

    #[test]
    fn shell_layout_accepts_scaffold_frame() {
        let frame = serde_json::json!([
            {
                "type": "component_tree",
                "root": {
                    "type": "column",
                    "gap": 8.0,
                    "padding": 24.0,
                    "children": [
                        {"type": "app_bar", "title": "Counter"},
                        scaffold_proof_body(),
                        {
                            "type": "action_bar",
                            "actions": [
                                {"type": "button", "node_id": "counter-increment", "label": "Increment"}
                            ]
                        },
                        {
                            "type": "pinned",
                            "edge": "bottom",
                            "child": {"type": "footer_keys", "entries": []}
                        }
                    ]
                }
            }
        ]);

        assert!(
            super::scaffold_shell_layout_errors(&frame, 320, 240).is_empty(),
            "{:?}",
            super::scaffold_shell_layout_errors(&frame, 320, 240)
        );
    }

    #[test]
    fn shell_layout_rejects_short_scaffold_frame() {
        let frame = serde_json::json!([
            {
                "type": "component_tree",
                "root": {
                    "type": "column",
                    "gap": 8.0,
                    "padding": 24.0,
                    "children": [
                        {"type": "app_bar", "title": "Counter"},
                        scaffold_proof_body(),
                        {
                            "type": "action_bar",
                            "actions": [
                                {"type": "button", "node_id": "counter-increment", "label": "Increment"}
                            ]
                        },
                        {
                            "type": "pinned",
                            "edge": "bottom",
                            "child": {"type": "footer_keys", "entries": []}
                        }
                    ]
                }
            }
        ]);

        let errors = super::scaffold_shell_layout_errors(&frame, 320, 110);

        assert!(
            errors
                .iter()
                .any(|error| error.contains("action_bar overlaps footer")),
            "{errors:?}"
        );
    }

    #[test]
    fn action_expectation_increments_seeded_counter() {
        let state = serde_json::json!({"count": 3});

        assert_eq!(
            super::expected_action_signal(&state, "counter-increment"),
            Some("4".to_string())
        );
        assert_eq!(super::expected_action_signal(&state, "unknown"), None);
    }

    #[test]
    fn missing_seed_fixture_skips_cleanly() {
        let dir = TempDir::new().unwrap();

        let fixture = super::load_seed_fixture(dir.path()).unwrap();

        assert!(fixture.is_none());
    }

    #[test]
    fn invalid_seed_fixture_errors() {
        let dir = TempDir::new().unwrap();
        let fixtures = dir.path().join("fixtures");
        fs::create_dir_all(&fixtures).unwrap();
        fs::write(fixtures.join("state.json"), "[1, 2, 3]\n").unwrap();

        let err = super::load_seed_fixture(dir.path()).unwrap_err();

        assert!(err.contains("must be a plain JSON object"), "{err}");
    }
}

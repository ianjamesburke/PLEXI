use crate::app::registry::AppManifest;
use crate::host::wasm_app::UiTree;
use crate::host::wasm_python::PythonLaunchConfig;
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// Build a headless launch config for the live CPython-WASM runtime from an
/// already-loaded manifest, so `app check`/`app render` boot the app exactly
/// the way the live host does (`wasm_python::WasmPythonRuntime`), not a
/// separate native subprocess speaking a different wire protocol.
fn python_launch_config(
    manifest: &AppManifest,
    entry_path: &Path,
    app_dir: &Path,
) -> PythonLaunchConfig {
    python_launch_config_from_parts(
        &manifest.app.id,
        app_dir,
        entry_path,
        &manifest.app.capabilities.capabilities,
        &manifest.app.capabilities.allowed_hosts,
    )
}

/// Same as [`python_launch_config`], for callers that only have the pieces
/// (e.g. `plexi app render`'s registry-id lookup path, which resolves an
/// `AppManifestApp` rather than a full `AppManifest`).
pub(crate) fn python_launch_config_from_parts(
    app_id: &str,
    app_dir: &Path,
    entry_path: &Path,
    capabilities: &[String],
    allowed_hosts: &[String],
) -> PythonLaunchConfig {
    PythonLaunchConfig {
        app_id: app_id.to_string(),
        app_dir: app_dir.to_path_buf(),
        entry: entry_path.to_path_buf(),
        module_name: entry_path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("main")
            .to_string(),
        launch_args: Vec::new(),
        workspace_root: app_dir.to_path_buf(),
        capabilities: capabilities.to_vec(),
        allowed_hosts: allowed_hosts.to_vec(),
    }
}

/// Serialize a live `UiTree` to the same JSON shape used elsewhere in the
/// codebase (`{"id","key","data"}` per node) so downstream checks can walk it
/// generically. `data.type` matches the wire tag (`Text`, `Column`, ...).
pub(crate) fn ui_tree_to_json(tree: &UiTree) -> serde_json::Value {
    serde_json::json!({
        "root": tree.root,
        "nodes": tree.nodes.iter().map(indexed_node_to_json).collect::<Vec<_>>(),
    })
}

fn indexed_node_to_json(node: &crate::host::wasm_app::IndexedNode) -> serde_json::Value {
    use crate::host::wasm_app::UiNodeData;
    let data = match &node.data {
        UiNodeData::Empty => serde_json::json!({"type": "Empty"}),
        UiNodeData::Text(t) => serde_json::json!({"type": "Text", "text": t.text}),
        UiNodeData::Button(b) => {
            serde_json::json!({"type": "Button", "label": b.label, "on_click": b.on_click})
        }
        UiNodeData::TextInput(_) => serde_json::json!({"type": "TextInput"}),
        UiNodeData::Row(r) => serde_json::json!({"type": "Row", "children": r.children}),
        UiNodeData::Column(c) => serde_json::json!({"type": "Column", "children": c.children}),
        UiNodeData::ProgressBar(_) => serde_json::json!({"type": "ProgressBar"}),
        UiNodeData::Badge(b) => serde_json::json!({"type": "Badge", "text": b.text}),
        UiNodeData::ListView(l) => serde_json::json!({"type": "ListView", "items": l.items}),
        UiNodeData::Scroll(s) => serde_json::json!({"type": "Scroll", "child": s.child}),
        UiNodeData::Padding(p) => serde_json::json!({"type": "Padding", "child": p.child}),
        UiNodeData::Canvas(c) => serde_json::json!({
            "type": "Canvas", "width": c.width, "height": c.height, "commands": c.commands.len()
        }),
        UiNodeData::Divider => serde_json::json!({"type": "Divider"}),
        UiNodeData::Space(s) => {
            serde_json::json!({"type": "Space", "size": s.size, "grow": s.grow})
        }
        UiNodeData::Surface(_) => serde_json::json!({"type": "Surface"}),
        UiNodeData::AppBar(a) => {
            serde_json::json!({"type": "AppBar", "title": a.title, "subtitle": a.subtitle})
        }
        UiNodeData::FooterKeys(f) => serde_json::json!({
            "type": "FooterKeys",
            "entries": f.entries.iter().map(|e| serde_json::json!({"keys": e.keys, "description": e.description})).collect::<Vec<_>>(),
        }),
        UiNodeData::Pinned(p) => serde_json::json!({
            "type": "Pinned", "edge": format!("{:?}", p.edge).to_ascii_lowercase(), "child": p.child
        }),
        UiNodeData::Spinner(s) => serde_json::json!({"type": "Spinner", "label": s.label}),
    };
    serde_json::json!({"id": node.id, "key": node.key, "data": data})
}

const DEFAULT_CHECK_SIZES: &[(u32, u32)] = &[(320, 240), (480, 320), (800, 600), (1200, 800)];
const SEEDED_PROBE_SIZE: (u32, u32) = (480, 320);
/// Upper bound on how many discovered handlers the action probe clicks in one
/// sequence, keeping a button-heavy app's check fast. The probe prints how
/// many it skipped when an app exceeds it.
const MAX_ACTION_PROBE_HANDLERS: usize = 8;

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
    // #0336: reject an invalid `[launch] on_launch` here, the same way
    // `AppRegistry::load_app` does at install time. Without this, an author gets
    // a green `app check` and then a silent runtime skip (the launch resolver
    // treats an unparseable policy as the `always_new` default).
    match on_launch_error(&manifest) {
        Some(err) => errors.push(err),
        None => {
            if let Some(mode) = &manifest.launch.on_launch {
                println!("✓ launch — on_launch = {mode}");
            }
        }
    }
    warnings.extend(scaffold_metadata_warnings(app_dir));
    if scaffold_requires_semantic_chrome(app_dir) {
        warnings.push(
            "semantic chrome — this scaffold expects a semantic-chrome check, which is not yet \
             implemented against the live WIT UI-node model; skipping"
                .to_string(),
        );
    }

    let is_python_entry = entry_path.extension().and_then(|ext| ext.to_str()) == Some("py");
    if is_python_entry {
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
        // Static type gate (stint 0415): the AST analysis above proves shape,
        // mypy proves attribute/type correctness — e.g. `event.payload` on a
        // `UiValueChange` (which only has `.value`) fails here instead of
        // crashing the guest on the first keystroke.
        // Findings are errors for every Python app (stint 0457). The
        // template-version split that spared pre-gate scaffolds existed only
        // to land the gate without turning the maintained set red overnight;
        // that set is annotated now, so there is no warning tier left.
        match run_python_type_check(&manifest.app.id, &entry_path, &manifest.app.dependencies) {
            Ok(TypeCheckOutcome::Clean) => println!("✓ types — mypy clean"),
            Ok(TypeCheckOutcome::Findings(findings)) => {
                errors.push(format!("types — mypy found errors:\n{findings}"));
            }
            Ok(TypeCheckOutcome::Unavailable(reason)) => {
                warnings.push(format!("types — type-check skipped: {reason}"));
            }
            Err(e) => errors.push(format!("types — {e}")),
        }
    } else {
        warnings.push(format!(
            "SDK — {} is not a Python entry; skipping Python SDK checks",
            entry_path.display()
        ));
    }

    if !is_python_entry {
        warnings.push(format!(
            "render — {} is not a Python entry; the live-runtime checker only drives SDK v3 Python apps (CPython-in-WASM), skipping render checks",
            entry_path.display()
        ));
    }
    let launch_config =
        is_python_entry.then(|| python_launch_config(&manifest, &entry_path, app_dir));

    // A guest that fails to boot fails identically at every size; stop after
    // the first boot failure instead of paying the probe cost 4 more times
    // (stint 0458 — a broken first draft is the assistant loop's hot path).
    let mut render_boot_failed = false;
    let total_sizes = render_sizes.len();
    for (index, (width, height)) in render_sizes.into_iter().enumerate() {
        let label = format!("{width}x{height}");
        let Some(launch_config) = &launch_config else {
            continue;
        };
        match crate::host::wasm_python::run_headless_frame(
            launch_config,
            (width as f32, height as f32),
            None,
        ) {
            Ok(tree) => {
                let node_count = tree.nodes.len();
                if node_count == 0 {
                    errors.push(format!("render {label} — app emitted an empty frame"));
                    continue;
                }
                println!("✓ render {label} — {node_count} node(s)");
                if let Some(dir) = &png_dir {
                    let png_path = dir.join(format!("{}-{label}.png", manifest.app.id));
                    match crate::host::wasm_render::render_ui_tree_to_png(
                        &tree,
                        width as f32,
                        height as f32,
                        1.0,
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
            Err(e) => {
                errors.push(format!("render {label} — {e}"));
                render_boot_failed = true;
                let remaining = total_sizes - index - 1;
                if remaining > 0 {
                    println!(
                        "skip render — {remaining} remaining size(s) skipped after {label} failed to boot"
                    );
                }
                break;
            }
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
            if render_boot_failed {
                println!("skip seeded probe — app failed to boot during render checks");
            } else if let Some(launch_config) = &launch_config {
                run_seeded_state_and_action_probe(
                    &manifest,
                    launch_config,
                    &fixture,
                    &mut errors,
                    &mut warnings,
                );
            } else {
                warnings.push("seeded probe — skipped, not a Python entry".to_string());
            }
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
    launch_config: &PythonLaunchConfig,
    fixture: &SeedFixture,
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

    let frame = match crate::host::wasm_python::run_headless_frame(
        launch_config,
        (width as f32, height as f32),
        Some(fixture.state.clone()),
    ) {
        Ok(tree) => ui_tree_to_json(&tree),
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

    let discovered = discover_action_handlers(&frame);
    if discovered.is_empty() {
        println!("skip action probe — no clickable handler in seeded render");
        log::info!(
            "app_check[{}]: no clickable handler in seeded render",
            manifest.app.id
        );
        return;
    }
    let total = discovered.len();
    let handlers: Vec<String> = discovered
        .into_iter()
        .take(MAX_ACTION_PROBE_HANDLERS)
        .collect();
    if total > handlers.len() {
        println!(
            "action probe — probing first {} of {total} clickable handler(s)",
            handlers.len()
        );
    }

    match crate::host::wasm_python::run_headless_ui_action_sequence(
        launch_config,
        (width as f32, height as f32),
        Some(fixture.state.clone()),
        &handlers,
    ) {
        Err(e) => errors.push(format!("action probe — {e}")),
        Ok((before, outcomes)) => {
            let mut prev_frame = ui_tree_to_json(&before);
            let mut any_effect = false;
            let mut probe_failed = false;
            for (index, (handler_id, outcome)) in outcomes.iter().enumerate() {
                let action_label = format!("action probe {handler_id}");
                match outcome {
                    Err(e) => {
                        probe_failed = true;
                        errors.push(format!("{action_label} — {e}"));
                    }
                    Ok(after) => {
                        let after_frame = ui_tree_to_json(after);
                        // Seed-derived expectations predict the state after a
                        // single click from the fixture, so only the first
                        // action in the sequence can assert one.
                        let expected = (index == 0)
                            .then(|| expected_action_signal(&fixture.state, handler_id))
                            .flatten();
                        if let Some(expected) = expected {
                            if frame_contains_scalar(&after_frame, &expected) {
                                any_effect = true;
                                println!(
                                    "✓ {action_label} — rendered expected state value {expected}"
                                );
                                log::info!(
                                    "app_check[{}]: {action_label} rendered expected value {expected}",
                                    manifest.app.id
                                );
                            } else {
                                probe_failed = true;
                                errors.push(format!(
                                    "{action_label} — expected rendered state value {expected} after action"
                                ));
                            }
                        } else if after_frame != prev_frame {
                            any_effect = true;
                            println!("✓ {action_label} — rendered frame changed after action");
                            log::info!(
                                "app_check[{}]: {action_label} changed rendered frame",
                                manifest.app.id
                            );
                        } else {
                            println!("• {action_label} — frame unchanged");
                        }
                        prev_frame = after_frame;
                    }
                }
            }
            if !any_effect && !probe_failed {
                warnings.push(format!(
                    "action probe — clicked {} handler(s), no rendered frame ever changed",
                    outcomes.len()
                ));
            }
        }
    }
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

/// Every clickable handler id in the rendered frame, in render order, deduped.
/// Any node carrying a non-empty string `on_click` counts — the encoder emits
/// that key for buttons and any future clickable node type.
fn discover_action_handlers(frame: &serde_json::Value) -> Vec<String> {
    let mut handlers = Vec::new();
    collect_action_handlers(frame, &mut handlers);
    handlers
}

fn collect_action_handlers(value: &serde_json::Value, handlers: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(handler) = map.get("on_click").and_then(serde_json::Value::as_str) {
                if !handler.is_empty() && !handlers.iter().any(|known| known == handler) {
                    handlers.push(handler.to_string());
                }
            }
            for value in map.values() {
                collect_action_handlers(value, handlers);
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                collect_action_handlers(value, handlers);
            }
        }
        _ => {}
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

/// Validate a manifest's `[launch] on_launch` policy against the single source
/// of valid strings in the registry (#0336). Returns the error message when the
/// value is unknown, `None` when it is valid or unset. Reuses
/// [`OnLaunchPolicy::parse`](crate::app::registry::OnLaunchPolicy::parse) and
/// [`VALID_ON_LAUNCH`](crate::app::registry::VALID_ON_LAUNCH) so `app check` and
/// install-time validation never diverge.
fn on_launch_error(manifest: &AppManifest) -> Option<String> {
    let mode = manifest.launch.on_launch.as_deref()?;
    if crate::app::registry::OnLaunchPolicy::parse(mode).is_err() {
        Some(format!(
            "manifest [launch] on_launch = '{mode}' is not a valid policy; valid values: {}",
            crate::app::registry::VALID_ON_LAUNCH.join(", ")
        ))
    } else {
        None
    }
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
    scaffold_at_current_template(app_dir)
}

/// True when the app was scaffolded at (or above) the current template
/// version — the gate for checks that only hold for freshly-generated apps.
fn scaffold_at_current_template(app_dir: &Path) -> bool {
    let metadata_path = app_dir.join(crate::cli::app::SCAFFOLD_METADATA_FILE);
    let Ok(raw) = std::fs::read_to_string(&metadata_path) else {
        return false;
    };
    let Ok(metadata) = toml::from_str::<ScaffoldMetadata>(&raw) else {
        return false;
    };
    metadata.template_version >= crate::cli::app::PYTHON_SCAFFOLD_TEMPLATE_VERSION
}

/// Outcome of the static type-check pass over the app entry (stint 0415).
enum TypeCheckOutcome {
    Clean,
    /// mypy exited 1 with findings — the app has type errors.
    Findings(String),
    /// mypy is not importable and could not be installed (offline venv);
    /// surfaced as a named warning, never silently skipped.
    Unavailable(String),
}

fn run_python_type_check(
    app_id: &str,
    entry_path: &Path,
    python_dependencies: &[String],
) -> Result<TypeCheckOutcome, String> {
    let runtime = crate::app::python_env::resolve_python_runtime(
        app_id,
        entry_path,
        true,
        python_dependencies,
    )?;
    run_python_type_check_with(
        entry_path,
        Path::new(&runtime.executable),
        &crate::config::build_pythonpath(None),
    )
}

/// Run mypy over `entry_path` through the app venv's `python`, resolving
/// `plexi_sdk` from `pythonpath`. `--check-untyped-defs` is load-bearing:
/// scaffolded apps leave `update(event)` unannotated, and without the flag
/// mypy skips untyped function bodies entirely — the exact place authoring
/// mistakes live. `--follow-imports=silent` type-checks against the SDK's
/// annotations without reporting on SDK-internal code.
fn run_python_type_check_with(
    entry_path: &Path,
    python: &Path,
    pythonpath: &str,
) -> Result<TypeCheckOutcome, String> {
    // The venv resolver can hand back paths relative to the caller's cwd;
    // the mypy run below changes cwd to the app dir, so resolve everything
    // to absolute paths first.
    let entry_path = &entry_path
        .canonicalize()
        .map_err(|e| format!("could not resolve app entry {}: {e}", entry_path.display()))?;
    let python = &python.canonicalize().map_err(|e| {
        format!(
            "could not resolve app venv python {}: {e}",
            python.display()
        )
    })?;
    if let Err(probe) = probe_mypy(python) {
        // One-time install into the app venv, mirroring how app
        // dependencies land there (`python_env::install_dependencies`).
        let install = std::process::Command::new("uv")
            .args(["pip", "install", "--python"])
            .arg(python)
            .arg("mypy")
            .output();
        let installed = matches!(&install, Ok(out) if out.status.success());
        if !installed {
            return Ok(TypeCheckOutcome::Unavailable(format!(
                "mypy not importable ({probe}) and `uv pip install mypy` did not succeed"
            )));
        }
        probe_mypy(python).map_err(|e| format!("mypy installed but still not importable: {e}"))?;
        log::info!(
            "app_check: installed mypy into the app venv at {}",
            python.display()
        );
    }
    let app_dir = entry_path.parent().unwrap_or_else(|| Path::new("."));
    let cache_dir = app_dir.join(".venv").join("mypy_cache");
    let output = std::process::Command::new(python)
        .args([
            "-m",
            "mypy",
            "--no-error-summary",
            "--no-color-output",
            "--hide-error-context",
            "--follow-imports=silent",
            "--check-untyped-defs",
            "--cache-dir",
        ])
        .arg(&cache_dir)
        .arg(entry_path)
        .env("MYPYPATH", pythonpath)
        .current_dir(app_dir)
        .output()
        .map_err(|e| format!("failed to run mypy via {}: {e}", python.display()))?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    log::info!(
        "app_check: mypy over {} exited {:?} ({} finding bytes)",
        entry_path.display(),
        output.status.code(),
        stdout.len()
    );
    match output.status.code() {
        Some(0) => Ok(TypeCheckOutcome::Clean),
        Some(1) if !stdout.is_empty() => Ok(TypeCheckOutcome::Findings(stdout)),
        code => Err(format!(
            "mypy failed (exit {code:?}): {}",
            if stdout.is_empty() {
                String::from_utf8_lossy(&output.stderr).trim().to_string()
            } else {
                stdout
            }
        )),
    }
}

fn probe_mypy(python: &Path) -> Result<(), String> {
    let output = std::process::Command::new(python)
        .args(["-m", "mypy", "--version"])
        .output()
        .map_err(|e| format!("could not run {}: {e}", python.display()))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
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

#[cfg(test)]
mod app_check_tests {
    use std::fs;
    use tempfile::TempDir;

    fn manifest_with_on_launch(on_launch: &str) -> crate::app::registry::AppManifest {
        toml::from_str(&format!(
            "schema_version = 1\n\n[app]\nid = \"x\"\ntype = \"app\"\nname = \"X\"\n\
             version = \"0.0.1\"\nentry = \"main.py\"\n\n[launch]\non_launch = \"{on_launch}\"\n"
        ))
        .expect("manifest must parse")
    }

    /// #0336: `app check` must reject an unknown on_launch policy with a message
    /// naming the field and the valid values — matching install-time validation,
    /// so authors never get a green check followed by a silent runtime skip.
    #[test]
    fn on_launch_invalid_policy_is_rejected() {
        let manifest = manifest_with_on_launch("focus_maybe");
        let err = super::on_launch_error(&manifest)
            .expect("invalid on_launch must be reported as an error");
        assert!(
            err.contains("on_launch"),
            "message must name the field: {err}"
        );
        assert!(
            err.contains("focus_existing") && err.contains("always_new"),
            "message must list the valid values: {err}"
        );
    }

    #[test]
    fn on_launch_valid_policy_is_accepted() {
        for policy in ["focus_existing", "focus_existing_in_context", "always_new"] {
            let manifest = manifest_with_on_launch(policy);
            assert!(
                super::on_launch_error(&manifest).is_none(),
                "'{policy}' must be accepted"
            );
        }
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

    /// Discovery must find every clickable handler in render order without an
    /// allowlist — the mimo tictactoe run shipped a first-click crash because
    /// the old probe only recognized `counter-increment` and skipped the
    /// app's real `cell-*`/`reset` handlers entirely.
    #[test]
    fn discover_action_handlers_finds_all_unique_on_click_ids() {
        let frame = serde_json::json!({
            "root": 0,
            "nodes": [
                {"id": 0, "data": {"type": "Column", "children": [1, 2, 3, 4]}},
                {"id": 1, "data": {"type": "Button", "label": "1", "on_click": "cell-0"}},
                {"id": 2, "data": {"type": "Button", "label": "2", "on_click": "cell-1"}},
                {"id": 3, "data": {"type": "Button", "label": "again", "on_click": "cell-0"}},
                {"id": 4, "data": {"type": "Button", "label": "New Game", "on_click": "reset"}},
            ],
        });

        assert_eq!(
            super::discover_action_handlers(&frame),
            vec![
                "cell-0".to_string(),
                "cell-1".to_string(),
                "reset".to_string()
            ]
        );
    }

    #[test]
    fn discover_action_handlers_ignores_empty_and_missing_on_click() {
        let frame = serde_json::json!({
            "root": 0,
            "nodes": [
                {"id": 0, "data": {"type": "Text", "text": "hello"}},
                {"id": 1, "data": {"type": "Button", "label": "dead", "on_click": ""}},
                {"id": 2, "data": {"type": "Button", "label": "null", "on_click": null}},
            ],
        });

        assert!(super::discover_action_handlers(&frame).is_empty());
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

    /// The render loop at the bottom of `app_check_cli` maps
    /// `run_headless_frame`'s `Err(e)` to `errors.push(format!("render {label} — {e}"))`
    /// (never a silent log-and-continue), so a component tree that fails to
    /// decode — e.g. an app emitting `Badge(color="blue")`, which the host
    /// decoder rejects (see `decode_badge_color`) — must surface as a named
    /// `plexi app check` error, not merely an ERROR-level log line while the
    /// check itself reports success.
    #[test]
    fn render_decode_error_becomes_a_named_check_error() {
        let tree_json = r#"{"root":0,"nodes":[
            {"id":0,"key":"0","data":{"type":"badge","text":"status","color":"blue"}}
        ]}"#;

        let decode_err = crate::host::wasm_python::decode_ui_tree(tree_json)
            .expect_err("'blue' is not a valid badge color");

        let label = "480x320";
        let check_error = format!("render {label} — {decode_err}");
        assert!(
            check_error.contains("unknown badge color: blue"),
            "check error must name the bad value: {check_error}"
        );
    }
}

#[cfg(test)]
mod type_check_tests {
    use super::*;

    /// Stint 0415, both halves of the contract: (1) an app reading a field
    /// that doesn't exist on an SDK event (`UiValueChange.payload` — it only
    /// has `.value`) fails the type gate instead of crashing the guest at
    /// runtime; (2) a mixed-children `Column([Text, Button])` type-checks
    /// clean now that container params are covariant `Sequence`. Runs mypy
    /// through the repo venv's python; skips (loudly) when that venv isn't
    /// synced so a fresh clone's `cargo test` doesn't require Python setup.
    #[test]
    fn mypy_gate_flags_wrong_event_attribute_and_accepts_mixed_children() {
        let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let python = manifest_dir.join(".venv/bin/python");
        if !python.exists() || probe_mypy(&python).is_err() {
            eprintln!("skipping: repo venv python with mypy not available");
            return;
        }
        let sdk_path = manifest_dir.join("sdk/python");
        let sdk = sdk_path.to_string_lossy();

        let dir = tempfile::tempdir().unwrap();
        let bad_dir = dir.path().join("bad");
        std::fs::create_dir_all(&bad_dir).unwrap();
        let bad = bad_dir.join("main.py");
        std::fs::write(
            &bad,
            "from plexi_sdk.events import UiValueChange\n\n\
             def update(event):\n\
             \x20   if isinstance(event, UiValueChange):\n\
             \x20       return [event.payload.get(\"value\")]\n\
             \x20   return []\n",
        )
        .unwrap();
        match run_python_type_check_with(&bad, &python, &sdk).expect("mypy run") {
            TypeCheckOutcome::Findings(findings) => assert!(
                findings.contains("payload"),
                "findings must name the bogus attribute: {findings}"
            ),
            TypeCheckOutcome::Clean => {
                panic!("`event.payload` on UiValueChange must fail the type gate")
            }
            TypeCheckOutcome::Unavailable(reason) => panic!("mypy vanished mid-test: {reason}"),
        }

        let good_dir = dir.path().join("good");
        std::fs::create_dir_all(&good_dir).unwrap();
        let good = good_dir.join("main.py");
        std::fs::write(
            &good,
            "from plexi_sdk.ui import Button, Column, Text\n\n\
             def view():\n\
             \x20   return Column([Text(\"hello\"), Button(\"go\", \"on-go\")])\n",
        )
        .unwrap();
        match run_python_type_check_with(&good, &python, &sdk).expect("mypy run") {
            TypeCheckOutcome::Clean => {}
            TypeCheckOutcome::Findings(findings) => panic!(
                "mixed-children Column must type-check clean under Sequence covariance: {findings}"
            ),
            TypeCheckOutcome::Unavailable(reason) => panic!("mypy vanished mid-test: {reason}"),
        }
    }
}

use crate::app::registry::PluginManifest;
use crate::host::wasm_python::PythonLaunchConfig;
use crate::host::wasm_view::UiTree;
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// Build a headless launch config for the live CPython-WASM runtime from an
/// already-loaded manifest, so `plugin check`/`view render` boot the plugin exactly
/// the way the live host does (`wasm_python::WasmPythonRuntime`), not a
/// separate native subprocess speaking a different wire protocol.
pub(crate) fn python_launch_config(
    manifest: &PluginManifest,
    entry_path: &Path,
    plugin_dir: &Path,
) -> Result<PythonLaunchConfig, String> {
    Ok(python_launch_config_from_parts(
        &manifest.plugin.id,
        plugin_dir,
        entry_path,
        &manifest.plugin.capabilities.capabilities,
        &manifest.plugin.capabilities.allowed_hosts,
        manifest
            .state_scopes()
            .map_err(|e| format!("manifest [state] scopes: {e}"))?,
        manifest.state_format()?,
    ))
}

/// Same as [`python_launch_config`], for callers that only have the pieces
/// (e.g. `plexi view render`'s registry-id lookup path, which resolves an
/// installed plugin rather than a full `PluginManifest`).
pub(crate) fn python_launch_config_from_parts(
    plugin_id: &str,
    plugin_dir: &Path,
    entry_path: &Path,
    capabilities: &[String],
    allowed_hosts: &[String],
    state_scopes: Vec<crate::host::state_scope::StateScope>,
    state_format: crate::host::state_scope::StateFormat,
) -> PythonLaunchConfig {
    log::info!(
        "plugin_check[{plugin_id}]: headless launch declares state scopes {:?} format {}",
        state_scopes.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
        state_format.as_str()
    );
    PythonLaunchConfig {
        plugin_id: plugin_id.to_string(),
        plugin_dir: plugin_dir.to_path_buf(),
        entry: entry_path.to_path_buf(),
        module_name: entry_path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("main")
            .to_string(),
        launch_args: Vec::new(),
        workspace_root: plugin_dir.to_path_buf(),
        capabilities: capabilities.to_vec(),
        allowed_hosts: allowed_hosts.to_vec(),
        theme: crate::ui::theme::colors_from_config(
            &crate::config::PlexiConfig::load_with_workspace(
                crate::config::active_workspace_root().as_deref(),
            ),
        )
        .to_theme_map(),
        // The SDK validates every `scope=` argument against this list, so it
        // must be the manifest's. Headless runs never persist state; the
        // plugin-dir anchor only gives the scopes somewhere to resolve.
        state_scopes,
        state_format,
        context_root: plugin_dir.to_path_buf(),
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

fn indexed_node_to_json(node: &crate::host::wasm_view::IndexedNode) -> serde_json::Value {
    use crate::host::wasm_view::UiNodeData;
    let data = match &node.data {
        UiNodeData::Empty => serde_json::json!({"type": "Empty"}),
        UiNodeData::Text(t) => serde_json::json!({"type": "Text", "text": t.text}),
        UiNodeData::Button(b) => {
            serde_json::json!({"type": "Button", "label": b.label, "on_click": b.on_click})
        }
        UiNodeData::TextInput(t) => serde_json::json!({
            "type": "TextInput", "value": t.value, "placeholder": t.placeholder
        }),
        UiNodeData::Row(r) => serde_json::json!({"type": "Row", "children": r.children}),
        UiNodeData::Column(c) => serde_json::json!({"type": "Column", "children": c.children}),
        UiNodeData::ProgressBar(p) => serde_json::json!({"type": "ProgressBar", "label": p.label}),
        UiNodeData::Badge(b) => serde_json::json!({"type": "Badge", "text": b.text}),
        UiNodeData::ListView(l) => serde_json::json!({"type": "ListView", "items": l.items}),
        UiNodeData::Scroll(s) => serde_json::json!({"type": "Scroll", "child": s.child}),
        UiNodeData::Padding(p) => serde_json::json!({"type": "Padding", "child": p.child}),
        UiNodeData::Canvas(c) => serde_json::json!({
            "type": "Canvas", "width": c.width, "height": c.height, "commands": c.commands.len(),
            "texts": c.commands.iter().filter_map(|command| match command {
                crate::host::wasm_view::bindings::plexi::platform::types::CanvasCommand::Text(t) => {
                    Some(t.text.as_str())
                }
                _ => None,
            }).collect::<Vec<_>>(),
        }),
        UiNodeData::Divider => serde_json::json!({"type": "Divider"}),
        UiNodeData::Space(s) => {
            serde_json::json!({"type": "Space", "size": s.size, "grow": s.grow})
        }
        UiNodeData::Surface(_) => serde_json::json!({"type": "Surface"}),
        UiNodeData::SurfaceHeader(a) => {
            serde_json::json!({"type": "SurfaceHeader", "title": a.title, "subtitle": a.subtitle})
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
/// sequence, keeping a button-heavy plugin's check fast. The probe prints how
/// many it skipped when a plugin exceeds it.
const MAX_ACTION_PROBE_HANDLERS: usize = 8;

#[derive(Debug, Default)]
struct SdkAnalysis {
    has_init: bool,
    has_update: bool,
    has_view: bool,
    legacy_plugin_classes: Vec<String>,
    errors: Vec<String>,
    warnings: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct PythonAstReport {
    functions: Vec<String>,
    legacy_plugin_classes: Vec<String>,
    warnings: Vec<String>,
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
    signals: Vec<SeedSignal>,
}

/// One scalar from `fixtures/state.json`, with the path that names it.
#[derive(Debug, Clone, PartialEq)]
struct SeedSignal {
    key: String,
    value: SeedValue,
}

#[derive(Debug, Clone, PartialEq)]
enum SeedValue {
    Text(String),
    Number(String),
}

impl std::fmt::Display for SeedValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Text(text) => write!(f, "{text:?}"),
            Self::Number(number) => f.write_str(number),
        }
    }
}

impl std::fmt::Display for SeedSignal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} = {}", self.key, self.value)
    }
}

fn join_signals(signals: &[SeedSignal]) -> String {
    signals
        .iter()
        .map(SeedSignal::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn plugin_check_cli(path: &str, sizes: &[String], png_dir: Option<&str>) -> i32 {
    let plugin_dir = Path::new(path);
    log::info!(
        "plugin_check: path={} sizes={sizes:?} png_dir={png_dir:?}",
        plugin_dir.display()
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

    let (manifest, entry_path) = match load_local_plugin(plugin_dir) {
        Ok(app) => app,
        Err(e) => {
            println!("✗ manifest — {e}");
            return 1;
        }
    };

    println!(
        "✓ manifest — {} ({})",
        manifest.plugin.id, manifest.plugin.name
    );
    for command in &manifest.contributes.commands {
        println!(
            "✓ command — {} → {} (reserved declaration; dispatch not yet available)",
            command.id, command.handler
        );
    }
    // #0336: reject an invalid `[launch] on_launch` here, the same way
    // `PluginRegistry::load_plugin` does at install time. Without this, an author gets
    // a green `plugin check` and then a silent runtime skip (the launch resolver
    // treats an unparseable policy as the `always_new` default).
    match on_launch_error(&manifest) {
        Some(err) => errors.push(err),
        None => {
            if let Some(mode) = &manifest.launch.on_launch {
                println!("✓ launch — on_launch = {mode}");
            }
        }
    }
    // Stint 0751: mcp.client spawns an unsandboxed process on the host's
    // behalf, so it is beta-gated (`ReleaseFeature::McpClient`). A stable
    // build cannot resolve this check on its own binary tier, so surface it
    // as a warning here rather than only discovering it at runtime.
    if manifest
        .plugin
        .capabilities
        .capabilities
        .iter()
        .any(|c| c == "mcp.client")
        && !crate::release::feature_enabled(crate::release::ReleaseFeature::McpClient)
    {
        warnings.push(format!(
            "mcp.client — {}",
            crate::release::feature_unavailable_message(crate::release::ReleaseFeature::McpClient)
        ));
    }
    warnings.extend(scaffold_metadata_warnings(plugin_dir));

    let is_python_entry = entry_path.extension().and_then(|ext| ext.to_str()) == Some("py");
    if is_python_entry {
        match analyze_python_entry(
            &manifest.plugin.id,
            &entry_path,
            &manifest.plugin.dependencies,
        ) {
            Ok(analysis) => {
                if analysis.has_init && analysis.has_update && analysis.has_view {
                    println!("✓ SDK — module-level init/update/view");
                }
                if !analysis.legacy_plugin_classes.is_empty() {
                    for class in &analysis.legacy_plugin_classes {
                        errors.push(format!(
                            "SDK — {class} subclasses legacy App; SDK v3 plugins use module-level init/update/view functions"
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
        // Findings are errors for every Python plugin (stint 0457). The
        // template-version split that spared pre-gate scaffolds existed only
        // to land the gate without turning the maintained set red overnight;
        // that set is annotated now, so there is no warning tier left.
        match run_python_type_check(
            &manifest.plugin.id,
            &entry_path,
            &manifest.plugin.dependencies,
        ) {
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
            "render — {} is not a Python entry; the live-runtime checker only drives SDK v3 Python plugins (CPython-in-WASM), skipping render checks",
            entry_path.display()
        ));
    }
    let launch_config = if is_python_entry {
        match python_launch_config(&manifest, &entry_path, plugin_dir) {
            Ok(config) => Some(config),
            Err(e) => {
                errors.push(format!(
                    "{} — {e}",
                    plugin_dir.join("manifest.toml").display()
                ));
                None
            }
        }
    } else {
        None
    };

    // A guest that fails to boot fails identically at every size; stop after
    // the first boot failure instead of paying the probe cost 4 more times
    // (stint 0458 — a broken first draft is the assistant loop's hot path).
    let mut render_boot_failed = false;
    let plan = plan_check_sizes(
        render_sizes,
        (manifest.launch.min_width, manifest.launch.min_height),
    );
    for (width, height) in &plan.skipped {
        println!(
            "skip render {width}x{height} — below the manifest's [launch] minimum pane size; the host shows its \"too small\" placeholder there"
        );
    }
    log::info!(
        "plugin_check[{}]: rendering {:?}, skipped {:?} below the manifest minimum, unreachable controls are errors at {:?}",
        manifest.plugin.id,
        plan.render,
        plan.skipped,
        plan.strict
    );
    if plan.render.is_empty() && launch_config.is_some() {
        warnings.push(
            "render — every requested size is below the manifest's [launch] minimum pane size; nothing was rendered. Pass a larger --size".to_string(),
        );
    }
    let strict_size = plan.strict;
    let total_sizes = plan.render.len();
    for (index, (width, height)) in plan.render.into_iter().enumerate() {
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
                    errors.push(format!("render {label} — plugin emitted an empty frame"));
                    continue;
                }
                println!("✓ render {label} — {node_count} node(s)");
                let layout = crate::host::wasm_render::render_ui_tree_layout(
                    &tree,
                    width as f32,
                    height as f32,
                );
                report_layout_findings(
                    &manifest.plugin.id,
                    &label,
                    layout_findings(&tree, &layout, strict_size == Some((width, height))),
                    &mut errors,
                    &mut warnings,
                );
                if let Some(dir) = &png_dir {
                    let png_path = dir.join(format!("{}-{label}.png", manifest.plugin.id));
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

    match load_seed_fixture(plugin_dir) {
        Ok(Some(fixture)) => {
            println!(
                "✓ state fixture — loaded {}",
                fixture
                    .path
                    .strip_prefix(plugin_dir)
                    .unwrap_or(&fixture.path)
                    .display()
            );
            if render_boot_failed {
                println!("skip seeded probe — plugin failed to boot during render checks");
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
                "plugin_check[{}]: no fixtures/state.json; seeded probes skipped",
                manifest.plugin.id
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
        println!("✓ plugin check passed");
        log::info!(
            "plugin_check[{}]: passed with {} warning(s)",
            manifest.plugin.id,
            warnings.len()
        );
        0
    } else {
        println!(
            "✗ plugin check failed — {} error(s), {} warning(s)",
            errors.len(),
            warnings.len()
        );
        log::warn!(
            "plugin_check[{}]: failed with {} error(s), {} warning(s)",
            manifest.plugin.id,
            errors.len(),
            warnings.len()
        );
        1
    }
}

fn load_seed_fixture(plugin_dir: &Path) -> Result<Option<SeedFixture>, String> {
    let path = plugin_dir.join("fixtures").join("state.json");
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
    manifest: &PluginManifest,
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

    // A value proves the seed reached the view only when the seed is what put
    // it on screen, so every signal is judged against the same frame without
    // the seed: static copy that happens to contain the value never counts.
    let unseeded = match crate::host::wasm_python::run_headless_frame(
        launch_config,
        (width as f32, height as f32),
        None,
    ) {
        Ok(tree) => ui_tree_to_json(&tree),
        Err(e) => {
            errors.push(format!(
                "seeded render {label} — unseeded comparison render failed: {e}"
            ));
            return;
        }
    };
    let matched = matched_seed_signals(&frame, &unseeded, &fixture.signals);
    if matched.is_empty() {
        let seed_ignored = unseeded == frame;
        log::info!(
            "plugin_check[{}]: seeded render {label} matched no fixture value; seed_ignored={seed_ignored}",
            manifest.plugin.id
        );
        errors.push(seed_miss_error(&label, &fixture.signals, seed_ignored));
        return;
    }
    println!(
        "✓ seeded render {label} — reflected fixture value(s): {}",
        join_signals(&matched)
    );
    log::info!(
        "plugin_check[{}]: seeded render {label} reflected {}",
        manifest.plugin.id,
        join_signals(&matched)
    );

    let discovered = discover_action_handlers(&frame);
    if discovered.is_empty() {
        println!("skip action probe — no clickable handler in seeded render");
        log::info!(
            "plugin_check[{}]: no clickable handler in seeded render",
            manifest.plugin.id
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
                            let expected_value = SeedValue::Number(expected.clone());
                            // Same differential rule as the seed: the click
                            // must be what put the number on screen.
                            if frame_shows(&after_frame, &expected_value)
                                && !frame_shows(&prev_frame, &expected_value)
                            {
                                any_effect = true;
                                println!(
                                    "✓ {action_label} — rendered expected state value {expected}"
                                );
                                log::info!(
                                    "plugin_check[{}]: {action_label} rendered expected value {expected}",
                                    manifest.plugin.id
                                );
                            } else {
                                probe_failed = true;
                                errors.push(format!(
                                    "{action_label} — expected the action to put state value {expected} on screen (shown after the click and not before it)"
                                ));
                            }
                        } else if after_frame != prev_frame {
                            any_effect = true;
                            println!("✓ {action_label} — rendered frame changed after action");
                            log::info!(
                                "plugin_check[{}]: {action_label} changed rendered frame",
                                manifest.plugin.id
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

/// The error for a seeded render that showed no fixture value: which file,
/// which values were looked for, the rule they are matched by, and, when the
/// frame is identical without the seed, the likeliest cause.
fn seed_miss_error(label: &str, signals: &[SeedSignal], seed_ignored: bool) -> String {
    let mut message = format!(
        "seeded render {label} — no value from fixtures/state.json appeared in the rendered frame because of the seed. Looked for: {}. Rule: a value counts when displayed text shows it with the seed and does not show it without; a string matches when displayed text contains it, ignoring case; a number matches as a whole number. Displayed text is Text, Button, Badge, list rows, headers, TextInput value and canvas text. Use fixture values that differ from the plugin's defaults and static copy.",
        join_signals(signals)
    );
    if seed_ignored {
        message.push_str(
            " The seed had no visible effect: the frame is identical without it; does init() overwrite these keys with SetState? Read the seeded value in init() (state.get) before returning defaults.",
        );
    }
    message
}

fn seed_state_signals(state: &serde_json::Value) -> Vec<SeedSignal> {
    let mut signals = Vec::new();
    collect_scalar_signals(state, "", &mut signals);
    signals.sort_by(|a, b| a.key.cmp(&b.key));
    signals.dedup();
    signals
}

fn collect_scalar_signals(value: &serde_json::Value, path: &str, signals: &mut Vec<SeedSignal>) {
    match value {
        serde_json::Value::String(s) if !s.trim().is_empty() => signals.push(SeedSignal {
            key: path.to_string(),
            value: SeedValue::Text(s.clone()),
        }),
        serde_json::Value::Number(n) => signals.push(SeedSignal {
            key: path.to_string(),
            value: SeedValue::Number(n.to_string()),
        }),
        serde_json::Value::Array(values) => {
            for (index, value) in values.iter().enumerate() {
                collect_scalar_signals(value, &format!("{path}[{index}]"), signals);
            }
        }
        serde_json::Value::Object(map) => {
            for (key, value) in map {
                let child = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}.{key}")
                };
                collect_scalar_signals(value, &child, signals);
            }
        }
        _ => {}
    }
}

/// Signals the seed put on screen: shown in the seeded frame and absent from
/// the unseeded one.
fn matched_seed_signals(
    seeded: &serde_json::Value,
    unseeded: &serde_json::Value,
    signals: &[SeedSignal],
) -> Vec<SeedSignal> {
    signals
        .iter()
        .filter(|signal| {
            frame_shows(seeded, &signal.value) && !frame_shows(unseeded, &signal.value)
        })
        .cloned()
        .collect()
}

/// Frame JSON fields that hold text showing plugin state. Everything else is
/// structure (type tags, keys, handler ids, sizes) or static affordance copy
/// (TextInput placeholders, footer key chips) and never satisfies a seed.
const DISPLAYED_FIELDS: &[&str] = &[
    "text",
    "label",
    "value",
    "title",
    "subtitle",
    "texts",
    "description",
];

/// True when the frame displays `expected` somewhere a user would read it.
fn frame_shows(frame: &serde_json::Value, expected: &SeedValue) -> bool {
    match frame {
        serde_json::Value::Array(values) => values.iter().any(|value| frame_shows(value, expected)),
        serde_json::Value::Object(map) => map.iter().any(|(key, value)| {
            if DISPLAYED_FIELDS.contains(&key.as_str()) && displayed_matches(value, expected) {
                return true;
            }
            frame_shows(value, expected)
        }),
        _ => false,
    }
}

fn displayed_matches(value: &serde_json::Value, expected: &SeedValue) -> bool {
    match (value, expected) {
        (serde_json::Value::String(shown), SeedValue::Text(text)) => {
            shown.to_lowercase().contains(&text.to_lowercase())
        }
        (serde_json::Value::String(shown), SeedValue::Number(number)) => {
            contains_whole_number(shown, number)
        }
        (serde_json::Value::Number(shown), SeedValue::Number(number)) => {
            shown.to_string() == *number
        }
        (serde_json::Value::Array(values), _) => values
            .iter()
            .any(|value| displayed_matches(value, expected)),
        _ => false,
    }
}

/// `number` occurs in `shown` as a whole number: not as digits inside a longer
/// number, so a seeded `3` is not satisfied by "13", "30" or "1.3".
fn contains_whole_number(shown: &str, number: &str) -> bool {
    let bytes = shown.as_bytes();
    shown.match_indices(number).any(|(start, matched)| {
        let end = start + matched.len();
        let digit_at = |index: usize| bytes.get(index).is_some_and(u8::is_ascii_digit);
        let before_ok = start == 0
            || !(digit_at(start - 1)
                || (bytes[start - 1] == b'.' && start >= 2 && digit_at(start - 2)));
        let after_ok = !(digit_at(end) || (bytes.get(end) == Some(&b'.') && digit_at(end + 1)));
        before_ok && after_ok
    })
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

/// The smallest pane a plugin is expected to work in unless its manifest sets
/// a larger `[launch]` minimum. At the smallest size checked at or above this
/// floor, a control that cannot be reached is an error, because nothing on
/// screen tells the user it exists.
const SMALLEST_CHECK_SIZE: (u32, u32) = (320, 240);
/// Less of a control than this, in points on either axis, is not a usable
/// click target.
const MIN_REACHABLE_EXTENT: f32 = 16.0;

#[derive(Debug, PartialEq)]
struct SizePlan {
    render: Vec<(u32, u32)>,
    /// Requested sizes below the manifest minimum, where the host shows its
    /// "too small" placeholder instead of the plugin.
    skipped: Vec<(u32, u32)>,
    /// The size at which an unreachable control is an error.
    strict: Option<(u32, u32)>,
}

fn plan_check_sizes(sizes: Vec<(u32, u32)>, manifest_min: (Option<f32>, Option<f32>)) -> SizePlan {
    let min_width = manifest_min.0.unwrap_or(0.0);
    let min_height = manifest_min.1.unwrap_or(0.0);
    let (render, skipped): (Vec<_>, Vec<_>) = sizes
        .into_iter()
        .partition(|(width, height)| *width as f32 >= min_width && *height as f32 >= min_height);
    let strict = render
        .iter()
        .copied()
        .filter(|(width, height)| {
            *width >= SMALLEST_CHECK_SIZE.0 && *height >= SMALLEST_CHECK_SIZE.1
        })
        .min_by_key(|(width, height)| (u64::from(*width) * u64::from(*height), *width));
    SizePlan {
        render,
        skipped,
        strict,
    }
}
/// Painted geometry may differ by this many points before it counts as cut
/// or overlapping; strokes and glyph ink sit slightly outside widget rects.
const LAYOUT_TOLERANCE: f32 = 2.0;
/// Findings printed per size before the rest are summarized as a count.
const MAX_LAYOUT_FINDINGS_PER_SIZE: usize = 8;

#[derive(Debug, Clone, PartialEq)]
struct LayoutFinding {
    is_error: bool,
    message: String,
}

fn report_layout_findings(
    plugin_id: &str,
    label: &str,
    findings: Vec<LayoutFinding>,
    errors: &mut Vec<String>,
    warnings: &mut Vec<String>,
) {
    log::info!(
        "plugin_check[{plugin_id}]: layout {label} produced {} finding(s), {} error(s)",
        findings.len(),
        findings.iter().filter(|f| f.is_error).count()
    );
    let total = findings.len();
    // Errors first, so the per-size cap never hides one behind warnings.
    let (failing, advisory): (Vec<_>, Vec<_>) = findings.into_iter().partition(|f| f.is_error);
    for finding in failing
        .into_iter()
        .chain(advisory)
        .take(MAX_LAYOUT_FINDINGS_PER_SIZE)
    {
        let line = format!("layout {label} — {}", finding.message);
        if finding.is_error {
            errors.push(line);
        } else {
            warnings.push(line);
        }
    }
    if total > MAX_LAYOUT_FINDINGS_PER_SIZE {
        warnings.push(format!(
            "layout {label} — {} more layout finding(s) not shown; fix the ones above and rerun",
            total - MAX_LAYOUT_FINDINGS_PER_SIZE
        ));
    }
}

fn clip_text(text: &str) -> String {
    const MAX: usize = 40;
    if text.chars().count() <= MAX {
        text.to_string()
    } else {
        format!("{}…", text.chars().take(MAX).collect::<String>())
    }
}

/// Node type plus the text a plugin author would recognize it by.
fn describe_node(node: &crate::host::wasm_view::IndexedNode) -> String {
    use crate::host::wasm_view::UiNodeData;
    let (kind, text) = match &node.data {
        UiNodeData::Empty => ("Empty", None),
        UiNodeData::Text(t) => ("Text", Some(t.text.as_str())),
        UiNodeData::Button(b) => ("Button", Some(b.label.as_str())),
        UiNodeData::TextInput(t) => (
            "TextInput",
            Some(if t.placeholder.is_empty() {
                t.value.as_str()
            } else {
                t.placeholder.as_str()
            }),
        ),
        UiNodeData::Row(_) => ("Row", None),
        UiNodeData::Column(_) => ("Column", None),
        UiNodeData::ProgressBar(p) => ("ProgressBar", p.label.as_deref()),
        UiNodeData::Badge(b) => ("Badge", Some(b.text.as_str())),
        UiNodeData::ListView(_) => ("ListView", None),
        UiNodeData::Scroll(_) => ("Scroll", None),
        UiNodeData::Padding(_) => ("Padding", None),
        UiNodeData::Canvas(_) => ("Canvas", None),
        UiNodeData::Divider => ("Divider", None),
        UiNodeData::Space(_) => ("Space", None),
        UiNodeData::Surface(_) => ("Surface", None),
        UiNodeData::SurfaceHeader(h) => ("SurfaceHeader", Some(h.title.as_str())),
        UiNodeData::FooterKeys(_) => ("FooterKeys", None),
        UiNodeData::Pinned(_) => ("Pinned", None),
        UiNodeData::Spinner(s) => ("Spinner", Some(s.label.as_str())),
    };
    match text.filter(|text| !text.is_empty()) {
        Some(text) => format!("{kind} \"{}\"", clip_text(text)),
        None if node.key.is_empty() || node.key.parse::<u64>().is_ok() => {
            format!("{kind} (node {})", node.id)
        }
        None => format!("{kind} (key {})", clip_text(&node.key)),
    }
}

fn node_children(data: &crate::host::wasm_view::UiNodeData) -> Vec<u32> {
    use crate::host::wasm_view::UiNodeData;
    match data {
        UiNodeData::Row(r) => r.children.clone(),
        UiNodeData::Column(c) => c.children.clone(),
        UiNodeData::ListView(l) => l.items.clone(),
        UiNodeData::Scroll(s) => vec![s.child],
        UiNodeData::Padding(p) => vec![p.child],
        UiNodeData::Pinned(p) => vec![p.child],
        _ => Vec::new(),
    }
}

/// Turn the headless pass's painted geometry into findings an author can act
/// on. Pure over its inputs: the renderer measured, this only judges.
///
/// Nodes under a `Scroll` or `ListView` are exempt from the visibility rules,
/// since scrolling reaches them. A node the pass has no entry for is unknown
/// and never reported. With `strict`, a Button or TextInput that cannot be
/// reached (nothing of it visible, or too little to click) is an error; a
/// control that is only partly cut still warns.
fn layout_findings(
    tree: &UiTree,
    layout: &[crate::host::wasm_render::NodeLayout],
    strict: bool,
) -> Vec<LayoutFinding> {
    use crate::host::wasm_view::UiNodeData;
    use std::collections::{HashMap, HashSet};

    let facts: HashMap<u32, &crate::host::wasm_render::NodeLayout> =
        layout.iter().map(|fact| (fact.id, fact)).collect();
    let nodes: HashMap<u32, &crate::host::wasm_view::IndexedNode> =
        tree.nodes.iter().map(|node| (node.id, node)).collect();

    // Every node below a scrolling container, walked from those containers.
    let mut scrollable: HashSet<u32> = HashSet::new();
    let mut pending: Vec<u32> = tree
        .nodes
        .iter()
        .filter(|node| matches!(node.data, UiNodeData::Scroll(_) | UiNodeData::ListView(_)))
        .map(|node| node.id)
        .collect();
    while let Some(id) = pending.pop() {
        if !scrollable.insert(id) {
            continue;
        }
        if let Some(node) = nodes.get(&id) {
            pending.extend(node_children(&node.data));
        }
    }

    let mut findings = Vec::new();

    for node in &tree.nodes {
        let Some(fact) = facts.get(&node.id) else {
            continue;
        };
        let paints = match &node.data {
            UiNodeData::Text(t) => !t.text.trim().is_empty(),
            UiNodeData::Button(_)
            | UiNodeData::TextInput(_)
            | UiNodeData::Badge(_)
            | UiNodeData::ProgressBar(_)
            | UiNodeData::SurfaceHeader(_)
            | UiNodeData::Spinner(_) => true,
            UiNodeData::FooterKeys(f) => !f.entries.is_empty(),
            _ => false,
        };
        if !paints {
            continue;
        }
        let interactive = matches!(node.data, UiNodeData::Button(_) | UiNodeData::TextInput(_));
        let name = describe_node(node);

        if matches!(node.data, UiNodeData::Button(_)) && fact.text_wrapped_mid_word {
            findings.push(LayoutFinding {
                is_error: false,
                message: format!(
                    "{name}: the label breaks in the middle of a word; shorten the label, or give the row fewer buttons at this width"
                ),
            });
        }

        if scrollable.contains(&node.id) {
            continue;
        }
        const UNREACHABLE: &str =
            " The control cannot be reached at the smallest pane size checked, so this is an error.";
        let (Some(rect), Some(visible)) = (fact.rect, fact.visible) else {
            let is_error = strict && interactive;
            findings.push(LayoutFinding {
                is_error,
                message: format!(
                    "{name} is not visible: it was laid out outside its container or with zero size; move it above any grow Spacer, or put the content that pushes it out inside Scrollable(...).{}",
                    if is_error { UNREACHABLE } else { "" }
                ),
            });
            continue;
        };
        let hidden = [
            (visible.min.x - rect.min.x, "left"),
            (rect.max.x - visible.max.x, "right"),
            (visible.min.y - rect.min.y, "top"),
            (rect.max.y - visible.max.y, "bottom"),
        ];
        let cut: Vec<String> = hidden
            .iter()
            .filter(|(amount, _)| *amount > LAYOUT_TOLERANCE)
            .map(|(amount, side)| format!("{}px on the {side}", amount.round()))
            .collect();
        if !cut.is_empty() {
            let is_error = strict
                && interactive
                && (visible.width() < MIN_REACHABLE_EXTENT
                    || visible.height() < MIN_REACHABLE_EXTENT);
            findings.push(LayoutFinding {
                is_error,
                message: format!(
                    "{name} is cut off ({}, {}x{}px still showing); shorten it, stack the row vertically at this width, or put its container inside Scrollable(...).{}",
                    cut.join(", "),
                    visible.width().round(),
                    visible.height().round(),
                    if is_error { UNREACHABLE } else { "" }
                ),
            });
        }
    }

    for parent in &tree.nodes {
        if !matches!(parent.data, UiNodeData::Row(_) | UiNodeData::Column(_)) {
            continue;
        }
        let placed: Vec<(&crate::host::wasm_view::IndexedNode, egui::Rect)> =
            node_children(&parent.data)
                .into_iter()
                .filter_map(|id| {
                    let node = nodes.get(&id)?;
                    if matches!(
                        node.data,
                        UiNodeData::Empty | UiNodeData::Space(_) | UiNodeData::Divider
                    ) {
                        return None;
                    }
                    Some((*node, facts.get(&id)?.visible?))
                })
                .collect();
        for (index, (first, first_rect)) in placed.iter().enumerate() {
            for (second, second_rect) in &placed[index + 1..] {
                let shared = first_rect.intersect(*second_rect);
                if shared.width() > LAYOUT_TOLERANCE && shared.height() > LAYOUT_TOLERANCE {
                    findings.push(LayoutFinding {
                        is_error: false,
                        message: format!(
                            "{} and {} overlap by {}x{}px; children of one {} must not share space (a Canvas drawing outside its own rect, or a second grow Spacer in a Column, does this)",
                            describe_node(first),
                            describe_node(second),
                            shared.width().round(),
                            shared.height().round(),
                            describe_node(parent)
                        ),
                    });
                }
            }
        }
    }

    findings
}

fn rect_json(rect: egui::Rect) -> serde_json::Value {
    let round = |value: f32| (f64::from(value) * 10.0).round() / 10.0;
    serde_json::json!([
        round(rect.min.x),
        round(rect.min.y),
        round(rect.width()),
        round(rect.height())
    ])
}

/// The frame JSON `view render` prints: the tree plus, per measured node, the
/// rect it painted (`[x, y, width, height]`), the visible part of that rect
/// and its clip rect, so a caller can check layout without reading a PNG.
pub(crate) fn frame_json_with_layout(
    tree: &UiTree,
    layout: &[crate::host::wasm_render::NodeLayout],
) -> serde_json::Value {
    let mut frame = ui_tree_to_json(tree);
    frame["layout"] = layout
        .iter()
        .map(|fact| {
            serde_json::json!({
                "id": fact.id,
                "rect": fact.rect.map(rect_json),
                "visible": fact.visible.map(rect_json),
                "clip": rect_json(fact.clip),
                "text_wrapped_mid_word": fact.text_wrapped_mid_word,
                "text_cut": fact.text_cut,
            })
        })
        .collect();
    frame
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
/// [`VALID_ON_LAUNCH`](crate::app::registry::VALID_ON_LAUNCH) so `plugin check` and
/// install-time validation never diverge.
fn on_launch_error(manifest: &PluginManifest) -> Option<String> {
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

pub(super) fn load_local_plugin(plugin_dir: &Path) -> Result<(PluginManifest, PathBuf), String> {
    if !plugin_dir.exists() {
        return Err(format!("path does not exist: {}", plugin_dir.display()));
    }
    if !plugin_dir.is_dir() {
        return Err(format!("path is not a directory: {}", plugin_dir.display()));
    }

    let manifest_path = plugin_dir.join("manifest.toml");
    let manifest_raw = std::fs::read_to_string(&manifest_path)
        .map_err(|e| format!("could not read {}: {e}", manifest_path.display()))?;
    let manifest =
        PluginManifest::parse(&manifest_raw).map_err(|e| format!("invalid manifest.toml: {e}"))?;

    if manifest.schema_version > crate::app::registry::MANIFEST_SCHEMA_VERSION {
        return Err(format!(
            "manifest schema_version {} is newer than this Plexi build supports ({})",
            manifest.schema_version,
            crate::app::registry::MANIFEST_SCHEMA_VERSION
        ));
    }

    let entry_path = plugin_dir.join(&manifest.plugin.entry);
    if !entry_path.exists() {
        return Err(format!(
            "entry file '{}' not found in {}",
            manifest.plugin.entry,
            plugin_dir.display()
        ));
    }

    Ok((manifest, entry_path))
}

fn scaffold_metadata_warnings(plugin_dir: &Path) -> Vec<String> {
    let metadata_path = plugin_dir.join(crate::cli::plugin::SCAFFOLD_METADATA_FILE);
    let mut warnings = Vec::new();

    let raw = match std::fs::read_to_string(&metadata_path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            warnings.push(format!(
                "scaffold metadata — missing {}; old scaffolds are still supported, but run `plexi plugin init --adopt <dir>` (or regenerate with `plexi plugin init`) before relying on drift checks",
                crate::cli::plugin::SCAFFOLD_METADATA_FILE
            ));
            log::warn!(
                "plugin_check: missing scaffold metadata at {}",
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
                "plugin_check: could not read scaffold metadata at {}: {e}",
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
                "plugin_check: invalid scaffold metadata at {}: {e}",
                metadata_path.display()
            );
            return warnings;
        }
    };

    let current_cli = env!("CARGO_PKG_VERSION");
    let current_sdk = crate::cli::plugin::python_sdk_version();
    let current_manifest_schema = crate::app::registry::MANIFEST_SCHEMA_VERSION;
    let current_runtime = crate::app::python_env::PYTHON_APP_VENV_VERSION;
    let current_template = crate::cli::plugin::PYTHON_SCAFFOLD_TEMPLATE_VERSION;
    let (current_channel, current_profile_dir) = crate::cli::plugin::current_scaffold_channel();

    if metadata.schema_version != crate::cli::plugin::SCAFFOLD_METADATA_SCHEMA_VERSION {
        warnings.push(format!(
            "scaffold metadata — schema_version {} differs from supported {}; run `plexi plugin init --adopt <dir>` or update {}",
            metadata.schema_version,
            crate::cli::plugin::SCAFFOLD_METADATA_SCHEMA_VERSION,
            crate::cli::plugin::SCAFFOLD_METADATA_FILE
        ));
    }
    if metadata.plexi_cli_version != current_cli {
        warnings.push(format!(
            "scaffold metadata — generated by Plexi CLI {}, checking with {}; run `plexi plugin check` on the intended build or refresh with `plexi plugin init --adopt <dir>`",
            metadata.plexi_cli_version, current_cli
        ));
    }
    if metadata.sdk_version != current_sdk {
        warnings.push(format!(
            "scaffold metadata — generated for Python SDK {}, current SDK is {}; rerun tests/checks with the intended channel or refresh with `plexi plugin init --adopt <dir>`",
            metadata.sdk_version, current_sdk
        ));
    }
    if metadata.manifest_schema_version != current_manifest_schema {
        warnings.push(format!(
            "scaffold metadata — manifest schema {} differs from current {}; update manifest.toml or refresh with `plexi plugin init --adopt <dir>`",
            metadata.manifest_schema_version, current_manifest_schema
        ));
    }
    if metadata.python_runtime_version != current_runtime {
        warnings.push(format!(
            "scaffold metadata — generated for Python {}, current plugin venv runtime is {}; recreate the venv or refresh with `plexi plugin init --adopt <dir>`",
            metadata.python_runtime_version, current_runtime
        ));
    }
    if metadata.template_version != current_template {
        warnings.push(format!(
            "scaffold metadata — generated from template {}, current template is {}; refresh with `plexi plugin init --adopt <dir>` to pick up scaffold guidance",
            metadata.template_version, current_template
        ));
    }
    if metadata.channel != current_channel || metadata.profile_dir != current_profile_dir {
        warnings.push(format!(
            "scaffold metadata — generated for channel/profile {}/{}, checking under {}/{}; use an explicit matching PLEXI_CHANNEL or refresh with `plexi plugin init --adopt <dir>`",
            metadata.channel, metadata.profile_dir, current_channel, current_profile_dir
        ));
    }

    if warnings.is_empty() {
        println!(
            "✓ scaffold metadata — {} template {}",
            metadata.sdk_version, metadata.template_version
        );
        log::info!(
            "plugin_check: scaffold metadata current at {}",
            metadata_path.display()
        );
    } else {
        log::warn!(
            "plugin_check: scaffold metadata produced {} warning(s) at {}",
            warnings.len(),
            metadata_path.display()
        );
    }

    warnings
}

/// Outcome of the static type-check pass over the plugin entry (stint 0415).
enum TypeCheckOutcome {
    Clean,
    /// mypy exited 1 with findings — the plugin has type errors.
    Findings(String),
    /// mypy is not importable and could not be installed (offline venv);
    /// surfaced as a named warning, never silently skipped.
    Unavailable(String),
}

fn run_python_type_check(
    plugin_id: &str,
    entry_path: &Path,
    python_dependencies: &[String],
) -> Result<TypeCheckOutcome, String> {
    let runtime = crate::app::python_env::resolve_python_runtime(
        plugin_id,
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

/// Run mypy over `entry_path` through the plugin venv's `python`, resolving
/// `plexi_sdk` from `pythonpath`. `--check-untyped-defs` is load-bearing:
/// scaffolded plugins leave `update(event)` unannotated, and without the flag
/// mypy skips untyped function bodies entirely — the exact place authoring
/// mistakes live. `--follow-imports=silent` type-checks against the SDK's
/// annotations without reporting on SDK-internal code.
fn run_python_type_check_with(
    entry_path: &Path,
    python: &Path,
    pythonpath: &str,
) -> Result<TypeCheckOutcome, String> {
    // The venv resolver can hand back paths relative to the caller's cwd;
    // the mypy run below changes cwd to the plugin dir, so resolve everything
    // to absolute paths first.
    let entry_path = &entry_path.canonicalize().map_err(|e| {
        format!(
            "could not resolve plugin entry {}: {e}",
            entry_path.display()
        )
    })?;
    let python = &python.canonicalize().map_err(|e| {
        format!(
            "could not resolve plugin venv python {}: {e}",
            python.display()
        )
    })?;
    if let Err(probe) = probe_mypy(python) {
        // One-time install into the plugin venv, mirroring how plugin
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
            "plugin_check: installed mypy into the plugin venv at {}",
            python.display()
        );
    }
    let plugin_dir = entry_path.parent().unwrap_or_else(|| Path::new("."));
    let cache_dir = plugin_dir.join(".venv").join("mypy_cache");
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
        .current_dir(plugin_dir)
        .output()
        .map_err(|e| format!("failed to run mypy via {}: {e}", python.display()))?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    log::info!(
        "plugin_check: mypy over {} exited {:?} ({} finding bytes)",
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
    plugin_id: &str,
    entry_path: &Path,
    python_dependencies: &[String],
) -> Result<SdkAnalysis, String> {
    let runtime = crate::app::python_env::resolve_python_runtime(
        plugin_id,
        entry_path,
        true,
        python_dependencies,
    )?;
    analyze_python_entry_with(entry_path, Path::new(&runtime.executable))
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

def child_list(call):
    for keyword in call.keywords:
        if keyword.arg in ("children", "actions", "buttons"):
            return keyword.value
    return call.args[0] if call.args else None

def view_warnings(view, component_state_names):
    warnings = []
    container_names = {"Column", "HStack", "Row", "Card", "Grid", "Section", "Actions", "ActionBar"}
    component_state_names = set(component_state_names)
    for node in ast.walk(view):
        if isinstance(node, ast.Assign) and isinstance(node.value, ast.Call) and base_name(node.value.func) == "component":
            component_state_names.update(target.id for target in node.targets if isinstance(target, ast.Name))
    for node in ast.walk(view):
        if not isinstance(node, ast.Call):
            continue
        if isinstance(node.func, ast.Attribute) and node.func.attr in ("set", "patch"):
            owner = node.func.value
            if isinstance(owner, ast.Name) and (owner.id == "state" or owner.id in component_state_names):
                warnings.append(f"view state mutation at line {node.lineno} — return state effects from update(), never call setters in view()")
        if base_name(node.func) not in container_names:
            continue
        children = child_list(node)
        if isinstance(children, (ast.List, ast.Tuple)):
            continue
        if not isinstance(children, (ast.ListComp, ast.GeneratorExp, ast.SetComp)):
            continue
        item = children.elt
        is_keyed = (
            isinstance(item, ast.Call)
            and base_name(item.func) == "Keyed"
            and any(keyword.arg == "key" for keyword in item.keywords)
        )
        if not is_keyed:
            warnings.append(
                f"generated children at line {node.lineno} must wrap each item in Keyed(..., key=...)"
            )
    return warnings

functions = []
legacy_plugin_classes = []
warnings = []
component_state_names = set()
for node in tree.body:
    if isinstance(node, ast.Assign) and isinstance(node.value, ast.Call) and base_name(node.value.func) == "component":
        component_state_names.update(target.id for target in node.targets if isinstance(target, ast.Name))
for node in tree.body:
    if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
        functions.append(node.name)
        if node.name == "view":
            warnings.extend(view_warnings(node, component_state_names))
    if isinstance(node, ast.ClassDef):
        if any(base_name(base) == "App" for base in node.bases):
            legacy_plugin_classes.append(node.name)

json.dump({"functions": functions, "legacy_plugin_classes": legacy_plugin_classes, "warnings": warnings}, sys.stdout)
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

    let mut analysis = SdkAnalysis {
        has_init: report.functions.iter().any(|name| name == "init"),
        has_update: report.functions.iter().any(|name| name == "update"),
        has_view: report.functions.iter().any(|name| name == "view"),
        legacy_plugin_classes: report.legacy_plugin_classes,
        warnings: report.warnings,
        ..Default::default()
    };

    for required in ["init", "update", "view"] {
        if !report.functions.iter().any(|name| name == required) {
            analysis.errors.push(format!(
                "SDK — missing module-level {required}(); SDK v3 plugins define init(size, args), update(event), and view()"
            ));
        }
    }

    Ok(analysis)
}

#[cfg(test)]
mod plugin_check_tests {
    use std::fs;
    use tempfile::TempDir;

    fn manifest_with_on_launch(on_launch: &str) -> crate::app::registry::PluginManifest {
        toml::from_str(&format!(
            "schema_version = 2\n\n[launch]\non_launch = \"{on_launch}\"\n\n[plugin]\nid = \"x\"\nname = \"X\"\nversion = \"0.0.1\"\n\n[runtime]\nkind = \"python\"\nentry = \"main.py\"\n\n[[contributes.views]]\nid = \"x.main\"\ndefault = true\n"
        ))
        .expect("manifest must parse")
    }

    /// #0336: `plugin check` must reject an unknown on_launch policy with a message
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
    fn sdk_analysis_accepts_module_level_v3_plugin() {
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
    fn sdk_analysis_rejects_legacy_plugin_subclass() {
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
                .legacy_plugin_classes
                .iter()
                .any(|class| class == "Bad"),
            "{:?}",
            analysis.legacy_plugin_classes
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
    fn sdk_analysis_warns_about_unkeyed_generated_children_and_view_state_mutation() {
        let dir = TempDir::new().unwrap();
        let entry = dir.path().join("main.py");
        fs::write(
            &entry,
            r#"
from plexi_sdk import state
from plexi_sdk.ui import Button, Column

def init(size, args): return []
def update(event): return []
def view():
    state.set("count", 1)
    return Column([Button(item) for item in ["one", "two"]])
"#,
        )
        .unwrap();

        let analysis = super::analyze_python_entry_with(&entry, std::path::Path::new("python3"))
            .expect("analysis should run");
        assert!(
            analysis
                .warnings
                .iter()
                .any(|warning| warning.contains("state mutation")),
            "{:?}",
            analysis.warnings
        );
        assert!(
            analysis
                .warnings
                .iter()
                .any(|warning| warning.contains("wrap each item in Keyed(..., key=...)")),
            "{:?}",
            analysis.warnings
        );
    }

    #[test]
    fn sdk_analysis_skips_fixed_literal_children_and_accepts_keyed_comprehensions() {
        let dir = TempDir::new().unwrap();
        let entry = dir.path().join("main.py");
        fs::write(
            &entry,
            r#"
from plexi_sdk.ui import Button, Column, Keyed

def init(size, args): return []
def update(event): return []
def view():
    fixed = Column([Button("one"), Button("two")])
    generated = Column([Keyed(Button(item), key=item) for item in ["three", "four"]])
    return Column([fixed, generated])
"#,
        )
        .unwrap();

        let analysis = super::analyze_python_entry_with(&entry, std::path::Path::new("python3"))
            .expect("analysis should run");
        assert!(analysis.warnings.is_empty(), "{:?}", analysis.warnings);
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
            dir.path().join(crate::cli::plugin::SCAFFOLD_METADATA_FILE),
            r#"schema_version = 1
generated_by = "plexi plugin init"
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
    fn plugin_adopt_writes_scaffold_metadata_without_touching_sources() {
        let _channel = crate::config::set_test_channel("alpha");
        let dir = TempDir::new().unwrap();
        let manifest_content = "schema_version = 2\n\n[plugin]\nid = \"adopted\"\nname = \"Adopted\"\nversion = \"0.1.0\"\n\n[runtime]\nkind = \"python\"\nentry = \"custom.py\"\n\n[[contributes.views]]\nid = \"adopted.main\"\ndefault = true\n";
        let entry_content = "# custom source preserved\ndef init(size, args): return []\ndef update(event): return []\ndef view(): return None\n";
        fs::write(dir.path().join("manifest.toml"), manifest_content).unwrap();
        fs::write(dir.path().join("custom.py"), entry_content).unwrap();

        let code = crate::cli::plugin::plugin_adopt(dir.path().to_str().unwrap());
        assert_eq!(code, 0);

        let metadata_path = dir.path().join(crate::cli::plugin::SCAFFOLD_METADATA_FILE);
        assert!(metadata_path.exists());
        let metadata_raw = fs::read_to_string(&metadata_path).unwrap();
        assert!(metadata_raw.contains("generated_by = \"plexi plugin init --adopt\""));
        assert_eq!(
            fs::read_to_string(dir.path().join("manifest.toml")).unwrap(),
            manifest_content
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("custom.py")).unwrap(),
            entry_content
        );
        assert!(!dir.path().join("main.py").exists());
        assert!(super::scaffold_metadata_warnings(dir.path()).is_empty());
    }

    #[test]
    fn seed_state_signals_collect_scalar_fixture_values() {
        let state = serde_json::json!({
            "count": 3,
            "mode": "demo",
            "nested": {"items": [7]},
        });

        let signals = super::seed_state_signals(&state);

        let shown: Vec<String> = signals.iter().map(ToString::to_string).collect();
        assert_eq!(
            shown,
            vec!["count = 3", "mode = \"demo\"", "nested.items[0] = 7"]
        );
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

        let number = |n: &str| super::SeedValue::Number(n.to_string());
        assert!(!super::frame_shows(&frame, &number("3")));
        assert!(super::frame_shows(&frame, &number("13")));
        // Structure is not display: the padding value never satisfies a seed.
        assert!(!super::frame_shows(&frame, &number("24")));
    }

    /// Discovery must find every clickable handler in render order without an
    /// allowlist — the mimo tictactoe run shipped a first-click crash because
    /// the old probe only recognized `counter-increment` and skipped the
    /// plugin's real `cell-*`/`reset` handlers entirely.
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

    /// The render loop at the bottom of `plugin_check_cli` maps
    /// `run_headless_frame`'s `Err(e)` to `errors.push(format!("render {label} — {e}"))`
    /// (never a silent log-and-continue), so a component tree that fails to
    /// decode — e.g. a plugin emitting `Badge(color="blue")`, which the host
    /// decoder rejects (see `decode_badge_color`) — must surface as a named
    /// `plexi plugin check` error, not merely an ERROR-level log line while the
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

    /// Stint 0415, both halves of the contract: (1) a plugin reading a field
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

#[cfg(test)]
mod gauntlet_probe_tests {
    fn tree(json: &str) -> crate::host::wasm_view::UiTree {
        crate::host::wasm_python::decode_ui_tree(json).expect("fixture tree decodes")
    }

    #[test]
    fn frame_json_includes_text_input_value_and_placeholder() {
        let frame = super::ui_tree_to_json(&tree(
            r#"{"root":0,"nodes":[
                {"id":0,"key":"0","data":{"type":"text_input","value":"calm","placeholder":"Mood"}}
            ]}"#,
        ));
        assert_eq!(frame["nodes"][0]["data"]["value"], "calm", "{frame}");
        assert_eq!(frame["nodes"][0]["data"]["placeholder"], "Mood", "{frame}");
    }

    #[test]
    fn frame_json_includes_canvas_text_strings() {
        let frame = super::ui_tree_to_json(&tree(
            r##"{"root":0,"nodes":[
                {"id":0,"key":"0","data":{"type":"canvas","width":100,"height":50,"commands":[
                    {"type":"rect","x":0,"y":0,"width":10,"height":10,"fill":"#000000"},
                    {"type":"text","x":4,"y":4,"text":"Score 42","color":"#ffffff"}
                ]}}
            ]}"##,
        ));
        assert_eq!(
            frame["nodes"][0]["data"]["texts"],
            serde_json::json!(["Score 42"]),
            "{frame}"
        );
    }

    fn text(value: &str) -> super::SeedValue {
        super::SeedValue::Text(value.to_string())
    }

    fn number(value: &str) -> super::SeedValue {
        super::SeedValue::Number(value.to_string())
    }

    #[test]
    fn seeded_string_match_is_case_insensitive_containment() {
        let frame =
            serde_json::json!({"nodes": [{"data": {"type": "Text", "text": "Mood: Calm"}}]});
        assert!(super::frame_shows(&frame, &text("calm")));
        assert!(!super::frame_shows(&frame, &text("tense")));
        // A type tag or handler id is structure, not something the user reads.
        assert!(!super::frame_shows(&frame, &text("text")));
    }

    #[test]
    fn seeded_value_shown_only_in_a_text_input_or_canvas_matches() {
        let input = super::ui_tree_to_json(&tree(
            r#"{"root":0,"nodes":[
                {"id":0,"key":"0","data":{"type":"text_input","value":"Buy milk"}}
            ]}"#,
        ));
        assert!(super::frame_shows(&input, &text("buy milk")));

        let canvas = super::ui_tree_to_json(&tree(
            r##"{"root":0,"nodes":[
                {"id":0,"key":"0","data":{"type":"canvas","width":100,"height":50,"commands":[
                    {"type":"text","x":4,"y":4,"text":"Score 42","color":"#ffffff"}
                ]}}
            ]}"##,
        ));
        assert!(super::frame_shows(&canvas, &number("42")));
        // The canvas's own width never satisfies a seeded 100.
        assert!(!super::frame_shows(&canvas, &number("100")));
    }

    #[test]
    fn seeded_number_matches_whole_numbers_inside_text_only() {
        for (shown, expected) in [
            ("Count: 3", true),
            ("3 items", true),
            ("3.", true),
            ("13", false),
            ("30", false),
            ("1.3", false),
            ("3.5", false),
        ] {
            assert_eq!(
                super::contains_whole_number(shown, "3"),
                expected,
                "{shown:?}"
            );
        }
    }

    /// A fixture value that static copy already shows proves nothing: it has
    /// to appear because of the seed.
    #[test]
    fn seed_value_already_in_static_copy_does_not_count() {
        let frame = |count: &str| {
            serde_json::json!({"nodes": [
                {"data": {"type": "Text", "text": "Step 1 of 3"}},
                {"data": {"type": "TextInput", "value": "", "placeholder": "Search…"}},
                {"data": {"type": "Text", "text": count}},
            ]})
        };
        let signals =
            super::seed_state_signals(&serde_json::json!({"count": 1, "query": "search"}));

        // The view ignores the seed entirely: "1" and "Search…" are static.
        let matched = super::matched_seed_signals(&frame("Count: 0"), &frame("Count: 0"), &signals);
        assert!(matched.is_empty(), "{matched:?}");

        // A placeholder is affordance copy, never state.
        assert!(!super::frame_shows(&frame("Count: 0"), &text("search")));

        // A seed the view does show counts, by the value it put on screen.
        let signals = super::seed_state_signals(&serde_json::json!({"count": 7}));
        let matched = super::matched_seed_signals(&frame("Count: 7"), &frame("Count: 0"), &signals);
        assert_eq!(super::join_signals(&matched), "count = 7");
    }

    #[test]
    fn seed_miss_error_names_the_fixture_values_rule_and_overwrite_hint() {
        let signals = super::seed_state_signals(&serde_json::json!({"mood": "calm", "count": 3}));

        let plain = super::seed_miss_error("480x320", &signals, false);
        assert!(plain.contains("fixtures/state.json"), "{plain}");
        assert!(
            plain.contains("count = 3") && plain.contains("mood = \"calm\""),
            "{plain}"
        );
        assert!(plain.contains("ignoring case"), "{plain}");
        assert!(!plain.contains("init()"), "{plain}");

        let ignored = super::seed_miss_error("480x320", &signals, true);
        assert!(
            ignored.contains("does init() overwrite these keys with SetState?"),
            "{ignored}"
        );
    }

    const TWO_SCOPE_MANIFEST: &str = "schema_version = 2\n\n[plugin]\nid = \"memo\"\nname = \"Memo\"\nversion = \"0.0.1\"\n\n[runtime]\nkind = \"python\"\nentry = \"main.py\"\n\n[state]\nscopes = [\"context\", \"global\"]\n\n[[contributes.views]]\nid = \"memo.main\"\ndefault = true\n";

    #[test]
    fn headless_launch_config_forwards_manifest_state_scopes() {
        let manifest: crate::app::registry::PluginManifest =
            toml::from_str(TWO_SCOPE_MANIFEST).expect("manifest parses");
        let dir = std::path::Path::new("/tmp/memo");
        let config = super::python_launch_config(&manifest, &dir.join("main.py"), dir)
            .expect("valid [state] section");
        assert_eq!(
            config.state_scopes,
            vec![
                crate::host::state_scope::StateScope::Context,
                crate::host::state_scope::StateScope::Global
            ]
        );
    }

    #[test]
    fn headless_seed_is_flat_for_one_scope_and_scope_keyed_for_several() {
        use crate::host::state_scope::StateScope;
        use crate::host::wasm_python::headless_seed_states;

        let flat = serde_json::json!({"count": 3});
        assert_eq!(
            headless_seed_states(&[StateScope::Global], Some(flat.clone())).unwrap(),
            serde_json::json!({"global": {"count": 3}})
        );

        let both = [StateScope::Context, StateScope::Global];
        let keyed = serde_json::json!({"context": {"memo": "a"}, "global": {"name": "b"}});
        assert_eq!(
            headless_seed_states(&both, Some(keyed.clone())).unwrap(),
            keyed
        );

        let err = headless_seed_states(&both, Some(flat)).unwrap_err();
        assert!(
            err.contains("context, global") && err.contains("count"),
            "error must list the declared scopes and the stray key: {err}"
        );
    }

    /// End to end through the live CPython-WASM runtime: a plugin that reads
    /// both declared scopes in `view()` boots headlessly, and a scope-keyed
    /// seed reaches each scope.
    #[test]
    fn two_scope_plugin_renders_headlessly_with_a_seed_per_scope() {
        let dir = tempfile::tempdir().expect("plugin dir");
        std::fs::write(dir.path().join("manifest.toml"), TWO_SCOPE_MANIFEST).unwrap();
        std::fs::write(
            dir.path().join("main.py"),
            "from plexi_sdk import state\nfrom plexi_sdk.ui import Column, Text\n\n\ndef init(size, args):\n    return []\n\n\ndef update(event):\n    return []\n\n\ndef view():\n    return Column([\n        Text(str(state.get(\"memo\", \"no memo\", scope=\"context\"))),\n        Text(str(state.get(\"owner\", \"nobody\", scope=\"global\"))),\n    ])\n",
        )
        .unwrap();
        let (manifest, entry) = super::load_local_plugin(dir.path()).expect("fixture loads");
        let config = super::python_launch_config(&manifest, &entry, dir.path()).unwrap();

        let unseeded = crate::host::wasm_python::run_headless_frame(&config, (480.0, 320.0), None)
            .expect("a two-scope plugin must boot headlessly");
        let unseeded = super::ui_tree_to_json(&unseeded);
        assert!(
            super::frame_shows(&unseeded, &text("no memo")),
            "{unseeded}"
        );

        let seeded = crate::host::wasm_python::run_headless_frame(
            &config,
            (480.0, 320.0),
            Some(serde_json::json!({
                "context": {"memo": "water the ficus"},
                "global": {"owner": "Ada"},
            })),
        )
        .expect("scope-keyed seed renders");
        let seeded = super::ui_tree_to_json(&seeded);
        assert!(
            super::frame_shows(&seeded, &text("water the ficus"))
                && super::frame_shows(&seeded, &text("Ada")),
            "both scopes must be seeded: {seeded}"
        );
    }
}

#[cfg(test)]
mod layout_finding_tests {
    use crate::host::wasm_render::NodeLayout;
    use egui::{pos2, Rect};

    fn tree(json: &str) -> crate::host::wasm_view::UiTree {
        crate::host::wasm_python::decode_ui_tree(json).expect("fixture tree decodes")
    }

    const PANE: Rect = Rect {
        min: pos2(0.0, 0.0),
        max: pos2(320.0, 240.0),
    };

    fn painted(id: u32, rect: Rect) -> NodeLayout {
        let visible = rect.intersect(PANE);
        NodeLayout {
            id,
            rect: Some(rect),
            visible: visible.is_positive().then_some(visible),
            clip: PANE,
            text_wrapped_mid_word: false,
            text_cut: false,
        }
    }

    fn unpainted(id: u32) -> NodeLayout {
        NodeLayout {
            id,
            rect: None,
            visible: None,
            clip: PANE,
            text_wrapped_mid_word: false,
            text_cut: false,
        }
    }

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::from_min_size(pos2(x, y), egui::vec2(w, h))
    }

    fn messages(findings: &[super::LayoutFinding]) -> String {
        findings
            .iter()
            .map(|f| {
                format!(
                    "[{}] {}",
                    if f.is_error { "error" } else { "warning" },
                    f.message
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    const ACTIONS_TREE: &str = r#"{"root":0,"nodes":[
        {"id":0,"key":"0","data":{"type":"column","children":[1,2]}},
        {"id":1,"key":"1","data":{"type":"text","text":"Timer"}},
        {"id":2,"key":"2","data":{"type":"row","children":[3,4]}},
        {"id":3,"key":"3","data":{"type":"button","label":"Start","on_click":"start"}},
        {"id":4,"key":"4","data":{"type":"button","label":"Reset","on_click":"reset"}}
    ]}"#;

    /// Gauntlet 02: an Actions row after a second grow Spacer was never painted.
    #[test]
    fn unpainted_actions_row_is_reported_per_button() {
        let layout = vec![
            painted(0, rect(0.0, 0.0, 320.0, 240.0)),
            painted(1, rect(24.0, 12.0, 60.0, 18.0)),
            unpainted(2),
            unpainted(3),
            unpainted(4),
        ];
        let findings = super::layout_findings(&tree(ACTIONS_TREE), &layout, false);
        let text = messages(&findings);
        assert!(text.contains("Button \"Start\" is not visible"), "{text}");
        assert!(text.contains("Button \"Reset\" is not visible"), "{text}");
        assert!(
            findings.iter().all(|f| !f.is_error),
            "800x600 only warns: {text}"
        );
    }

    /// Gauntlet 10: text drawn on top of buttons.
    #[test]
    fn overlapping_siblings_are_reported_by_name() {
        let layout = vec![
            painted(0, rect(0.0, 0.0, 320.0, 240.0)),
            painted(1, rect(24.0, 12.0, 120.0, 18.0)),
            painted(2, rect(24.0, 10.0, 200.0, 28.0)),
            painted(3, rect(24.0, 10.0, 90.0, 28.0)),
            painted(4, rect(122.0, 10.0, 90.0, 28.0)),
        ];
        let text = messages(&super::layout_findings(&tree(ACTIONS_TREE), &layout, false));
        assert!(
            text.contains("Text \"Timer\" and Row (node 2) overlap by 120x18px"),
            "{text}"
        );
        assert!(
            !text.contains("Button \"Start\" and Button \"Reset\""),
            "{text}"
        );
    }

    /// Gauntlet 05: an EmptyState pushed the only button off a 320x240 pane.
    /// A control with nothing showing, or too little to click, is an error at
    /// the strict size. One that is only partly cut still works, so it warns.
    #[test]
    fn unreachable_button_at_the_smallest_size_is_an_error() {
        let layout = vec![
            painted(0, rect(0.0, 0.0, 320.0, 240.0)),
            painted(1, rect(24.0, 12.0, 60.0, 18.0)),
            painted(2, rect(24.0, 220.0, 200.0, 28.0)),
            painted(3, rect(24.0, 220.0, 90.0, 28.0)),
            unpainted(4),
        ];
        let tree = tree(ACTIONS_TREE);
        let small = super::layout_findings(&tree, &layout, true);
        let text = messages(&small);
        assert!(
            text.contains(
                "[warning] Button \"Start\" is cut off (8px on the bottom, 90x20px still showing)"
            ),
            "{text}"
        );
        assert!(
            text.contains("[error] Button \"Reset\" is not visible"),
            "{text}"
        );
        assert!(text.contains("cannot be reached"), "{text}");

        let larger = super::layout_findings(&tree, &layout, false);
        assert!(larger.iter().all(|f| !f.is_error), "{}", messages(&larger));

        let sliver = vec![
            painted(0, rect(0.0, 0.0, 320.0, 240.0)),
            painted(3, rect(24.0, 234.0, 90.0, 28.0)),
        ];
        let text = messages(&super::layout_findings(&tree, &sliver, true));
        assert!(
            text.contains(
                "[error] Button \"Start\" is cut off (22px on the bottom, 90x6px still showing)"
            ),
            "{text}"
        );
    }

    #[test]
    fn check_sizes_skip_below_the_manifest_minimum_and_pick_the_strict_size() {
        let defaults = super::DEFAULT_CHECK_SIZES.to_vec();

        let plain = super::plan_check_sizes(defaults.clone(), (None, None));
        assert_eq!(plain.render, defaults);
        assert_eq!(plain.strict, Some((320, 240)));

        // A plugin that declares a 400px minimum is never shown at 320 wide.
        let wide = super::plan_check_sizes(defaults, (Some(400.0), None));
        assert_eq!(wide.skipped, vec![(320, 240)]);
        assert_eq!(wide.strict, Some((480, 320)));

        // Below the 320x240 floor nothing is strict; the next size up is.
        let custom = super::plan_check_sizes(vec![(300, 200), (640, 480)], (None, None));
        assert_eq!(custom.render, vec![(300, 200), (640, 480)]);
        assert_eq!(custom.strict, Some((640, 480)));
        assert_eq!(
            super::plan_check_sizes(vec![(300, 200)], (None, None)).strict,
            None
        );
    }

    /// Gauntlet 02: a button label wrapped as "Star" / "t".
    #[test]
    fn button_label_broken_mid_word_is_reported() {
        assert!(crate::host::wasm_render::rows_break_mid_word(&[
            ("Star".to_string(), false),
            ("t".to_string(), false)
        ]));
        assert!(!crate::host::wasm_render::rows_break_mid_word(&[
            ("Save ".to_string(), false),
            ("draft".to_string(), false)
        ]));
        assert!(!crate::host::wasm_render::rows_break_mid_word(&[
            ("one".to_string(), true),
            ("two".to_string(), false)
        ]));

        let mut start = painted(3, rect(24.0, 40.0, 40.0, 44.0));
        start.text_wrapped_mid_word = true;
        let layout = vec![
            painted(0, rect(0.0, 0.0, 320.0, 240.0)),
            painted(1, rect(24.0, 12.0, 60.0, 18.0)),
            painted(2, rect(24.0, 40.0, 140.0, 44.0)),
            start,
            painted(4, rect(72.0, 40.0, 90.0, 44.0)),
        ];
        let text = messages(&super::layout_findings(&tree(ACTIONS_TREE), &layout, true));
        assert!(
            text.contains("[warning] Button \"Start\": the label breaks in the middle of a word"),
            "{text}"
        );
    }

    /// Gauntlet 05/07/09: a Banner's text cut at the pane edge.
    #[test]
    fn text_cut_at_the_pane_edge_is_reported_and_scroll_content_is_not() {
        let banner = r#"{"root":0,"nodes":[
            {"id":0,"key":"0","data":{"type":"column","children":[1,2]}},
            {"id":1,"key":"1","data":{"type":"text","text":"Sync failed: check the relay address"}},
            {"id":2,"key":"2","data":{"type":"scroll","child":3}},
            {"id":3,"key":"3","data":{"type":"text","text":"A long log line that scrolls"}}
        ]}"#;
        let layout = vec![
            painted(0, rect(0.0, 0.0, 320.0, 240.0)),
            painted(1, rect(24.0, 12.0, 340.0, 18.0)),
            painted(2, rect(24.0, 40.0, 296.0, 100.0)),
            painted(3, rect(24.0, 40.0, 500.0, 18.0)),
        ];
        let findings = super::layout_findings(&tree(banner), &layout, true);
        let text = messages(&findings);
        assert!(
            text.contains(
                "[warning] Text \"Sync failed: check the relay address\" is cut off (44px on the right, 296x18px still showing)"
            ),
            "{text}"
        );
        assert_eq!(findings.len(), 1, "scrollable text is reachable: {text}");
    }

    /// The renderer's own measurement, on a layout no fix changes: content
    /// taller than the pane leaves the last button unpainted, and a tree that
    /// fits produces no finding.
    #[test]
    fn rendered_tree_reports_a_button_below_the_pane_and_passes_one_that_fits() {
        let tall = tree(
            r#"{"root":0,"nodes":[
                {"id":0,"key":"0","data":{"type":"column","children":[1,2,3]}},
                {"id":1,"key":"1","data":{"type":"text","text":"Nothing here yet"}},
                {"id":2,"key":"2","data":{"type":"spacer","size":400}},
                {"id":3,"key":"3","data":{"type":"button","label":"Add item","on_click":"add"}}
            ]}"#,
        );
        let layout = crate::host::wasm_render::render_ui_tree_layout(&tall, 320.0, 240.0);
        let findings = super::layout_findings(&tall, &layout, true);
        let text = messages(&findings);
        assert!(
            text.contains("[error] Button \"Add item\" is not visible"),
            "{text}"
        );
        assert!(!text.contains("Nothing here yet"), "{text}");

        let fits = tree(
            r#"{"root":0,"nodes":[
                {"id":0,"key":"0","data":{"type":"column","children":[1,2]}},
                {"id":1,"key":"1","data":{"type":"text","text":"Nothing here yet"}},
                {"id":2,"key":"2","data":{"type":"button","label":"Add item","on_click":"add"}}
            ]}"#,
        );
        let layout = crate::host::wasm_render::render_ui_tree_layout(&fits, 320.0, 240.0);
        let findings = super::layout_findings(&fits, &layout, true);
        assert!(findings.is_empty(), "{}", messages(&findings));

        let frame = super::frame_json_with_layout(&fits, &layout);
        let button = frame["layout"]
            .as_array()
            .and_then(|facts| facts.iter().find(|fact| fact["id"] == 2))
            .expect("the button has a layout entry");
        assert!(button["rect"][2].as_f64().unwrap_or(0.0) > 0.0, "{frame}");
    }

    fn rendered(
        json: &str,
        width: f32,
        height: f32,
    ) -> (crate::host::wasm_view::UiTree, Vec<NodeLayout>) {
        let tree = tree(json);
        let layout = crate::host::wasm_render::render_ui_tree_layout(&tree, width, height);
        (tree, layout)
    }

    /// The renderer's own text measurement: a word wider than the pane has to
    /// break inside itself, and a sentence that wraps at its spaces does not.
    #[test]
    fn rendered_text_reports_a_mid_word_break_and_not_a_break_at_a_space() {
        let fact = |layout: &[NodeLayout]| {
            layout
                .iter()
                .find(|fact| fact.id == 1)
                .cloned()
                .expect("the text node is measured")
        };
        let (_, layout) = rendered(
            r#"{"root":0,"nodes":[
                {"id":0,"key":"0","data":{"type":"column","children":[1]}},
                {"id":1,"key":"1","data":{"type":"text","text":"Pneumonoultramicroscopicsilicovolcanoconiosislongword"}}
            ]}"#,
            200.0,
            240.0,
        );
        assert!(fact(&layout).text_wrapped_mid_word, "{layout:?}");

        let (_, layout) = rendered(
            r#"{"root":0,"nodes":[
                {"id":0,"key":"0","data":{"type":"column","children":[1]}},
                {"id":1,"key":"1","data":{"type":"text","text":"one two three four five six seven eight nine ten eleven twelve"}}
            ]}"#,
            200.0,
            240.0,
        );
        let wrapped = fact(&layout);
        assert!(
            wrapped.rect.is_some_and(|rect| rect.height() > 30.0),
            "the sentence must wrap onto several rows for this to prove anything: {wrapped:?}"
        );
        assert!(!wrapped.text_wrapped_mid_word, "{wrapped:?}");
    }

    /// A correct layout through the real pass: a Text above a Row of two
    /// buttons shares no space and produces no finding at all.
    #[test]
    fn rendered_row_of_buttons_under_text_has_no_overlap_finding() {
        let (tree, layout) = rendered(ACTIONS_TREE, 480.0, 320.0);
        for id in [1, 3, 4] {
            assert!(
                layout
                    .iter()
                    .any(|fact| fact.id == id && fact.visible.is_some()),
                "node {id} must be measured and visible: {layout:?}"
            );
        }
        let findings = super::layout_findings(&tree, &layout, true);
        assert!(findings.is_empty(), "{}", messages(&findings));
    }

    /// Stint: buttons never break mid-word; rows, Select and Banner wrap at
    /// narrow widths.
    const STINT_NARROW_ROWS: &str = "1333";
    /// Stint: bundled canvas plugins paint over their SurfaceHeader.
    const STINT_CANVAS_OVER_HEADER: &str = "1353";

    /// A layout finding in a bundled plugin that is known and has a stint to
    /// fix it. `finding` is the start of the finding's message.
    struct KnownLayoutFinding {
        plugin: &'static str,
        size: (u32, u32),
        finding: &'static str,
        fix_stint: &'static str,
        /// False for a finding that depends on random plugin state (where the
        /// balls spawn): it is allowed, and not required to reproduce.
        every_run: bool,
    }

    const fn known(
        plugin: &'static str,
        size: (u32, u32),
        finding: &'static str,
        fix_stint: &'static str,
    ) -> KnownLayoutFinding {
        KnownLayoutFinding {
            plugin,
            size,
            finding,
            fix_stint,
            every_run: true,
        }
    }

    const fn sometimes(
        plugin: &'static str,
        size: (u32, u32),
        finding: &'static str,
        fix_stint: &'static str,
    ) -> KnownLayoutFinding {
        KnownLayoutFinding {
            plugin,
            size,
            finding,
            fix_stint,
            every_run: false,
        }
    }

    const KNOWN_CORE_LAYOUT_FINDINGS: &[KnownLayoutFinding] = &[
        known(
            "logs",
            (320, 240),
            "Button \"DEBUG\": the label breaks in the middle of a word",
            STINT_NARROW_ROWS,
        ),
        known(
            "logs",
            (320, 240),
            "Button \"DEBUG\" is cut off",
            STINT_NARROW_ROWS,
        ),
        known(
            "github-issues",
            (320, 240),
            "TextInput \"owner/repo (e.g. plexiapp/plexi)\" is cut off",
            STINT_NARROW_ROWS,
        ),
        known(
            "github-issues",
            (320, 240),
            "Text \"#\" is not visible",
            STINT_NARROW_ROWS,
        ),
        known(
            "github-issues",
            (320, 240),
            "Text \"No repository loaded\" is not visible",
            STINT_NARROW_ROWS,
        ),
        known(
            "github-issues",
            (320, 240),
            "Text \"Type owner/repo above",
            STINT_NARROW_ROWS,
        ),
        known(
            "sudoku",
            (320, 240),
            "SurfaceHeader \"Sudoku\" and Canvas",
            STINT_CANVAS_OVER_HEADER,
        ),
        sometimes(
            "balls",
            (320, 240),
            "SurfaceHeader \"Balls\" and Canvas",
            STINT_CANVAS_OVER_HEADER,
        ),
        sometimes(
            "balls",
            (800, 600),
            "SurfaceHeader \"Balls\" and Canvas",
            STINT_CANVAS_OVER_HEADER,
        ),
    ];

    /// The maintained set (`packs/core.toml`) is clean under the layout pass
    /// except for `KNOWN_CORE_LAYOUT_FINDINGS`, at the smallest size each
    /// plugin supports and at a normal one. A new finding fails, and so does
    /// a known one that stopped reproducing, so the list cannot go stale:
    /// the change that fixes a plugin removes its entries.
    #[test]
    fn core_pack_plugins_have_only_known_layout_findings() {
        let pack = crate::app::packs::Pack::from_toml_str(crate::cli::install_host::CORE_PACK_TOML)
            .expect("core pack parses");
        let apps = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("apps");
        let mut errors = Vec::new();
        let mut reproduced = vec![false; KNOWN_CORE_LAYOUT_FINDINGS.len()];
        let mut checked = 0;
        for plugin in &pack.plugins {
            let Some(dir) = plugin.source.strip_prefix("local:") else {
                continue;
            };
            let dir = apps.join(dir);
            let (manifest, entry) = super::load_local_plugin(&dir).expect("core plugin loads");
            if entry.extension().and_then(|ext| ext.to_str()) != Some("py") {
                continue;
            }
            let config = super::python_launch_config(&manifest, &entry, &dir).unwrap();
            let plan = super::plan_check_sizes(
                vec![super::SMALLEST_CHECK_SIZE, (480, 320), (800, 600)],
                (manifest.launch.min_width, manifest.launch.min_height),
            );
            // The strict size and one normal size; the rest add boots, not coverage.
            let sizes: Vec<(u32, u32)> = plan.strict.into_iter().chain([(800, 600)]).collect();
            for (width, height) in sizes {
                let tree = crate::host::wasm_python::run_headless_frame(
                    &config,
                    (width as f32, height as f32),
                    None,
                )
                .unwrap_or_else(|e| panic!("{} must render headlessly: {e}", manifest.plugin.id));
                let layout = crate::host::wasm_render::render_ui_tree_layout(
                    &tree,
                    width as f32,
                    height as f32,
                );
                assert!(
                    !layout.is_empty(),
                    "{} {width}x{height}: the layout pass measured no node",
                    manifest.plugin.id
                );
                let strict = plan.strict == Some((width, height));
                for finding in super::layout_findings(&tree, &layout, strict) {
                    let line = format!(
                        "{} {width}x{height}: {}",
                        manifest.plugin.id, finding.message
                    );
                    eprintln!(
                        "core-pack layout {}: {line}",
                        if finding.is_error { "error" } else { "warning" }
                    );
                    match KNOWN_CORE_LAYOUT_FINDINGS.iter().position(|known| {
                        known.plugin == manifest.plugin.id
                            && known.size == (width, height)
                            && finding.message.starts_with(known.finding)
                    }) {
                        Some(index) => reproduced[index] = true,
                        None => errors.push(line),
                    }
                }
                checked += 1;
            }
        }
        assert!(checked > 0, "no core plugin was checked");
        assert!(
            errors.is_empty(),
            "new core pack layout findings (fix the plugin, or add each to KNOWN_CORE_LAYOUT_FINDINGS with its fix stint):\n{}",
            errors.join("\n")
        );
        let stale: Vec<String> = KNOWN_CORE_LAYOUT_FINDINGS
            .iter()
            .zip(&reproduced)
            .filter(|(known, reproduced)| known.every_run && !**reproduced)
            .map(|(known, _)| {
                format!(
                    "{} {}x{}: {} (stint {})",
                    known.plugin, known.size.0, known.size.1, known.finding, known.fix_stint
                )
            })
            .collect();
        assert!(
            stale.is_empty(),
            "known core pack layout findings no longer reproduce; remove them from KNOWN_CORE_LAYOUT_FINDINGS:\n{}",
            stale.join("\n")
        );
    }
}

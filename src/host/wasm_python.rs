//! CPython-in-WASM adapter boundary for SDK v3 Python apps.
//!
//! This module owns the deterministic host side of the Python compatibility
//! path: manifest routing, CPython bundle resolution, and JSON bridge
//! marshalling. It intentionally does not fall back to the native PGAP
//! subprocess path when the CPython WASM bundle is unavailable.

use std::path::{Path, PathBuf};

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::app::registry::{AppManifest, RuntimeExecution};

use super::wasm_app::bindings::plexi::platform::types::{
    BadgeColor, ButtonNode, ButtonStyle, ColumnNode, FileReadEffect, FileWriteEffect, IndexedNode,
    InputEvent, KeyEvent, StateSnapshot, TextNode, TimerEffect, UiActionEvent, UiNodeData, UiTree,
    UiValueChangeEvent,
};
use super::wasm_app::{Alignment, Effect};

pub const CPYTHON_BUNDLE_VERSION: &str = "3.12.3";
pub const CPYTHON_BUNDLE_FILE: &str = "cpython-3.12.wasm";
pub const CPYTHON_BUNDLE_SHA256: &str = "unavailable-until-bundle-is-vendored";
pub const FETCH_CPYTHON_BUNDLE_COMMAND: &str = "just fetch-cpython-bundle";

#[derive(Debug, Error)]
pub enum WasmPythonError {
    #[error("read manifest at {path}: {source}")]
    ReadManifest {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("parse manifest at {path}: {source}")]
    ParseManifest {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("runtime.python_compat requires a .py or .pyc entry, got '{entry}'")]
    InvalidEntry { entry: String },
    #[error("runtime.python_compat entry is missing: {path}")]
    MissingEntry { path: PathBuf },
    #[error("runtime.python_compat execution='{execution}' is not implemented yet")]
    UnsupportedExecution { execution: &'static str },
    #[error("CPython WASM bundle unavailable at {path}; run: {command}")]
    MissingBundle {
        path: PathBuf,
        command: &'static str,
    },
    #[error("CPython WASM bundle hash is not pinned for {version}; run: {command}")]
    BundleHashUnpinned {
        version: &'static str,
        command: &'static str,
    },
    #[error("CPython WASM bundle hash mismatch at {path}: expected {expected}, got {actual}")]
    BundleHashMismatch {
        path: PathBuf,
        expected: &'static str,
        actual: String,
    },
    #[error("read CPython WASM bundle at {path}: {source}")]
    ReadBundle {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("bridge JSON error: {0}")]
    BridgeJson(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PythonLaunchConfig {
    pub app_id: String,
    pub app_dir: PathBuf,
    pub entry: PathBuf,
    pub module_name: String,
}

impl PythonLaunchConfig {
    pub fn from_manifest_file(app_dir: &Path) -> Result<Option<Self>, WasmPythonError> {
        let manifest_path = app_dir.join("manifest.toml");
        let raw = std::fs::read_to_string(&manifest_path).map_err(|source| {
            WasmPythonError::ReadManifest {
                path: manifest_path.clone(),
                source,
            }
        })?;
        let manifest: AppManifest =
            toml::from_str(&raw).map_err(|source| WasmPythonError::ParseManifest {
                path: manifest_path,
                source,
            })?;

        if manifest.runtime.python_compat != Some(true) {
            return Ok(None);
        }
        if manifest.runtime.execution != RuntimeExecution::Local {
            return Err(WasmPythonError::UnsupportedExecution {
                execution: runtime_execution_label(manifest.runtime.execution),
            });
        }

        let entry = manifest.app.entry;
        if !(entry.ends_with(".py") || entry.ends_with(".pyc")) {
            return Err(WasmPythonError::InvalidEntry { entry });
        }
        let entry_path = app_dir.join(&entry);
        if !entry_path.is_file() {
            return Err(WasmPythonError::MissingEntry { path: entry_path });
        }
        let module_name = entry_path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("main")
            .to_string();

        Ok(Some(Self {
            app_id: manifest.app.id,
            app_dir: app_dir.to_path_buf(),
            entry: entry_path,
            module_name,
        }))
    }
}

pub struct WasmPythonAdapter {
    pub config: PythonLaunchConfig,
    pub cpython_bundle: PathBuf,
}

impl WasmPythonAdapter {
    pub fn prepare_from_manifest(app_dir: &Path) -> Result<Option<Self>, WasmPythonError> {
        let Some(config) = PythonLaunchConfig::from_manifest_file(app_dir)? else {
            return Ok(None);
        };
        let bundle = resolve_default_cpython_bundle()?;
        log::info!(
            "app::{}: python_compat routed to CPython WASM bundle {}",
            config.app_id,
            bundle.display()
        );
        Ok(Some(Self {
            config,
            cpython_bundle: bundle,
        }))
    }

    pub fn bridge_contract_probe(
        &self,
        size: (f32, f32),
        args: &[String],
    ) -> Result<(Value, Value, Value), WasmPythonError> {
        let snapshot = StateSnapshot {
            entries: Vec::new(),
        };
        log::info!(
            "app::{}: python_compat bridge prepared module={} bundle={}",
            self.config.app_id,
            self.config.module_name,
            self.cpython_bundle.display()
        );
        for effect in decode_effects(
            r#"[{"type":"SetTitle","title":"probe"},{"type":"SetState","data":{"probe":true}}]"#,
        )? {
            match effect {
                PythonBridgeEffect::Host(host_effect) => {
                    if !matches!(host_effect, Effect::SetTitle(_)) {
                        return Err(WasmPythonError::BridgeJson(
                            "bridge probe expected SetTitle host effect".to_string(),
                        ));
                    }
                }
                PythonBridgeEffect::SetState(entries) => {
                    if entries.len() != 1 {
                        return Err(WasmPythonError::BridgeJson(
                            "bridge probe expected one SetState entry".to_string(),
                        ));
                    }
                }
            }
        }
        let _ =
            decode_ui_tree(r#"{"root":0,"nodes":[{"id":0,"key":"0","data":{"type":"Empty"}}]}"#)?;
        Ok((
            init_bridge_arg(&snapshot, size, args),
            update_bridge_arg(&snapshot, &InputEvent::FocusGained)?,
            view_bridge_arg(&snapshot),
        ))
    }
}

pub fn resolve_default_cpython_bundle() -> Result<PathBuf, WasmPythonError> {
    resolve_cpython_bundle(crate::config::config_dir().join("wasm-bundles"))
}

pub fn resolve_cpython_bundle(cache_dir: PathBuf) -> Result<PathBuf, WasmPythonError> {
    let path = cache_dir.join(CPYTHON_BUNDLE_FILE);
    if !path.is_file() {
        return Err(WasmPythonError::MissingBundle {
            path,
            command: FETCH_CPYTHON_BUNDLE_COMMAND,
        });
    }
    if CPYTHON_BUNDLE_SHA256.len() != 64 {
        return Err(WasmPythonError::BundleHashUnpinned {
            version: CPYTHON_BUNDLE_VERSION,
            command: FETCH_CPYTHON_BUNDLE_COMMAND,
        });
    }
    let bytes = std::fs::read(&path).map_err(|source| WasmPythonError::ReadBundle {
        path: path.clone(),
        source,
    })?;
    let actual = format!("{:x}", Sha256::digest(&bytes));
    if actual != CPYTHON_BUNDLE_SHA256 {
        return Err(WasmPythonError::BundleHashMismatch {
            path,
            expected: CPYTHON_BUNDLE_SHA256,
            actual,
        });
    }
    Ok(path)
}

#[derive(Debug, Clone)]
pub enum PythonBridgeEffect {
    Host(Effect),
    SetState(Vec<(String, Vec<u8>)>),
}

pub fn init_bridge_arg(snapshot: &StateSnapshot, size: (f32, f32), args: &[String]) -> Value {
    json!({
        "state": encode_state(snapshot),
        "size": [size.0, size.1],
        "args": args,
    })
}

pub fn update_bridge_arg(
    snapshot: &StateSnapshot,
    event: &InputEvent,
) -> Result<Value, WasmPythonError> {
    Ok(json!({
        "state": encode_state(snapshot),
        "event": encode_input_event(event)?,
    }))
}

pub fn view_bridge_arg(snapshot: &StateSnapshot) -> Value {
    json!({ "state": encode_state(snapshot) })
}

pub fn encode_state(snapshot: &StateSnapshot) -> Value {
    let mut out = serde_json::Map::new();
    for (key, bytes) in &snapshot.entries {
        let encoded = if serde_json::from_slice::<Value>(bytes).is_ok() {
            BASE64.encode(bytes)
        } else {
            format!("b64:{}", BASE64.encode(bytes))
        };
        out.insert(key.clone(), Value::String(encoded));
    }
    Value::Object(out)
}

pub fn encode_input_event(event: &InputEvent) -> Result<Value, WasmPythonError> {
    let value = match event {
        InputEvent::Key(KeyEvent {
            key,
            modifiers,
            pressed,
        }) => json!({
            "type": "KeyEvent",
            "key": key,
            "modifiers": {
                "ctrl": modifiers.ctrl,
                "shift": modifiers.shift,
                "alt": modifiers.alt,
                "meta": modifiers.meta,
            },
            "pressed": pressed,
        }),
        InputEvent::UiAction(UiActionEvent { handler_id }) => {
            json!({ "type": "UiAction", "handler_id": handler_id })
        }
        InputEvent::UiValueChange(UiValueChangeEvent { handler_id, value }) => {
            json!({ "type": "UiValueChange", "handler_id": handler_id, "value": value })
        }
        InputEvent::Resize(size) => {
            json!({ "type": "Resize", "width": size.width, "height": size.height })
        }
        InputEvent::FocusGained => json!({ "type": "FocusGained" }),
        InputEvent::FocusLost => json!({ "type": "FocusLost" }),
        InputEvent::TimerFired(id) => json!({ "type": "TimerFired", "id": id }),
        InputEvent::CapabilityGranted(name) => {
            json!({ "type": "CapabilityGranted", "name": name })
        }
        InputEvent::CapabilityDenied(name) => json!({ "type": "CapabilityDenied", "name": name }),
        other => {
            return Err(WasmPythonError::BridgeJson(format!(
                "input event not yet supported by Python bridge: {other:?}"
            )));
        }
    };
    Ok(value)
}

pub fn decode_effects(json_text: &str) -> Result<Vec<PythonBridgeEffect>, WasmPythonError> {
    let values = serde_json::from_str::<Vec<Value>>(json_text)
        .map_err(|e| WasmPythonError::BridgeJson(e.to_string()))?;
    values.into_iter().map(decode_effect).collect()
}

fn decode_effect(value: Value) -> Result<PythonBridgeEffect, WasmPythonError> {
    let kind = value
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| WasmPythonError::BridgeJson("effect missing string 'type'".to_string()))?;
    match kind {
        "SetState" => decode_set_state(value).map(PythonBridgeEffect::SetState),
        "SetTitle" => Ok(PythonBridgeEffect::Host(Effect::SetTitle(required_string(
            &value, "title",
        )?))),
        "SetStatus" => Ok(PythonBridgeEffect::Host(Effect::SetStatus(
            required_string(&value, "text")?,
        ))),
        "CloseSelf" => Ok(PythonBridgeEffect::Host(Effect::CloseSelf)),
        "RequestCapability" => Ok(PythonBridgeEffect::Host(Effect::RequestCapability(
            required_string(&value, "name")?,
        ))),
        "SetTimer" => Ok(PythonBridgeEffect::Host(Effect::SetTimer(TimerEffect {
            id: required_u32(&value, "id")?,
            delay_ms: required_u32(&value, "delay_ms")?,
            repeat: value
                .get("repeat")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        }))),
        "CancelTimer" => Ok(PythonBridgeEffect::Host(Effect::CancelTimer(required_u32(
            &value, "id",
        )?))),
        "GetSystemStats" => Ok(PythonBridgeEffect::Host(Effect::GetSystemStats)),
        "FileRead" => Ok(PythonBridgeEffect::Host(Effect::FileRead(FileReadEffect {
            path: required_string(&value, "path")?,
        }))),
        "FileWrite" => Ok(PythonBridgeEffect::Host(Effect::FileWrite(
            FileWriteEffect {
                path: required_string(&value, "path")?,
                content: bytes_field(&value, "content")?,
            },
        ))),
        other => Err(WasmPythonError::BridgeJson(format!(
            "Unknown effect type: {other}"
        ))),
    }
}

fn decode_set_state(value: Value) -> Result<Vec<(String, Vec<u8>)>, WasmPythonError> {
    let data = value
        .get("data")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            WasmPythonError::BridgeJson("SetState.data must be an object".to_string())
        })?;
    data.iter()
        .map(|(key, value)| {
            serde_json::to_vec(value)
                .map(|bytes| (key.clone(), bytes))
                .map_err(|e| WasmPythonError::BridgeJson(e.to_string()))
        })
        .collect()
}

pub fn decode_ui_tree(json_text: &str) -> Result<UiTree, WasmPythonError> {
    let value = serde_json::from_str::<Value>(json_text)
        .map_err(|e| WasmPythonError::BridgeJson(e.to_string()))?;
    let root = required_u32(&value, "root")?;
    let nodes = value
        .get("nodes")
        .and_then(Value::as_array)
        .ok_or_else(|| WasmPythonError::BridgeJson("ui tree missing nodes array".to_string()))?
        .iter()
        .map(decode_indexed_node)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(UiTree { root, nodes })
}

fn decode_indexed_node(value: &Value) -> Result<IndexedNode, WasmPythonError> {
    let data = value
        .get("data")
        .ok_or_else(|| WasmPythonError::BridgeJson("indexed node missing data".to_string()))?;
    Ok(IndexedNode {
        id: required_u32(value, "id")?,
        key: required_string(value, "key")?,
        data: decode_node_data(data)?,
    })
}

fn decode_node_data(value: &Value) -> Result<UiNodeData, WasmPythonError> {
    let kind = value
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| WasmPythonError::BridgeJson("ui node missing string 'type'".to_string()))?;
    match kind {
        "Empty" => Ok(UiNodeData::Empty),
        "Text" | "label" => Ok(UiNodeData::Text(TextNode {
            text: required_string(value, "text")?,
            size: optional_f32(value, "size")?,
            bold: value.get("bold").and_then(Value::as_bool).unwrap_or(false),
            color: None,
            truncate: value
                .get("truncate")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            align: decode_alignment(
                value
                    .get("align")
                    .and_then(Value::as_str)
                    .unwrap_or("start"),
            )?,
        })),
        "Column" | "column" => Ok(UiNodeData::Column(ColumnNode {
            children: u32_list(value, "children")?,
            gap: optional_f32(value, "gap")?.unwrap_or(0.0),
            align: decode_alignment(
                value
                    .get("align")
                    .and_then(Value::as_str)
                    .unwrap_or("start"),
            )?,
            grow: value.get("grow").and_then(Value::as_bool).unwrap_or(false),
        })),
        "Button" | "button" => Ok(UiNodeData::Button(ButtonNode {
            label: required_string(value, "label")?,
            on_click: required_string(value, "on_click")?,
            style: decode_button_style(
                value
                    .get("style")
                    .and_then(Value::as_str)
                    .unwrap_or("secondary"),
            )?,
            disabled: value
                .get("disabled")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        })),
        "Divider" | "divider" => Ok(UiNodeData::Divider),
        "Space" | "spacer" => Ok(UiNodeData::Space(
            optional_f32(value, "size")?.unwrap_or(0.0),
        )),
        "Badge" | "badge" => Ok(UiNodeData::Badge(
            super::wasm_app::bindings::plexi::platform::types::BadgeNode {
                text: required_string(value, "text")?,
                color: decode_badge_color(
                    value
                        .get("color")
                        .and_then(Value::as_str)
                        .unwrap_or("neutral"),
                )?,
            },
        )),
        other => Err(WasmPythonError::BridgeJson(format!(
            "Unknown UINode type: {other}"
        ))),
    }
}

fn required_string(value: &Value, field: &str) -> Result<String, WasmPythonError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| WasmPythonError::BridgeJson(format!("missing string field '{field}'")))
}

fn required_u32(value: &Value, field: &str) -> Result<u32, WasmPythonError> {
    let n = value
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| WasmPythonError::BridgeJson(format!("missing u32 field '{field}'")))?;
    u32::try_from(n)
        .map_err(|_| WasmPythonError::BridgeJson(format!("field '{field}' out of u32 range")))
}

fn optional_f32(value: &Value, field: &str) -> Result<Option<f32>, WasmPythonError> {
    value
        .get(field)
        .map(|v| {
            v.as_f64().map(|n| n as f32).ok_or_else(|| {
                WasmPythonError::BridgeJson(format!("field '{field}' must be a number"))
            })
        })
        .transpose()
}

fn bytes_field(value: &Value, field: &str) -> Result<Vec<u8>, WasmPythonError> {
    match value.get(field) {
        Some(Value::String(s)) => Ok(s.as_bytes().to_vec()),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| {
                let n = item.as_u64().ok_or_else(|| {
                    WasmPythonError::BridgeJson(format!(
                        "field '{field}' byte array contains non-u64"
                    ))
                })?;
                u8::try_from(n).map_err(|_| {
                    WasmPythonError::BridgeJson(format!("field '{field}' byte out of range"))
                })
            })
            .collect(),
        _ => Err(WasmPythonError::BridgeJson(format!(
            "missing bytes field '{field}'"
        ))),
    }
}

fn u32_list(value: &Value, field: &str) -> Result<Vec<u32>, WasmPythonError> {
    value
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| WasmPythonError::BridgeJson(format!("missing array field '{field}'")))?
        .iter()
        .map(|item| {
            let n = item.as_u64().ok_or_else(|| {
                WasmPythonError::BridgeJson(format!("field '{field}' contains non-u64"))
            })?;
            u32::try_from(n)
                .map_err(|_| WasmPythonError::BridgeJson(format!("field '{field}' out of range")))
        })
        .collect()
}

fn decode_alignment(value: &str) -> Result<Alignment, WasmPythonError> {
    match value {
        "start" => Ok(Alignment::Start),
        "center" => Ok(Alignment::Center),
        "end" => Ok(Alignment::End),
        "stretch" => Ok(Alignment::Stretch),
        other => Err(WasmPythonError::BridgeJson(format!(
            "unknown alignment: {other}"
        ))),
    }
}

fn decode_button_style(value: &str) -> Result<ButtonStyle, WasmPythonError> {
    match value {
        "primary" => Ok(ButtonStyle::Primary),
        "secondary" => Ok(ButtonStyle::Secondary),
        "danger" => Ok(ButtonStyle::Danger),
        "ghost" => Ok(ButtonStyle::Ghost),
        other => Err(WasmPythonError::BridgeJson(format!(
            "unknown button style: {other}"
        ))),
    }
}

fn decode_badge_color(value: &str) -> Result<BadgeColor, WasmPythonError> {
    match value {
        "accent" => Ok(BadgeColor::Accent),
        "success" => Ok(BadgeColor::Success),
        "warning" => Ok(BadgeColor::Warning),
        "danger" => Ok(BadgeColor::Danger),
        "neutral" => Ok(BadgeColor::Neutral),
        other => Err(WasmPythonError::BridgeJson(format!(
            "unknown badge color: {other}"
        ))),
    }
}

fn runtime_execution_label(execution: RuntimeExecution) -> &'static str {
    match execution {
        RuntimeExecution::Local => "local",
        RuntimeExecution::Cloud => "cloud",
        RuntimeExecution::PreferredLocal => "preferred-local",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::wasm_app::Modifiers;
    use tempfile::tempdir;

    #[test]
    fn manifest_python_compat_routes_to_launch_config() {
        let dir = tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("manifest.toml"),
            r#"
schema_version = 1

[app]
id = "hello-py"
type = "app"
name = "Hello Python"
entry = "main.py"
version = "0.1.0"

[runtime]
python_compat = true
"#,
        )
        .expect("manifest");
        std::fs::write(dir.path().join("main.py"), "def view(): pass\n").expect("entry");

        let config = PythonLaunchConfig::from_manifest_file(dir.path())
            .expect("route")
            .expect("python compat config");

        assert_eq!(config.module_name, "main");
        assert_eq!(config.app_id, "hello-py");
    }

    #[test]
    fn missing_bundle_returns_actionable_error() {
        let dir = tempdir().expect("tempdir");
        let err = resolve_cpython_bundle(dir.path().join("wasm-bundles")).unwrap_err();

        assert!(err.to_string().contains(FETCH_CPYTHON_BUNDLE_COMMAND));
    }

    #[test]
    fn state_snapshot_encodes_json_bytes_and_raw_bytes() {
        let snapshot = StateSnapshot {
            entries: vec![
                ("count".to_string(), b"3".to_vec()),
                ("raw".to_string(), vec![0, 159, 146, 150]),
            ],
        };

        let encoded = encode_state(&snapshot);

        assert_eq!(encoded["count"], BASE64.encode(b"3"));
        assert_eq!(
            encoded["raw"],
            format!("b64:{}", BASE64.encode([0, 159, 146, 150]))
        );
    }

    #[test]
    fn key_event_encodes_sdk_v3_shape() {
        let event = InputEvent::Key(KeyEvent {
            key: "q".to_string(),
            modifiers: Modifiers {
                ctrl: false,
                shift: true,
                alt: false,
                meta: true,
            },
            pressed: true,
        });

        let encoded = encode_input_event(&event).expect("encode event");

        assert_eq!(encoded["type"], "KeyEvent");
        assert_eq!(encoded["modifiers"]["shift"], true);
        assert_eq!(encoded["modifiers"]["meta"], true);
    }

    #[test]
    fn effects_decode_host_effects_and_set_state_boundary() {
        let effects = decode_effects(
            r#"[
                {"type":"SetTitle","title":"hello"},
                {"type":"SetState","data":{"count":4}}
            ]"#,
        )
        .expect("effects");

        assert!(matches!(
            &effects[0],
            PythonBridgeEffect::Host(Effect::SetTitle(title)) if title == "hello"
        ));
        assert!(matches!(
            &effects[1],
            PythonBridgeEffect::SetState(entries) if entries == &vec![("count".to_string(), b"4".to_vec())]
        ));
    }

    #[test]
    fn ui_tree_decodes_text_node() {
        let tree = decode_ui_tree(
            r#"{
                "root":0,
                "nodes":[
                    {"id":0,"key":"0","data":{"type":"Text","text":"ok","bold":true,"align":"center"}}
                ]
            }"#,
        )
        .expect("tree");

        match &tree.nodes[0].data {
            UiNodeData::Text(text) => assert_eq!(text.text, "ok"),
            other => panic!("expected text node, got {other:?}"),
        }
    }
}

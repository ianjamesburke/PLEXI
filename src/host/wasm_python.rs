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
    BadgeColor, ButtonNode, ButtonStyle, CanvasCircle, CanvasCommand, CanvasLine, CanvasNode,
    CanvasRect, CanvasText, Color, ColumnNode, FileReadEffect, FileWriteEffect, HttpFetchEffect,
    IndexedNode, InputEvent, KeyEvent, ListNode, PaddingNode, ProgressBarNode, RowNode, ScrollNode,
    StateSnapshot, TextInputNode, TextNode, TimerEffect, UiActionEvent, UiNodeData, UiTree,
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
        InputEvent::HttpResponse(response) => json!({
            "type": "HttpResponse",
            "status": response.status,
            "headers": response.headers,
            "body": response.body,
        }),
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
        "HttpFetch" => Ok(PythonBridgeEffect::Host(Effect::HttpFetch(
            HttpFetchEffect {
                url: required_string(&value, "url")?,
                method: value
                    .get("method")
                    .and_then(Value::as_str)
                    .unwrap_or("GET")
                    .to_string(),
                headers: headers_field(&value, "headers")?,
                body: optional_bytes_field(&value, "body")?,
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
        "TextInput" | "text_input" | "text-input" => Ok(UiNodeData::TextInput(TextInputNode {
            value: value
                .get("value")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            placeholder: value
                .get("placeholder")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            on_change: value
                .get("on_change")
                .or_else(|| value.get("on-change"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            on_submit: value
                .get("on_submit")
                .or_else(|| value.get("on-submit"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            password: value
                .get("password")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        })),
        "Row" | "row" => Ok(UiNodeData::Row(RowNode {
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
        "Divider" | "divider" => Ok(UiNodeData::Divider),
        "Space" | "spacer" => Ok(UiNodeData::Space(
            optional_f32(value, "size")?.unwrap_or(0.0),
        )),
        "ProgressBar" | "progress_bar" | "progress-bar" => {
            Ok(UiNodeData::ProgressBar(ProgressBarNode {
                value: optional_f32(value, "value")?.unwrap_or(0.0),
                max: optional_f32(value, "max")?.unwrap_or(1.0),
                color: None,
                label: value
                    .get("label")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            }))
        }
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
        "ListView" | "list_view" | "list-view" => Ok(UiNodeData::ListView(ListNode {
            items: u32_list(value, "items")?,
            selected: value
                .get("selected")
                .and_then(Value::as_u64)
                .map(u32::try_from)
                .transpose()
                .map_err(|_| {
                    WasmPythonError::BridgeJson("field 'selected' out of range".to_string())
                })?,
            on_select: value
                .get("on_select")
                .or_else(|| value.get("on-select"))
                .and_then(Value::as_str)
                .map(str::to_string),
        })),
        "Scroll" | "scroll" => Ok(UiNodeData::Scroll(ScrollNode {
            child: required_u32(value, "child")?,
            horizontal: value
                .get("horizontal")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        })),
        "Padding" | "padding" => Ok(UiNodeData::Padding(PaddingNode {
            child: required_u32(value, "child")?,
            top: optional_f32(value, "top")?.unwrap_or(0.0),
            right: optional_f32(value, "right")?.unwrap_or(0.0),
            bottom: optional_f32(value, "bottom")?.unwrap_or(0.0),
            left: optional_f32(value, "left")?.unwrap_or(0.0),
        })),
        "Canvas" | "canvas" => Ok(UiNodeData::Canvas(CanvasNode {
            width: optional_f32(value, "width")?.unwrap_or(640.0),
            height: optional_f32(value, "height")?.unwrap_or(360.0),
            grow: value.get("grow").and_then(Value::as_bool).unwrap_or(true),
            commands: canvas_commands(value, "commands")?,
        })),
        other => Err(WasmPythonError::BridgeJson(format!(
            "Unknown UINode type: {other}"
        ))),
    }
}

fn canvas_commands(value: &Value, field: &str) -> Result<Vec<CanvasCommand>, WasmPythonError> {
    value
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| WasmPythonError::BridgeJson(format!("missing array field '{field}'")))?
        .iter()
        .map(decode_canvas_command)
        .collect()
}

fn decode_canvas_command(value: &Value) -> Result<CanvasCommand, WasmPythonError> {
    let kind = value.get("type").and_then(Value::as_str).ok_or_else(|| {
        WasmPythonError::BridgeJson("canvas command missing string 'type'".to_string())
    })?;
    match kind {
        "rect" | "Rect" => Ok(CanvasCommand::Rect(CanvasRect {
            x: optional_f32(value, "x")?.unwrap_or(0.0),
            y: optional_f32(value, "y")?.unwrap_or(0.0),
            width: optional_f32(value, "width")?
                .or(optional_f32(value, "w")?)
                .unwrap_or(0.0),
            height: optional_f32(value, "height")?
                .or(optional_f32(value, "h")?)
                .unwrap_or(0.0),
            fill: decode_color_field(value, "fill")?,
            radius: optional_f32(value, "radius")?.unwrap_or(0.0),
        })),
        "circle" | "Circle" => Ok(CanvasCommand::Circle(CanvasCircle {
            x: optional_f32(value, "x")?.unwrap_or(0.0),
            y: optional_f32(value, "y")?.unwrap_or(0.0),
            radius: optional_f32(value, "radius")?
                .or(optional_f32(value, "r")?)
                .unwrap_or(0.0),
            fill: decode_color_field(value, "fill")?,
        })),
        "line" | "Line" => Ok(CanvasCommand::Line(CanvasLine {
            x1: optional_f32(value, "x1")?.unwrap_or(0.0),
            y1: optional_f32(value, "y1")?.unwrap_or(0.0),
            x2: optional_f32(value, "x2")?.unwrap_or(0.0),
            y2: optional_f32(value, "y2")?.unwrap_or(0.0),
            width: optional_f32(value, "width")?.unwrap_or(1.0),
            color: decode_color_field(value, "color")?,
        })),
        "text" | "Text" => Ok(CanvasCommand::Text(CanvasText {
            x: optional_f32(value, "x")?.unwrap_or(0.0),
            y: optional_f32(value, "y")?.unwrap_or(0.0),
            text: required_string(value, "text")?,
            size: optional_f32(value, "size")?.unwrap_or(14.0),
            color: decode_color_field(value, "color")?,
            bold: value.get("bold").and_then(Value::as_bool).unwrap_or(false),
            align: decode_alignment(
                value
                    .get("align")
                    .and_then(Value::as_str)
                    .unwrap_or("start"),
            )?,
        })),
        other => Err(WasmPythonError::BridgeJson(format!(
            "unknown canvas command type: {other}"
        ))),
    }
}

fn decode_color_field(value: &Value, field: &str) -> Result<Color, WasmPythonError> {
    let Some(raw) = value.get(field) else {
        return Err(WasmPythonError::BridgeJson(format!(
            "missing color field '{field}'"
        )));
    };
    decode_color(raw)
}

fn decode_color(value: &Value) -> Result<Color, WasmPythonError> {
    if let Some(hex) = value.as_str() {
        return decode_hex_color(hex);
    }
    let r = required_u8(value, "r")?;
    let g = required_u8(value, "g")?;
    let b = required_u8(value, "b")?;
    let a = value
        .get("a")
        .map(|_| required_u8(value, "a"))
        .transpose()?
        .unwrap_or(255);
    Ok(Color { r, g, b, a })
}

fn decode_hex_color(hex: &str) -> Result<Color, WasmPythonError> {
    let value = hex.strip_prefix('#').unwrap_or(hex);
    let parse = |s: &str| {
        u8::from_str_radix(s, 16)
            .map_err(|e| WasmPythonError::BridgeJson(format!("invalid color '{hex}': {e}")))
    };
    match value.len() {
        6 => Ok(Color {
            r: parse(&value[0..2])?,
            g: parse(&value[2..4])?,
            b: parse(&value[4..6])?,
            a: 255,
        }),
        8 => Ok(Color {
            r: parse(&value[0..2])?,
            g: parse(&value[2..4])?,
            b: parse(&value[4..6])?,
            a: parse(&value[6..8])?,
        }),
        _ => Err(WasmPythonError::BridgeJson(format!(
            "invalid color '{hex}': expected #rrggbb or #rrggbbaa"
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

fn required_u8(value: &Value, field: &str) -> Result<u8, WasmPythonError> {
    let n = value
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| WasmPythonError::BridgeJson(format!("missing u8 field '{field}'")))?;
    u8::try_from(n)
        .map_err(|_| WasmPythonError::BridgeJson(format!("field '{field}' out of u8 range")))
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

fn optional_bytes_field(value: &Value, field: &str) -> Result<Option<Vec<u8>>, WasmPythonError> {
    if matches!(value.get(field), None | Some(Value::Null)) {
        return Ok(None);
    }
    bytes_field(value, field).map(Some)
}

fn headers_field(value: &Value, field: &str) -> Result<Vec<(String, String)>, WasmPythonError> {
    match value.get(field) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Object(map)) => Ok(map
            .iter()
            .map(|(key, value)| {
                value
                    .as_str()
                    .map(|text| (key.clone(), text.to_string()))
                    .ok_or_else(|| {
                        WasmPythonError::BridgeJson(format!(
                            "field '{field}' object values must be strings"
                        ))
                    })
            })
            .collect::<Result<Vec<_>, _>>()?),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| {
                let pair = item.as_array().ok_or_else(|| {
                    WasmPythonError::BridgeJson(format!(
                        "field '{field}' header entry must be an array"
                    ))
                })?;
                if pair.len() != 2 {
                    return Err(WasmPythonError::BridgeJson(format!(
                        "field '{field}' header entry must have two items"
                    )));
                }
                let key = pair[0].as_str().ok_or_else(|| {
                    WasmPythonError::BridgeJson(format!(
                        "field '{field}' header name must be a string"
                    ))
                })?;
                let val = pair[1].as_str().ok_or_else(|| {
                    WasmPythonError::BridgeJson(format!(
                        "field '{field}' header value must be a string"
                    ))
                })?;
                Ok((key.to_string(), val.to_string()))
            })
            .collect(),
        _ => Err(WasmPythonError::BridgeJson(format!(
            "field '{field}' must be an object or array"
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
    use crate::host::wasm_app::bindings::plexi::platform::types::HttpResponse;
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
    fn effects_decode_http_fetch() {
        let effects = decode_effects(
            r#"[
                {
                    "type":"HttpFetch",
                    "url":"https://api.example.test/items",
                    "method":"POST",
                    "headers":{"Accept":"application/json"},
                    "body":[111,107]
                }
            ]"#,
        )
        .expect("effects");

        let PythonBridgeEffect::Host(Effect::HttpFetch(req)) = &effects[0] else {
            panic!("expected http fetch");
        };
        assert_eq!(req.url, "https://api.example.test/items");
        assert_eq!(req.method, "POST");
        assert_eq!(
            req.headers,
            vec![("Accept".to_string(), "application/json".to_string())]
        );
        assert_eq!(req.body, Some(b"ok".to_vec()));
    }

    #[test]
    fn http_response_event_encodes_sdk_v3_shape() {
        let encoded = encode_input_event(&InputEvent::HttpResponse(HttpResponse {
            status: 200,
            headers: vec![("content-type".to_string(), "application/json".to_string())],
            body: b"ok".to_vec(),
        }))
        .expect("event");

        assert_eq!(encoded["type"], "HttpResponse");
        assert_eq!(encoded["status"], 200);
        assert_eq!(encoded["body"], json!([111, 107]));
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

    #[test]
    fn ui_tree_decodes_interactive_nodes() {
        let tree = decode_ui_tree(
            r#"{
                "root":0,
                "nodes":[
                    {"id":0,"key":"0","data":{"type":"Column","children":[1,2,3,4],"gap":4.0}},
                    {"id":1,"key":"0/input","data":{"type":"TextInput","value":"draft","placeholder":"New item","on_change":"draft","on_submit":"submit"}},
                    {"id":2,"key":"0/item","data":{"type":"Text","text":"Write tests"}},
                    {"id":3,"key":"0/list","data":{"type":"ListView","items":[2],"selected":0}},
                    {"id":4,"key":"0/progress","data":{"type":"ProgressBar","value":3.0,"max":5.0,"label":"3 / 5"}}
                ]
            }"#,
        )
        .expect("tree");

        assert!(matches!(tree.nodes[1].data, UiNodeData::TextInput(_)));
        assert!(matches!(tree.nodes[3].data, UiNodeData::ListView(_)));
        assert!(matches!(tree.nodes[4].data, UiNodeData::ProgressBar(_)));
    }

    #[test]
    fn ui_tree_decodes_canvas_node() {
        let tree = decode_ui_tree(
            r##"{
                "root":0,
                "nodes":[
                    {"id":0,"key":"0","data":{
                        "type":"Canvas",
                        "width":320.0,
                        "height":180.0,
                        "grow":true,
                        "commands":[
                            {"type":"rect","x":1.0,"y":2.0,"w":30.0,"h":40.0,"fill":"#112233","radius":2.0},
                            {"type":"text","x":9.0,"y":10.0,"text":"ok","size":14.0,"color":"#ffffffcc","bold":true,"align":"center"}
                        ]
                    }}
                ]
            }"##,
        )
        .expect("tree");

        let UiNodeData::Canvas(canvas) = &tree.nodes[0].data else {
            panic!("expected canvas node");
        };
        assert_eq!(canvas.width, 320.0);
        assert_eq!(canvas.commands.len(), 2);
        assert!(matches!(canvas.commands[0], CanvasCommand::Rect(_)));
        assert!(matches!(canvas.commands[1], CanvasCommand::Text(_)));
    }
}

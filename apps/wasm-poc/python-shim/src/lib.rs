// CPython lifecycle shim boundary POC.
//
// This component deliberately does not embed CPython yet. It exports the real
// Plexi lifecycle and exercises the same SDK v3 JSON bridge shapes the eventual
// CPython-backed shim must use: lifecycle arg JSON in, effect/UI JSON out, then
// conversion to WIT effects and UiTree.

wit_bindgen::generate!({
    world: "plexi-app",
    path: "wit/world.wit",
});

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use exports::plexi::platform::lifecycle::Guest;
use plexi::platform::host_log;
use plexi::platform::host_state;
use plexi::platform::types::{
    Alignment, ButtonNode, ButtonStyle, ColumnNode, Effect, IndexedNode, InputEvent, KeyEvent,
    StateSnapshot, TextNode, TimerEffect, UiActionEvent, UiNodeData, UiTree,
};
use serde_json::{json, Value};

const INCREMENT_HANDLER: &str = "increment";

struct Component;

impl Guest for Component {
    fn init(state: StateSnapshot, size: (f32, f32), args: Vec<String>) -> Vec<Effect> {
        host_log::info(
            "python-shim: lifecycle component reached; CPython embed remains the blocker",
        );
        let arg = json!({
            "state": encode_state(&state),
            "size": [size.0, size.1],
            "args": args,
        });
        bridge_effects("init", &arg)
    }

    fn update(event: InputEvent) -> Vec<Effect> {
        let snapshot = host_state::snapshot();
        let arg = json!({
            "state": encode_state(&snapshot),
            "event": encode_event(&event),
        });
        bridge_effects("update", &arg)
    }

    fn view() -> UiTree {
        let snapshot = host_state::snapshot();
        let arg = json!({ "state": encode_state(&snapshot) });
        let output = call_sdk_bridge("view", &arg);
        match decode_ui_tree(&output) {
            Ok(tree) => tree,
            Err(err) => error_tree(&format!("python-shim view error: {err}")),
        }
    }
}

fn bridge_effects(fn_name: &str, arg: &Value) -> Vec<Effect> {
    let output = call_sdk_bridge(fn_name, arg);
    match decode_effects(&output) {
        Ok(effects) => effects,
        Err(err) => {
            host_log::error(&format!("python-shim bridge error: {err}"));
            vec![Effect::SetStatus(format!(
                "python-shim bridge error: {err}"
            ))]
        }
    }
}

fn call_sdk_bridge(fn_name: &str, arg: &Value) -> String {
    match fn_name {
        "init" => json!([
            {"type":"SetTitle","title":"Python Shim POC"},
            {"type":"SetState","data":{"count":0,"module":"shim_fixture"}}
        ])
        .to_string(),
        "update" => {
            let count = current_count(arg);
            let event = &arg["event"];
            let should_increment =
                event["type"] == "UiAction" && event["handler_id"] == INCREMENT_HANDLER;
            if should_increment {
                json!([{"type":"SetState","data":{"count":count + 1}}]).to_string()
            } else {
                "[]".to_string()
            }
        }
        "view" => view_json(current_count(arg)).to_string(),
        other => json!([{"type":"SetStatus","text":format!("unknown bridge function: {other}")}])
            .to_string(),
    }
}

fn view_json(count: u64) -> Value {
    json!({
        "root": 3,
        "nodes": [
            {"id":0,"key":"title","data":{"type":"Text","text":"Python Shim POC","bold":true,"align":"start"}},
            {"id":1,"key":"count","data":{"type":"Text","text":format!("Count: {count}"),"align":"start"}},
            {"id":2,"key":"increment","data":{"type":"Button","label":"Increment","on_click":INCREMENT_HANDLER,"style":"primary"}},
            {"id":3,"key":"root","data":{"type":"Column","children":[0,1,2],"gap":8.0,"align":"start","grow":true}}
        ]
    })
}

fn encode_state(snapshot: &StateSnapshot) -> Value {
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

fn current_count(arg: &Value) -> u64 {
    arg["state"]["count"]
        .as_str()
        .map(|encoded| encoded.strip_prefix("b64:").unwrap_or(encoded))
        .and_then(|encoded| BASE64.decode(encoded).ok())
        .and_then(|bytes| serde_json::from_slice::<u64>(&bytes).ok())
        .unwrap_or(0)
}

fn encode_event(event: &InputEvent) -> Value {
    match event {
        InputEvent::UiAction(UiActionEvent { handler_id }) => {
            json!({"type":"UiAction","handler_id":handler_id})
        }
        InputEvent::Key(KeyEvent {
            key,
            modifiers,
            pressed,
        }) => json!({
            "type":"KeyEvent",
            "key": key,
            "modifiers": {
                "ctrl": modifiers.ctrl,
                "shift": modifiers.shift,
                "alt": modifiers.alt,
                "meta": modifiers.meta,
            },
            "pressed": pressed,
        }),
        InputEvent::TimerFired(id) => json!({"type":"TimerFired","id":id}),
        _ => json!({"type":"Unsupported"}),
    }
}

fn decode_effects(json_text: &str) -> Result<Vec<Effect>, String> {
    let values = serde_json::from_str::<Vec<Value>>(json_text).map_err(|e| e.to_string())?;
    let mut effects = Vec::new();
    for value in values {
        match value["type"].as_str().unwrap_or_default() {
            "SetTitle" => effects.push(Effect::SetTitle(required_string(&value, "title")?)),
            "SetStatus" => effects.push(Effect::SetStatus(required_string(&value, "text")?)),
            "SetTimer" => effects.push(Effect::SetTimer(TimerEffect {
                id: required_u32(&value, "id")?,
                delay_ms: required_u32(&value, "delay_ms")?,
                repeat: value["repeat"].as_bool().unwrap_or(false),
            })),
            "SetState" => apply_set_state(&value)?,
            other => return Err(format!("unknown effect type: {other}")),
        }
    }
    Ok(effects)
}

fn apply_set_state(value: &Value) -> Result<(), String> {
    let data = value["data"]
        .as_object()
        .ok_or_else(|| "SetState.data must be an object".to_string())?;
    for (key, value) in data {
        let bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
        host_state::set(key, &bytes)?;
    }
    Ok(())
}

fn decode_ui_tree(json_text: &str) -> Result<UiTree, String> {
    let value = serde_json::from_str::<Value>(json_text).map_err(|e| e.to_string())?;
    let root = required_u32(&value, "root")?;
    let nodes = value["nodes"]
        .as_array()
        .ok_or_else(|| "ui tree missing nodes array".to_string())?
        .iter()
        .map(decode_indexed_node)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(UiTree { root, nodes })
}

fn decode_indexed_node(value: &Value) -> Result<IndexedNode, String> {
    Ok(IndexedNode {
        id: required_u32(value, "id")?,
        key: required_string(value, "key")?,
        data: decode_node_data(&value["data"])?,
    })
}

fn decode_node_data(value: &Value) -> Result<UiNodeData, String> {
    match value["type"].as_str().unwrap_or_default() {
        "Text" => Ok(UiNodeData::Text(TextNode {
            text: required_string(value, "text")?,
            size: optional_f32(value, "size")?,
            bold: value["bold"].as_bool().unwrap_or(false),
            color: None,
            truncate: value["truncate"].as_bool().unwrap_or(false),
            align: decode_alignment(value["align"].as_str().unwrap_or("start"))?,
        })),
        "Button" => Ok(UiNodeData::Button(ButtonNode {
            label: required_string(value, "label")?,
            on_click: required_string(value, "on_click")?,
            style: decode_button_style(value["style"].as_str().unwrap_or("secondary"))?,
            disabled: value["disabled"].as_bool().unwrap_or(false),
        })),
        "Column" => Ok(UiNodeData::Column(ColumnNode {
            children: value["children"]
                .as_array()
                .ok_or_else(|| "Column.children must be an array".to_string())?
                .iter()
                .map(|v| {
                    v.as_u64()
                        .and_then(|n| u32::try_from(n).ok())
                        .ok_or_else(|| "Column.children contains non-u32".to_string())
                })
                .collect::<Result<Vec<_>, _>>()?,
            gap: optional_f32(value, "gap")?.unwrap_or(0.0),
            align: decode_alignment(value["align"].as_str().unwrap_or("start"))?,
            grow: value["grow"].as_bool().unwrap_or(false),
        })),
        other => Err(format!("unknown ui node type: {other}")),
    }
}

fn error_tree(message: &str) -> UiTree {
    UiTree {
        root: 0,
        nodes: vec![IndexedNode {
            id: 0,
            key: "error".to_string(),
            data: UiNodeData::Text(TextNode {
                text: message.to_string(),
                size: Some(13.0),
                bold: false,
                color: None,
                truncate: false,
                align: Alignment::Start,
            }),
        }],
    }
}

fn required_string(value: &Value, field: &str) -> Result<String, String> {
    value[field]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| format!("missing string field '{field}'"))
}

fn required_u32(value: &Value, field: &str) -> Result<u32, String> {
    value[field]
        .as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(|| format!("missing u32 field '{field}'"))
}

fn optional_f32(value: &Value, field: &str) -> Result<Option<f32>, String> {
    value
        .get(field)
        .map(|v| {
            v.as_f64()
                .map(|n| n as f32)
                .ok_or_else(|| format!("field '{field}' must be a number"))
        })
        .transpose()
}

fn decode_alignment(value: &str) -> Result<Alignment, String> {
    match value {
        "start" => Ok(Alignment::Start),
        "center" => Ok(Alignment::Center),
        "end" => Ok(Alignment::End),
        "stretch" => Ok(Alignment::Stretch),
        other => Err(format!("unknown alignment: {other}")),
    }
}

fn decode_button_style(value: &str) -> Result<ButtonStyle, String> {
    match value {
        "primary" => Ok(ButtonStyle::Primary),
        "secondary" => Ok(ButtonStyle::Secondary),
        "danger" => Ok(ButtonStyle::Danger),
        "ghost" => Ok(ButtonStyle::Ghost),
        other => Err(format!("unknown button style: {other}")),
    }
}

export!(Component);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_bridge_returns_title_and_state() {
        let output = call_sdk_bridge("init", &json!({"state":{},"size":[320.0,240.0],"args":[]}));

        assert!(output.contains("\"SetTitle\""));
        assert!(output.contains("\"SetState\""));
    }

    #[test]
    fn update_bridge_increments_count() {
        let arg = json!({
            "state": {"count": BASE64.encode(b"2")},
            "event": {"type":"UiAction","handler_id":INCREMENT_HANDLER},
        });

        let output = call_sdk_bridge("update", &arg);

        assert!(output.contains("\"count\":3"));
    }

    #[test]
    fn view_bridge_decodes_text_tree() {
        let tree = decode_ui_tree(&view_json(7).to_string()).expect("tree");

        assert!(tree
            .nodes
            .iter()
            .any(|node| matches!(&node.data, UiNodeData::Text(text) if text.text == "Count: 7")));
    }
}

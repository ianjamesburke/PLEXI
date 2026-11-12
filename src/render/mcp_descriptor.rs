//! MCP schema adaptation for the shared descriptor list and form widgets.
use crate::app::plexi_descriptor::{ArgSpec, ArgType, Command};
use serde_json::{json, Value};
use std::collections::HashMap;

pub fn tool_command(tool: &Value) -> Result<Command, String> {
    let name = tool["name"].as_str().ok_or("MCP tool has no name")?;
    let schema = &tool["inputSchema"];
    let required = schema["required"].as_array();
    let mut args = Vec::new();
    if let Some(properties) = schema["properties"].as_object() {
        for (name, field) in properties {
            let enums = field["enum"].as_array();
            let ty = if enums.is_some() {
                ArgType::Enum
            } else {
                match field["type"].as_str() {
                    Some("integer") => ArgType::Int,
                    Some("number") => ArgType::Float,
                    Some("boolean") => ArgType::Bool,
                    _ => ArgType::String,
                }
            };
            args.push(ArgSpec {
                name: name.clone(),
                ty,
                required: Some(required.is_some_and(|items| items.contains(&json!(name)))),
                default: field.get("default").cloned(),
                description: field["description"].as_str().map(str::to_owned),
                placeholder: matches!(field["type"].as_str(), Some("array" | "object"))
                    .then(|| "JSON".into()),
                enum_values: enums.map(|items| items.iter().map(display_value).collect()),
                min: field["minimum"].as_f64(),
                max: field["maximum"].as_f64(),
            });
        }
    }
    serde_json::from_value(json!({"name":name,"description":tool.get("description"),"args":args}))
        .map_err(|e| e.to_string())
}

pub fn display_value(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string())
}

pub fn arguments(
    command: &Command,
    schema: &Value,
    values: &HashMap<String, String>,
) -> Result<Value, String> {
    let mut result = serde_json::Map::new();
    for field in &command.args {
        let raw = values.get(&field.name).map(String::as_str).unwrap_or("");
        let spec = &schema["properties"][&field.name];
        if raw.is_empty() && field.ty != ArgType::Bool {
            if field.required == Some(true) {
                return Err(format!("{} is required", field.name));
            }
            continue;
        }
        let value = if let Some(choices) = spec["enum"].as_array() {
            choices
                .iter()
                .find(|v| display_value(v) == raw)
                .cloned()
                .ok_or_else(|| format!("{}: select an enum value", field.name))?
        } else {
            match spec["type"].as_str() {
                Some("boolean") => json!(raw == "true"),
                Some("integer") => json!(raw
                    .parse::<i64>()
                    .map_err(|_| format!("{}: expected integer", field.name))?),
                Some("number") => {
                    let n = raw
                        .parse::<f64>()
                        .ok()
                        .filter(|n| n.is_finite())
                        .ok_or_else(|| format!("{}: expected finite number", field.name))?;
                    json!(n)
                }
                Some("array" | "object") => {
                    let value: Value =
                        serde_json::from_str(raw).map_err(|e| format!("{}: {e}", field.name))?;
                    if (spec["type"] == "array" && !value.is_array())
                        || (spec["type"] == "object" && !value.is_object())
                    {
                        return Err(format!("{}: expected {} JSON", field.name, spec["type"]));
                    }
                    value
                }
                _ => json!(raw),
            }
        };
        if let Some(n) = value.as_f64() {
            if field.min.is_some_and(|min| n < min) || field.max.is_some_and(|max| n > max) {
                return Err(format!("{}: number outside allowed range", field.name));
            }
        }
        result.insert(field.name.clone(), value);
    }
    Ok(Value::Object(result))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn captured_tools_all_generate_forms() {
        for fixture in [
            include_str!("../host/testdata/mcp/filesystem_tools_list.json"),
            include_str!("../host/testdata/mcp/everything_tools_list.json"),
        ] {
            let value: Value = serde_json::from_str(fixture).unwrap();
            for tool in value["result"]["tools"].as_array().unwrap() {
                let command = tool_command(tool).unwrap();
                assert_eq!(
                    command.args.len(),
                    tool["inputSchema"]["properties"]
                        .as_object()
                        .map_or(0, |p| p.len())
                );
                assert_eq!(command.name, tool["name"]);
            }
        }
    }
    #[test]
    fn typed_values_and_optional_blanks() {
        let tool = json!({"name":"test","inputSchema":{"type":"object","required":["flag","count"],"properties":{
            "flag":{"type":"boolean"},"count":{"type":"integer"},"optional":{"type":"number"},"list":{"type":"array"},"choice":{"enum":[1,2]},"obj":{"type":"object"}
        }}});
        let command = tool_command(&tool).unwrap();
        let mut values = HashMap::new();
        assert!(arguments(&command, &tool["inputSchema"], &values).is_err());
        values.extend([
            ("count".into(), "4".into()),
            ("list".into(), "[1]".into()),
            ("choice".into(), "2".into()),
            ("obj".into(), "{}".into()),
        ]);
        assert_eq!(
            arguments(&command, &tool["inputSchema"], &values).unwrap(),
            json!({"flag":false,"count":4,"list":[1],"choice":2,"obj":{}})
        );
        values.insert("obj".into(), "[]".into());
        assert!(arguments(&command, &tool["inputSchema"], &values).is_err());
    }
}

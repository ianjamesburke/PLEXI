//! Schema-validated structured result storage and validation for agent panes.
//!
//! Spawners can declare a JSON Schema on a pane at spawn time (`--result-schema`)
//! or via `plexi pane result schema`. The agent writes its structured result once
//! via `plexi pane result write <json>`; the host validates the payload against
//! the pane's schema and rejects malformed payloads synchronously back to the
//! agent so it can self-correct before any consumer sees the result.

use std::path::Path;

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Per-pane structured result state retained by the host across the pane's lifetime
/// (and after pane close so spawners can read results after an ephemeral worker exits).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PaneResultState {
    /// Optional JSON Schema declared for this pane's structured return value.
    pub schema: Option<Value>,
    /// Validated JSON result committed once by the pane.
    pub committed: Option<Value>,
    /// Error summary from the most recent rejected write attempt, if no valid result
    /// has been committed yet. Cleared once a valid result is committed.
    pub last_invalid_error: Option<String>,
    /// Detailed validation errors from the most recent rejected write attempt.
    pub last_validation_errors: Vec<String>,
    /// True once the pane has been closed.
    pub pane_closed: bool,
}

/// Reason a `plexi pane result write` call was rejected by the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteResultError {
    /// A valid result was already committed for this pane (write-once contract).
    AlreadySet,
    /// The payload was not valid JSON or failed schema validation.
    Invalid {
        summary: String,
        validation_errors: Vec<String>,
    },
}

impl WriteResultError {
    pub fn error_code(&self) -> &'static str {
        match self {
            Self::AlreadySet => "result_already_set",
            Self::Invalid { .. } => "invalid_result",
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::AlreadySet => {
                "pane result has already been committed (pane result is write-once)".to_string()
            }
            Self::Invalid { summary, .. } => summary.clone(),
        }
    }

    pub fn validation_errors(&self) -> &[String] {
        match self {
            Self::AlreadySet => &[],
            Self::Invalid {
                validation_errors, ..
            } => validation_errors,
        }
    }
}

impl PaneResultState {
    /// Declare or update the JSON Schema for this pane before a result is committed.
    pub fn set_schema(&mut self, schema: Value, base_dir: Option<&Path>) -> Result<Value, String> {
        if self.committed.is_some() {
            return Err(
                "cannot change result schema after a result has already been committed".to_string(),
            );
        }
        let normalized = normalize_schema_value(&schema, base_dir)?;
        self.schema = Some(normalized.clone());
        Ok(normalized)
    }

    /// Validate and commit a structured result payload (write-once).
    ///
    /// On malformed JSON or schema violation, records the error in `last_invalid_error`
    /// without committing a result, and returns `Err(WriteResultError::Invalid { .. })`
    /// so the caller receives a synchronous rejection and can self-correct.
    pub fn write_payload(&mut self, raw_payload: &str) -> Result<Value, WriteResultError> {
        if self.committed.is_some() {
            return Err(WriteResultError::AlreadySet);
        }

        let trimmed = raw_payload.trim();
        if trimmed.is_empty() {
            let msg = "result payload must not be empty".to_string();
            self.last_invalid_error = Some(msg.clone());
            self.last_validation_errors = vec![msg.clone()];
            return Err(WriteResultError::Invalid {
                summary: format!("invalid result payload: {msg}"),
                validation_errors: vec![msg],
            });
        }

        let parsed: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(e) => {
                let msg = format!("malformed JSON: {e}");
                self.last_invalid_error = Some(msg.clone());
                self.last_validation_errors = vec![msg.clone()];
                return Err(WriteResultError::Invalid {
                    summary: format!("invalid result payload: {msg}"),
                    validation_errors: vec![msg],
                });
            }
        };

        if let Some(ref schema) = self.schema {
            if let Err(errors) = validate_instance(schema, &parsed) {
                let joined = errors.join("; ");
                let summary = format!("result failed schema validation: {joined}");
                self.last_invalid_error = Some(summary.clone());
                self.last_validation_errors = errors.clone();
                return Err(WriteResultError::Invalid {
                    summary,
                    validation_errors: errors,
                });
            }
        } else if !parsed.is_object() {
            let msg = format!(
                "$: expected a JSON object for pane result, got {}",
                json_type_name(&parsed)
            );
            self.last_invalid_error = Some(msg.clone());
            self.last_validation_errors = vec![msg.clone()];
            return Err(WriteResultError::Invalid {
                summary: format!("result failed validation: {msg}"),
                validation_errors: vec![msg],
            });
        }

        self.last_invalid_error = None;
        self.last_validation_errors.clear();
        self.committed = Some(parsed.clone());
        Ok(parsed)
    }
}

/// Parse a `<json-or-path>` string into a validated JSON Schema object.
///
/// If `raw` starts with `{`, it is parsed directly as inline JSON (with a fallback
/// to file reading if a file of that exact name exists). Otherwise, `raw` is
/// resolved against `base_dir` (when relative) and read from disk if it exists,
/// or parsed as inline JSON.
pub fn parse_schema_spec(raw: &str, base_dir: Option<&Path>) -> Result<Value, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("result schema must not be empty".to_string());
    }

    let candidate_path = {
        let p = Path::new(trimmed);
        if p.is_absolute() {
            p.to_path_buf()
        } else if let Some(base) = base_dir {
            base.join(p)
        } else {
            p.to_path_buf()
        }
    };

    let json_val: Value = if trimmed.starts_with('{') {
        match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(json_err) => {
                if candidate_path.is_file() {
                    let content = std::fs::read_to_string(&candidate_path).map_err(|io_err| {
                        format!(
                            "failed to read schema file {}: {io_err}",
                            candidate_path.display()
                        )
                    })?;
                    serde_json::from_str(&content).map_err(|e| {
                        format!(
                            "invalid JSON in schema file {}: {e}",
                            candidate_path.display()
                        )
                    })?
                } else {
                    return Err(format!("invalid JSON in result schema: {json_err}"));
                }
            }
        }
    } else if candidate_path.is_file() {
        let content = std::fs::read_to_string(&candidate_path).map_err(|io_err| {
            format!(
                "failed to read schema file {}: {io_err}",
                candidate_path.display()
            )
        })?;
        serde_json::from_str(&content).map_err(|e| {
            format!(
                "invalid JSON in schema file {}: {e}",
                candidate_path.display()
            )
        })?
    } else {
        match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(json_err) => {
                if trimmed.ends_with(".json")
                    || trimmed.contains('/')
                    || trimmed.contains(std::path::MAIN_SEPARATOR)
                {
                    return Err(format!(
                        "schema file not found: {} (and not valid inline JSON: {json_err})",
                        candidate_path.display()
                    ));
                }
                return Err(format!("invalid JSON in result schema: {json_err}"));
            }
        }
    };

    validate_schema_definition(&json_val)?;
    Ok(json_val)
}

/// Normalize a `serde_json::Value` passed as a schema (either already a JSON object
/// or a JSON string containing inline JSON / a path) and validate its structure.
pub fn normalize_schema_value(schema: &Value, base_dir: Option<&Path>) -> Result<Value, String> {
    if let Some(raw_str) = schema.as_str() {
        return parse_schema_spec(raw_str, base_dir);
    }
    validate_schema_definition(schema)?;
    Ok(schema.clone())
}

/// Validate that `schema` is a well-formed JSON Schema object.
pub fn validate_schema_definition(schema: &Value) -> Result<(), String> {
    let obj = schema
        .as_object()
        .ok_or_else(|| "result schema must be a JSON object".to_string())?;
    if obj.is_empty() {
        // An empty object `{}` is a valid catch-all JSON Schema.
        return Ok(());
    }
    validate_subschema_definition(schema, "$")
}

fn validate_subschema_definition(schema: &Value, path: &str) -> Result<(), String> {
    match schema {
        Value::Bool(_) => return Ok(()),
        Value::Object(_) => {}
        _ => {
            return Err(format!(
                "{path}: schema must be a JSON object or boolean, got {}",
                json_type_name(schema)
            ));
        }
    }

    let obj = schema.as_object().expect("checked object above");

    if let Some(ty) = obj.get("type") {
        match ty {
            Value::String(s) => validate_type_keyword(s, &format!("{path}.type"))?,
            Value::Array(arr) => {
                if arr.is_empty() {
                    return Err(format!("{path}.type: type array must not be empty"));
                }
                for (idx, item) in arr.iter().enumerate() {
                    let s = item
                        .as_str()
                        .ok_or_else(|| format!("{path}.type[{idx}]: type name must be a string"))?;
                    validate_type_keyword(s, &format!("{path}.type[{idx}]"))?;
                }
            }
            _ => {
                return Err(format!(
                    "{path}.type: expected string or array of strings, got {}",
                    json_type_name(ty)
                ));
            }
        }
    }

    if let Some(props) = obj.get("properties") {
        let props_obj = props.as_object().ok_or_else(|| {
            format!("{path}.properties: expected an object mapping property names to schemas")
        })?;
        for (k, v) in props_obj {
            validate_subschema_definition(v, &format!("{path}.properties.{k}"))?;
        }
    }

    if let Some(req) = obj.get("required") {
        let req_arr = req.as_array().ok_or_else(|| {
            format!("{path}.required: expected an array of property name strings")
        })?;
        for (idx, item) in req_arr.iter().enumerate() {
            if !item.is_string() {
                return Err(format!(
                    "{path}.required[{idx}]: required property name must be a string"
                ));
            }
        }
    }

    if let Some(addl) = obj.get("additionalProperties") {
        match addl {
            Value::Bool(_) => {}
            Value::Object(_) => {
                validate_subschema_definition(addl, &format!("{path}.additionalProperties"))?;
            }
            _ => {
                return Err(format!(
                    "{path}.additionalProperties: expected boolean or schema object"
                ));
            }
        }
    }

    if let Some(pat_props) = obj.get("patternProperties") {
        let pat_obj = pat_props.as_object().ok_or_else(|| {
            format!("{path}.patternProperties: expected an object mapping regexes to schemas")
        })?;
        for (pattern, subschema) in pat_obj {
            Regex::new(pattern).map_err(|e| {
                format!("{path}.patternProperties[{pattern:?}]: invalid regex: {e}")
            })?;
            validate_subschema_definition(
                subschema,
                &format!("{path}.patternProperties[{pattern:?}]"),
            )?;
        }
    }

    if let Some(items) = obj.get("items") {
        match items {
            Value::Object(_) | Value::Bool(_) => {
                validate_subschema_definition(items, &format!("{path}.items"))?;
            }
            Value::Array(arr) => {
                for (idx, subschema) in arr.iter().enumerate() {
                    validate_subschema_definition(subschema, &format!("{path}.items[{idx}]"))?;
                }
            }
            _ => {
                return Err(format!(
                    "{path}.items: expected a schema object or array of schemas"
                ));
            }
        }
    }

    if let Some(prefix_items) = obj.get("prefixItems") {
        let arr = prefix_items
            .as_array()
            .ok_or_else(|| format!("{path}.prefixItems: expected an array of schema objects"))?;
        for (idx, subschema) in arr.iter().enumerate() {
            validate_subschema_definition(subschema, &format!("{path}.prefixItems[{idx}]"))?;
        }
    }

    if let Some(en) = obj.get("enum") {
        let arr = en
            .as_array()
            .ok_or_else(|| format!("{path}.enum: expected a non-empty array"))?;
        if arr.is_empty() {
            return Err(format!("{path}.enum: enum array must not be empty"));
        }
    }

    if let Some(pat) = obj.get("pattern") {
        let pat_str = pat
            .as_str()
            .ok_or_else(|| format!("{path}.pattern: expected a regex string"))?;
        Regex::new(pat_str).map_err(|e| format!("{path}.pattern: invalid regex: {e}"))?;
    }

    for kw in [
        "minLength",
        "maxLength",
        "minItems",
        "maxItems",
        "minProperties",
        "maxProperties",
    ] {
        if let Some(v) = obj.get(kw) {
            if v.as_u64().is_none() {
                return Err(format!("{path}.{kw}: expected a non-negative integer"));
            }
        }
    }

    for kw in ["minimum", "maximum", "exclusiveMinimum", "exclusiveMaximum"] {
        if let Some(v) = obj.get(kw) {
            if !v.is_number() {
                return Err(format!("{path}.{kw}: expected a number"));
            }
        }
    }

    if let Some(u) = obj.get("uniqueItems") {
        if !u.is_boolean() {
            return Err(format!("{path}.uniqueItems: expected a boolean"));
        }
    }

    for comb in ["allOf", "anyOf", "oneOf"] {
        if let Some(v) = obj.get(comb) {
            let arr = v
                .as_array()
                .ok_or_else(|| format!("{path}.{comb}: expected a non-empty array of schemas"))?;
            if arr.is_empty() {
                return Err(format!("{path}.{comb}: array must not be empty"));
            }
            for (idx, subschema) in arr.iter().enumerate() {
                validate_subschema_definition(subschema, &format!("{path}.{comb}[{idx}]"))?;
            }
        }
    }

    if let Some(not_schema) = obj.get("not") {
        validate_subschema_definition(not_schema, &format!("{path}.not"))?;
    }

    for defs_kw in ["$defs", "definitions"] {
        if let Some(defs) = obj.get(defs_kw) {
            let defs_obj = defs
                .as_object()
                .ok_or_else(|| format!("{path}.{defs_kw}: expected an object of definitions"))?;
            for (k, subschema) in defs_obj {
                validate_subschema_definition(subschema, &format!("{path}.{defs_kw}.{k}"))?;
            }
        }
    }

    if let Some(r) = obj.get("$ref") {
        let ref_str = r
            .as_str()
            .ok_or_else(|| format!("{path}.$ref: expected a string reference"))?;
        if !ref_str.starts_with('#') {
            return Err(format!(
                "{path}.$ref: only local document references ('#/...')"
            ));
        }
    }

    Ok(())
}

fn validate_type_keyword(ty: &str, path: &str) -> Result<(), String> {
    match ty {
        "object" | "array" | "string" | "number" | "integer" | "boolean" | "null" => Ok(()),
        other => Err(format!(
            "{path}: unknown JSON Schema type {other:?} (expected one of: object, array, string, number, integer, boolean, null)"
        )),
    }
}

/// Validate a JSON `instance` against a validated JSON `schema`.
///
/// Returns `Ok(())` when `instance` conforms to `schema`, or `Err(errors)` with one
/// or more human-readable validation error strings referencing the offending JSON path.
pub fn validate_instance(schema: &Value, instance: &Value) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();
    validate_node(schema, schema, instance, "$", &mut errors, 0);
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

const MAX_REF_DEPTH: usize = 32;

fn validate_node(
    root_schema: &Value,
    schema: &Value,
    instance: &Value,
    path: &str,
    errors: &mut Vec<String>,
    depth: usize,
) {
    if depth > MAX_REF_DEPTH {
        errors.push(format!("{path}: maximum schema reference depth exceeded"));
        return;
    }

    match schema {
        Value::Bool(true) => return,
        Value::Bool(false) => {
            errors.push(format!("{path}: value is disallowed by false schema"));
            return;
        }
        Value::Object(_) => {}
        _ => {
            errors.push(format!("{path}: invalid schema node"));
            return;
        }
    }

    let obj = schema.as_object().expect("checked object above");

    // Resolve $ref if present
    if let Some(Value::String(ref_str)) = obj.get("$ref") {
        match resolve_local_ref(root_schema, ref_str) {
            Some(target) => {
                validate_node(root_schema, target, instance, path, errors, depth + 1);
            }
            None => {
                errors.push(format!("{path}: unresolved schema $ref {ref_str:?}"));
                return;
            }
        }
    }

    // 1. type check
    if let Some(ty) = obj.get("type") {
        let matches = match ty {
            Value::String(expected) => instance_matches_type(instance, expected),
            Value::Array(types) => types
                .iter()
                .filter_map(|v| v.as_str())
                .any(|expected| instance_matches_type(instance, expected)),
            _ => true,
        };
        if !matches {
            let expected_desc = match ty {
                Value::String(s) => format!("{s:?}"),
                Value::Array(arr) => {
                    let names: Vec<String> = arr
                        .iter()
                        .filter_map(|v| v.as_str())
                        .map(|s| format!("{s:?}"))
                        .collect();
                    format!("one of [{}]", names.join(", "))
                }
                _ => "valid type".to_string(),
            };
            errors.push(format!(
                "{path}: expected type {expected_desc}, got {}",
                json_type_name(instance)
            ));
            return;
        }
    }

    // 2. const check
    if let Some(const_val) = obj.get("const") {
        if instance != const_val {
            errors.push(format!(
                "{path}: value {} does not match required const {}",
                compact_json(instance),
                compact_json(const_val)
            ));
        }
    }

    // 3. enum check
    if let Some(Value::Array(enum_vals)) = obj.get("enum") {
        if !enum_vals.iter().any(|candidate| candidate == instance) {
            let allowed: Vec<String> = enum_vals.iter().map(compact_json).collect();
            errors.push(format!(
                "{path}: value {} is not one of the allowed enum values [{}]",
                compact_json(instance),
                allowed.join(", ")
            ));
        }
    }

    // 4. String constraints
    if let Some(s) = instance.as_str() {
        let char_len = s.chars().count() as u64;
        if let Some(min_len) = obj.get("minLength").and_then(|v| v.as_u64()) {
            if char_len < min_len {
                errors.push(format!(
                    "{path}: string length {char_len} is less than minLength {min_len}"
                ));
            }
        }
        if let Some(max_len) = obj.get("maxLength").and_then(|v| v.as_u64()) {
            if char_len > max_len {
                errors.push(format!(
                    "{path}: string length {char_len} exceeds maxLength {max_len}"
                ));
            }
        }
        if let Some(pat_str) = obj.get("pattern").and_then(|v| v.as_str()) {
            if let Ok(re) = Regex::new(pat_str) {
                if !re.is_match(s) {
                    errors.push(format!(
                        "{path}: string {s:?} does not match pattern {pat_str:?}"
                    ));
                }
            }
        }
    }

    // 5. Numeric constraints
    if let Some(num) = instance.as_f64() {
        if let Some(min) = obj.get("minimum").and_then(|v| v.as_f64()) {
            if num < min {
                errors.push(format!("{path}: value {num} is less than minimum {min}"));
            }
        }
        if let Some(max) = obj.get("maximum").and_then(|v| v.as_f64()) {
            if num > max {
                errors.push(format!("{path}: value {num} exceeds maximum {max}"));
            }
        }
        if let Some(ex_min) = obj.get("exclusiveMinimum").and_then(|v| v.as_f64()) {
            if num <= ex_min {
                errors.push(format!(
                    "{path}: value {num} must be greater than exclusiveMinimum {ex_min}"
                ));
            }
        }
        if let Some(ex_max) = obj.get("exclusiveMaximum").and_then(|v| v.as_f64()) {
            if num >= ex_max {
                errors.push(format!(
                    "{path}: value {num} must be less than exclusiveMaximum {ex_max}"
                ));
            }
        }
    }

    // 6. Array constraints
    if let Some(arr) = instance.as_array() {
        let len = arr.len() as u64;
        if let Some(min_items) = obj.get("minItems").and_then(|v| v.as_u64()) {
            if len < min_items {
                errors.push(format!(
                    "{path}: array length {len} is less than minItems {min_items}"
                ));
            }
        }
        if let Some(max_items) = obj.get("maxItems").and_then(|v| v.as_u64()) {
            if len > max_items {
                errors.push(format!(
                    "{path}: array length {len} exceeds maxItems {max_items}"
                ));
            }
        }
        if obj.get("uniqueItems").and_then(|v| v.as_bool()) == Some(true) {
            for i in 0..arr.len() {
                for j in (i + 1)..arr.len() {
                    if arr[i] == arr[j] {
                        errors.push(format!(
                            "{path}: duplicate array item at indices {i} and {j} (uniqueItems is true)"
                        ));
                    }
                }
            }
        }

        let mut prefix_count = 0usize;
        if let Some(Value::Array(prefix_schemas)) = obj.get("prefixItems") {
            prefix_count = prefix_schemas.len();
            for (idx, (item, subschema)) in arr.iter().zip(prefix_schemas.iter()).enumerate() {
                validate_node(
                    root_schema,
                    subschema,
                    item,
                    &format!("{path}[{idx}]"),
                    errors,
                    depth + 1,
                );
            }
        }

        if let Some(items_schema) = obj.get("items") {
            match items_schema {
                Value::Object(_) | Value::Bool(_) => {
                    for (idx, item) in arr.iter().enumerate().skip(prefix_count) {
                        validate_node(
                            root_schema,
                            items_schema,
                            item,
                            &format!("{path}[{idx}]"),
                            errors,
                            depth + 1,
                        );
                    }
                }
                Value::Array(tuple_schemas) => {
                    for (idx, (item, subschema)) in arr.iter().zip(tuple_schemas.iter()).enumerate()
                    {
                        validate_node(
                            root_schema,
                            subschema,
                            item,
                            &format!("{path}[{idx}]"),
                            errors,
                            depth + 1,
                        );
                    }
                }
                _ => {}
            }
        }
    }

    // 7. Object constraints
    if let Some(inst_obj) = instance.as_object() {
        let prop_count = inst_obj.len() as u64;
        if let Some(min_props) = obj.get("minProperties").and_then(|v| v.as_u64()) {
            if prop_count < min_props {
                errors.push(format!(
                    "{path}: object has {prop_count} properties, less than minProperties {min_props}"
                ));
            }
        }
        if let Some(max_props) = obj.get("maxProperties").and_then(|v| v.as_u64()) {
            if prop_count > max_props {
                errors.push(format!(
                    "{path}: object has {prop_count} properties, exceeds maxProperties {max_props}"
                ));
            }
        }

        if let Some(Value::Array(req_arr)) = obj.get("required") {
            for req_val in req_arr {
                if let Some(req_key) = req_val.as_str() {
                    if !inst_obj.contains_key(req_key) {
                        errors.push(format!("{path}: missing required property {req_key:?}"));
                    }
                }
            }
        }

        let props_map = obj.get("properties").and_then(|v| v.as_object());
        let pattern_props_map = obj.get("patternProperties").and_then(|v| v.as_object());
        let additional_props = obj.get("additionalProperties");

        for (key, val) in inst_obj {
            let child_path = format!("{path}.{key}");
            let mut matched_known = false;

            if let Some(props) = props_map {
                if let Some(prop_schema) = props.get(key) {
                    matched_known = true;
                    validate_node(
                        root_schema,
                        prop_schema,
                        val,
                        &child_path,
                        errors,
                        depth + 1,
                    );
                }
            }

            if let Some(pat_props) = pattern_props_map {
                for (pattern, pat_schema) in pat_props {
                    if let Ok(re) = Regex::new(pattern) {
                        if re.is_match(key) {
                            matched_known = true;
                            validate_node(
                                root_schema,
                                pat_schema,
                                val,
                                &child_path,
                                errors,
                                depth + 1,
                            );
                        }
                    }
                }
            }

            if !matched_known {
                if let Some(addl) = additional_props {
                    match addl {
                        Value::Bool(false) => {
                            errors.push(format!(
                                "{path}: additional property {key:?} is not allowed"
                            ));
                        }
                        Value::Bool(true) => {}
                        Value::Object(_) => {
                            validate_node(root_schema, addl, val, &child_path, errors, depth + 1);
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    // 8. Combinators: allOf, anyOf, oneOf, not
    if let Some(Value::Array(all_of)) = obj.get("allOf") {
        for subschema in all_of {
            validate_node(root_schema, subschema, instance, path, errors, depth + 1);
        }
    }

    if let Some(Value::Array(any_of)) = obj.get("anyOf") {
        let mut branch_matched = false;
        let mut branch_errors = Vec::new();
        for subschema in any_of {
            let mut sub_errs = Vec::new();
            validate_node(
                root_schema,
                subschema,
                instance,
                path,
                &mut sub_errs,
                depth + 1,
            );
            if sub_errs.is_empty() {
                branch_matched = true;
                break;
            }
            branch_errors.extend(sub_errs);
        }
        if !branch_matched {
            errors.push(format!(
                "{path}: value did not match any branch of anyOf ({})",
                branch_errors.join("; ")
            ));
        }
    }

    if let Some(Value::Array(one_of)) = obj.get("oneOf") {
        let mut match_count = 0usize;
        let mut branch_errors = Vec::new();
        for subschema in one_of {
            let mut sub_errs = Vec::new();
            validate_node(
                root_schema,
                subschema,
                instance,
                path,
                &mut sub_errs,
                depth + 1,
            );
            if sub_errs.is_empty() {
                match_count += 1;
            } else {
                branch_errors.extend(sub_errs);
            }
        }
        if match_count == 0 {
            errors.push(format!(
                "{path}: value did not match any branch of oneOf ({})",
                branch_errors.join("; ")
            ));
        } else if match_count > 1 {
            errors.push(format!(
                "{path}: value matched {match_count} branches of oneOf (expected exactly 1)"
            ));
        }
    }

    if let Some(not_schema) = obj.get("not") {
        let mut sub_errs = Vec::new();
        validate_node(
            root_schema,
            not_schema,
            instance,
            path,
            &mut sub_errs,
            depth + 1,
        );
        if sub_errs.is_empty() {
            errors.push(format!("{path}: value matched disallowed 'not' schema"));
        }
    }
}

fn resolve_local_ref<'a>(root_schema: &'a Value, ref_str: &str) -> Option<&'a Value> {
    if ref_str == "#" {
        return Some(root_schema);
    }
    let pointer = ref_str.strip_prefix('#')?;
    root_schema.pointer(pointer)
}

fn instance_matches_type(instance: &Value, expected: &str) -> bool {
    match expected {
        "object" => instance.is_object(),
        "array" => instance.is_array(),
        "string" => instance.is_string(),
        "number" => instance.is_number(),
        "integer" => {
            instance.is_i64()
                || instance.is_u64()
                || instance
                    .as_f64()
                    .is_some_and(|f| f.is_finite() && f.fract() == 0.0)
        }
        "boolean" => instance.is_boolean(),
        "null" => instance.is_null(),
        _ => false,
    }
}

fn json_type_name(val: &Value) -> &'static str {
    match val {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(n) => {
            if n.is_i64() || n.is_u64() {
                "integer"
            } else {
                "number"
            }
        }
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn compact_json(val: &Value) -> String {
    serde_json::to_string(val).unwrap_or_else(|_| format!("{val:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn schema_validation_accepts_valid_payload_and_rejects_malformed() {
        let schema = json!({
            "type": "object",
            "required": ["verdict", "summary"],
            "additionalProperties": false,
            "properties": {
                "verdict": { "type": "string", "enum": ["PASS", "FAIL"] },
                "summary": { "type": "string", "minLength": 3 },
                "bugs": {
                    "type": "array",
                    "items": { "type": "string" }
                }
            }
        });
        validate_schema_definition(&schema).expect("valid schema definition");

        let mut state = PaneResultState::default();
        state
            .set_schema(schema, None)
            .expect("schema set should succeed");

        // 1. Missing required field + invalid enum
        let err = state
            .write_payload(r#"{"verdict": "MAYBE"}"#)
            .expect_err("should reject invalid payload");
        assert_eq!(err.error_code(), "invalid_result");
        assert!(
            err.message()
                .contains("missing required property \"summary\""),
            "unexpected message: {}",
            err.message()
        );
        assert!(
            err.message().contains("allowed enum values"),
            "unexpected message: {}",
            err.message()
        );
        assert!(state.committed.is_none());
        assert!(state.last_invalid_error.is_some());

        // 2. Additional property rejected
        let err2 = state
            .write_payload(r#"{"verdict": "PASS", "summary": "All good", "extra": 1}"#)
            .expect_err("should reject unexpected property");
        assert!(
            err2.message()
                .contains("additional property \"extra\" is not allowed"),
            "unexpected message: {}",
            err2.message()
        );
        assert!(state.committed.is_none());

        // 3. Agent self-corrects with valid payload
        let committed = state
            .write_payload(r#"{"verdict": "PASS", "summary": "All checks green", "bugs": []}"#)
            .expect("valid payload should commit");
        assert_eq!(committed["verdict"], "PASS");
        assert!(state.last_invalid_error.is_none());
        assert!(state.committed.is_some());

        // 4. Write-once enforced on second write
        let second = state
            .write_payload(r#"{"verdict": "FAIL", "summary": "Changed mind"}"#)
            .expect_err("second write must fail");
        assert_eq!(second, WriteResultError::AlreadySet);
        assert_eq!(second.error_code(), "result_already_set");
        assert_eq!(state.committed.as_ref().unwrap()["verdict"], "PASS");
    }

    #[test]
    fn schema_definition_rejects_unknown_type_and_bad_regex() {
        let bad_type = json!({
            "type": "not_a_type"
        });
        let err = validate_schema_definition(&bad_type).expect_err("bad type should fail");
        assert!(err.contains("unknown JSON Schema type"));

        let bad_regex = json!({
            "type": "object",
            "properties": {
                "code": { "type": "string", "pattern": "[unclosed" }
            }
        });
        let err = validate_schema_definition(&bad_regex).expect_err("bad regex should fail");
        assert!(err.contains("invalid regex"));
    }

    #[test]
    fn parse_schema_spec_reads_inline_json_and_file_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let schema_path = dir.path().join("verdict.schema.json");
        std::fs::write(
            &schema_path,
            r#"{"type":"object","required":["ok"],"properties":{"ok":{"type":"boolean"}}}"#,
        )
        .expect("write schema file");

        let from_rel =
            parse_schema_spec("verdict.schema.json", Some(dir.path())).expect("parse relative");
        assert_eq!(from_rel["required"][0], "ok");

        let from_inline =
            parse_schema_spec(r#"{"type":"object","required":["ok"]}"#, None).expect("inline");
        assert_eq!(from_inline["type"], "object");
    }

    #[test]
    fn combinators_and_refs_validate_correctly() {
        let schema = json!({
            "type": "object",
            "required": ["item"],
            "properties": {
                "item": { "$ref": "#/$defs/entry" }
            },
            "$defs": {
                "entry": {
                    "oneOf": [
                        { "type": "string", "minLength": 2 },
                        { "type": "integer", "minimum": 10 }
                    ]
                }
            }
        });
        validate_schema_definition(&schema).expect("valid schema");

        assert!(validate_instance(&schema, &json!({"item": "ok"})).is_ok());
        assert!(validate_instance(&schema, &json!({"item": 15})).is_ok());
        assert!(validate_instance(&schema, &json!({"item": "a"})).is_err());
        assert!(validate_instance(&schema, &json!({"item": 5})).is_err());
    }
}

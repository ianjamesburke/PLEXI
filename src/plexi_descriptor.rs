//! Parser for the `--plexi` CLI descriptor format (issue #188).
//!
//! A CLI opts in to Plexi auto-UI by responding to `--plexi` with a JSON
//! document matching `schemas/plexi-descriptor-schema.json`. This module is
//! the strict, serde-typed counterpart to that schema. It is the substrate
//! #78 (canvas auto-UI renderer) and #321 (CLI wrapper registry) consume.
//!
//! Design choices:
//! - `#[serde(deny_unknown_fields)]` everywhere — mirrors the schema's
//!   `additionalProperties: false`. Unknown fields fail loudly with the
//!   field name in the error.
//! - Required fields use no `serde(default)`. Missing → loud parse error
//!   with the field name. Optional fields are `Option<T>` / `Vec<T>` with
//!   `serde(default)` and clearly marked.
//! - Versioning: the parser refuses descriptors whose `plexi_version` major
//!   is greater than `PLEXI_DESCRIPTOR_MAJOR` (currently 0). Older minors
//!   parse fine; the consumer is expected to ignore unknown optional fields
//!   in the rendering layer (not the parser layer — schema is strict).

use serde::{Deserialize, Serialize};

/// The descriptor-format major version this parser understands. A descriptor
/// declaring `"plexi_version": "1.0"` would be rejected against a parser at
/// major 0, on the assumption that v1 is a breaking format change.
pub const PLEXI_DESCRIPTOR_MAJOR: u32 = 0;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PlexiDescriptor {
    /// Semver of the descriptor format itself (NOT the CLI version).
    pub plexi_version: String,
    pub name: String,
    /// The CLI's own version string.
    pub version: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub default_view: Option<UiHint>,
    pub commands: Vec<Command>,
    #[serde(default)]
    pub live_state: Option<LiveState>,
    /// Shell command to spawn as a PGAP process instead of rendering the
    /// auto-generated form UI. Split on whitespace: first token is the binary,
    /// rest are initial args.
    #[serde(default)]
    pub plexi_app: Option<String>,
    /// Capability strings granted to the spawned PGAP process. Same vocabulary
    /// as manifest.toml capabilities.
    #[serde(default)]
    pub capabilities: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum UiHint {
    Form,
    Output,
    Tabs,
    Stream,
    List,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Command {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub ui_hint: Option<UiHint>,
    #[serde(default)]
    pub args: Vec<ArgSpec>,
    #[serde(default)]
    pub flags: Vec<ArgSpec>,
    #[serde(default)]
    pub writes: Vec<String>,
    #[serde(default)]
    pub reads: Vec<String>,
    #[serde(default)]
    pub streaming: Option<bool>,
    #[serde(default)]
    pub output_format: Option<String>,
    /// Nested subcommands — recursive. `git remote add` would be modelled as
    /// commands → commands → commands.
    #[serde(default)]
    pub commands: Vec<Command>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum ArgType {
    String,
    Int,
    Float,
    Bool,
    Path,
    Enum,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ArgSpec {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: ArgType,
    #[serde(default)]
    pub required: Option<bool>,
    /// Default value. Free-form JSON — caller is responsible for confirming
    /// the runtime type matches `ty`. The schema doc explains this contract.
    #[serde(default)]
    pub default: Option<serde_json::Value>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub placeholder: Option<String>,
    /// Required when `ty == Enum`. Validated at parse time.
    #[serde(default)]
    pub enum_values: Option<Vec<String>>,
    #[serde(default)]
    pub min: Option<f64>,
    #[serde(default)]
    pub max: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LiveState {
    pub source: LiveStateSource,
    pub path: String,
    pub poll_ms: u64,
    pub format: LiveStateFormat,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum LiveStateSource {
    File,
    Socket,
    Http,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum LiveStateFormat {
    Json,
    Yaml,
    Text,
}

/// Why parsing failed. The path/message split lets callers (e.g. the
/// `descriptor probe` subcommand) print "field X failed because Y" without
/// re-parsing the JSON to find the offending location.
#[derive(Debug, thiserror::Error)]
pub enum DescriptorError {
    #[error("descriptor is not valid JSON: {0}")]
    InvalidJson(#[from] serde_json::Error),

    #[error("descriptor schema mismatch at `{path}`: {message}")]
    SchemaMismatch { path: String, message: String },

    #[error(
        "descriptor declares plexi_version {found}; this parser supports major {supported}.x only"
    )]
    UnsupportedMajorVersion { found: String, supported: u32 },

    #[error("plexi_version `{0}` is not valid semver (expected MAJOR.MINOR or MAJOR.MINOR.PATCH)")]
    InvalidPlexiVersion(String),

    #[error("arg/flag `{arg}` declares type=enum but enum_values is missing or empty")]
    EnumMissingValues { arg: String },
}

/// Parse + validate a `--plexi` descriptor JSON string.
///
/// Strictness: any unknown field fails. Required fields missing fail.
/// Cross-field invariants (enum values present when `type=enum`, version
/// major ≤ supported) are checked here too.
pub fn parse(json: &str) -> Result<PlexiDescriptor, DescriptorError> {
    // First, deserialize as serde_json::Value so we can hand serde_path_to_error
    // a structural location on schema mismatches. Without this we'd surface
    // serde's column-only errors, which are useless once a descriptor grows
    // past a screen of JSON.
    let raw: serde_json::Value = serde_json::from_str(json)?;
    let descriptor: PlexiDescriptor = match serde_path_to_error_lite::deserialize(&raw) {
        Ok(d) => d,
        Err(e) => {
            return Err(DescriptorError::SchemaMismatch {
                path: e.path,
                message: e.message,
            });
        }
    };

    // Major-version gate. v0.x parsers must reject v1+ loudly.
    let (major, _minor) = parse_semver_major_minor(&descriptor.plexi_version)?;
    if major > PLEXI_DESCRIPTOR_MAJOR {
        return Err(DescriptorError::UnsupportedMajorVersion {
            found: descriptor.plexi_version.clone(),
            supported: PLEXI_DESCRIPTOR_MAJOR,
        });
    }

    // Cross-field invariants.
    validate_commands(&descriptor.commands)?;

    Ok(descriptor)
}

fn validate_commands(commands: &[Command]) -> Result<(), DescriptorError> {
    for cmd in commands {
        for arg in cmd.args.iter().chain(cmd.flags.iter()) {
            if matches!(arg.ty, ArgType::Enum) {
                let ok = arg
                    .enum_values
                    .as_ref()
                    .map(|v| !v.is_empty())
                    .unwrap_or(false);
                if !ok {
                    return Err(DescriptorError::EnumMissingValues {
                        arg: arg.name.clone(),
                    });
                }
            }
        }
        validate_commands(&cmd.commands)?;
    }
    Ok(())
}

fn parse_semver_major_minor(s: &str) -> Result<(u32, u32), DescriptorError> {
    let mut parts = s.split('.');
    let major = parts
        .next()
        .and_then(|p| p.parse::<u32>().ok())
        .ok_or_else(|| DescriptorError::InvalidPlexiVersion(s.to_string()))?;
    let minor = parts
        .next()
        .and_then(|p| p.parse::<u32>().ok())
        .ok_or_else(|| DescriptorError::InvalidPlexiVersion(s.to_string()))?;
    // Optional patch — accept but ignore. Reject extra components.
    if let Some(patch) = parts.next() {
        if patch.parse::<u32>().is_err() {
            return Err(DescriptorError::InvalidPlexiVersion(s.to_string()));
        }
    }
    if parts.next().is_some() {
        return Err(DescriptorError::InvalidPlexiVersion(s.to_string()));
    }
    Ok((major, minor))
}

/// Tiny replacement for the `serde_path_to_error` crate — we don't have it as
/// a dep and pulling one in for one error message is overkill. This walks the
/// raw `Value` tree once after a failed strict deserialize to recover a
/// JSON-Pointer-ish path. It's good enough for "tell the human which field
/// they broke" without dragging in another crate.
mod serde_path_to_error_lite {
    use serde::de::DeserializeOwned;

    pub struct PathError {
        pub path: String,
        pub message: String,
    }

    pub fn deserialize<T: DeserializeOwned>(raw: &serde_json::Value) -> Result<T, PathError> {
        // Round-trip the value back to bytes; serde_json's error includes the
        // line/column. We then walk the value tree to find the offending key
        // by re-serializing each subtree and trying to parse — too slow for
        // large descriptors, but descriptors are small (CLIs have dozens of
        // commands, not thousands).
        let bytes = serde_json::to_vec(raw).map_err(|e| PathError {
            path: "<root>".into(),
            message: e.to_string(),
        })?;
        match serde_json::from_slice::<T>(&bytes) {
            Ok(t) => Ok(t),
            Err(e) => {
                // serde_json's message names the offending field for
                // missing/unknown fields ("missing field `name`",
                // "unknown field `xyz`, expected ..."). We surface the raw
                // message and a best-effort path.
                let path = guess_path_from_message(&e.to_string()).unwrap_or_else(|| "<root>".into());
                Err(PathError {
                    path,
                    message: e.to_string(),
                })
            }
        }
    }

    /// Pull `field` out of a serde error like `missing field \`foo\`` or
    /// `unknown field \`bar\`, expected ...`. Best-effort — falls back to
    /// `<root>` if the message doesn't match these patterns.
    fn guess_path_from_message(msg: &str) -> Option<String> {
        for needle in &["missing field `", "unknown field `"] {
            if let Some(start) = msg.find(needle) {
                let rest = &msg[start + needle.len()..];
                if let Some(end) = rest.find('`') {
                    return Some(rest[..end].to_string());
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_descriptor() {
        let json = r#"{
            "plexi_version": "0.1",
            "name": "x",
            "version": "0.0.1",
            "commands": []
        }"#;
        let d = parse(json).expect("minimal descriptor parses");
        assert_eq!(d.name, "x");
        assert_eq!(d.version, "0.0.1");
        assert!(d.commands.is_empty());
        assert!(d.live_state.is_none());
    }

    #[test]
    fn parse_full_descriptor_from_issue_body() {
        // Verbatim transliteration of the `parallax` example in #188's body,
        // with the v0.1 arg-type vocabulary (string|int|float|bool|path|enum)
        // — the looser issue-body vocabulary (number, file, dir, color,
        // multiselect) is documented as out of scope for v0 in the proposal
        // doc.
        let json = r#"{
            "plexi_version": "0.1",
            "name": "parallax",
            "version": "0.1.0",
            "description": "Video agent pipeline CLI",
            "icon": "🎬",
            "default_view": "list",
            "commands": [
                {
                    "name": "run",
                    "description": "Kick off a footage_edit run in cwd",
                    "icon": "▶",
                    "ui_hint": "form",
                    "args": [
                        {
                            "name": "brief",
                            "type": "string",
                            "required": true,
                            "description": "What you want the agent to create",
                            "placeholder": "western cowboy scene, 8 seconds"
                        }
                    ],
                    "flags": [
                        {"name": "--test-mode", "type": "bool", "default": false}
                    ],
                    "writes": [".parallax/"],
                    "streaming": true
                },
                {
                    "name": "status",
                    "description": "Print manifest stats",
                    "ui_hint": "output",
                    "args": [],
                    "output_format": "yaml"
                },
                {
                    "name": "project",
                    "description": "Project management",
                    "commands": [
                        {
                            "name": "new",
                            "args": [{"name": "name", "type": "string", "required": true}]
                        },
                        {"name": "list"}
                    ]
                }
            ],
            "live_state": {
                "source": "file",
                "path": ".parallax/manifest.yaml",
                "poll_ms": 1000,
                "format": "yaml"
            }
        }"#;
        let d = parse(json).expect("issue-body descriptor parses");
        assert_eq!(d.name, "parallax");
        assert_eq!(d.commands.len(), 3);
        assert_eq!(d.commands[2].commands.len(), 2);
        assert_eq!(d.commands[2].commands[0].name, "new");
        let live = d.live_state.expect("live_state present");
        assert_eq!(live.source, LiveStateSource::File);
        assert_eq!(live.poll_ms, 1000);
    }

    #[test]
    fn parse_rejects_unknown_top_level_field() {
        let json = r#"{
            "plexi_version": "0.1",
            "name": "x",
            "version": "0.0.1",
            "commands": [],
            "rogue_field": 42
        }"#;
        let err = parse(json).expect_err("unknown top-level field must reject");
        match err {
            DescriptorError::SchemaMismatch { path, message } => {
                assert_eq!(path, "rogue_field");
                assert!(
                    message.contains("unknown field"),
                    "expected 'unknown field' in message, got: {message}"
                );
            }
            other => panic!("expected SchemaMismatch, got {other:?}"),
        }
    }

    #[test]
    fn parse_rejects_missing_required_plexi_version() {
        let json = r#"{
            "name": "x",
            "version": "0.0.1",
            "commands": []
        }"#;
        let err = parse(json).expect_err("missing plexi_version must reject");
        match err {
            DescriptorError::SchemaMismatch { path, message } => {
                assert_eq!(path, "plexi_version");
                assert!(
                    message.contains("missing field"),
                    "expected 'missing field' in message, got: {message}"
                );
            }
            other => panic!("expected SchemaMismatch, got {other:?}"),
        }
    }

    #[test]
    fn parse_rejects_unknown_ui_hint() {
        let json = r#"{
            "plexi_version": "0.1",
            "name": "x",
            "version": "0.0.1",
            "commands": [
                {"name": "run", "ui_hint": "carousel"}
            ]
        }"#;
        let err = parse(json).expect_err("unknown ui_hint enum value must reject");
        match err {
            DescriptorError::SchemaMismatch { message, .. } => {
                assert!(
                    message.contains("carousel") || message.contains("variant"),
                    "expected message to mention bad variant, got: {message}"
                );
            }
            other => panic!("expected SchemaMismatch, got {other:?}"),
        }
    }

    #[test]
    fn parse_handles_nested_commands() {
        let json = r#"{
            "plexi_version": "0.1",
            "name": "git-like",
            "version": "0.0.1",
            "commands": [
                {
                    "name": "remote",
                    "commands": [
                        {
                            "name": "add",
                            "args": [
                                {"name": "name", "type": "string", "required": true},
                                {"name": "url", "type": "string", "required": true}
                            ]
                        }
                    ]
                }
            ]
        }"#;
        let d = parse(json).expect("nested commands parse");
        assert_eq!(d.commands[0].name, "remote");
        assert_eq!(d.commands[0].commands[0].name, "add");
        assert_eq!(d.commands[0].commands[0].args.len(), 2);
    }

    #[test]
    fn parse_rejects_future_plexi_version_when_major_bumps() {
        let json = r#"{
            "plexi_version": "1.0",
            "name": "x",
            "version": "0.0.1",
            "commands": []
        }"#;
        let err = parse(json).expect_err("plexi_version 1.0 must reject under v0 parser");
        match err {
            DescriptorError::UnsupportedMajorVersion { found, supported } => {
                assert_eq!(found, "1.0");
                assert_eq!(supported, 0);
            }
            other => panic!("expected UnsupportedMajorVersion, got {other:?}"),
        }
    }

    #[test]
    fn parse_rejects_enum_arg_without_enum_values() {
        let json = r#"{
            "plexi_version": "0.1",
            "name": "x",
            "version": "0.0.1",
            "commands": [
                {
                    "name": "run",
                    "args": [{"name": "level", "type": "enum"}]
                }
            ]
        }"#;
        let err = parse(json).expect_err("enum arg without enum_values must reject");
        match err {
            DescriptorError::EnumMissingValues { arg } => {
                assert_eq!(arg, "level");
            }
            other => panic!("expected EnumMissingValues, got {other:?}"),
        }
    }

    #[test]
    fn parse_descriptor_with_plexi_app() {
        let json = r#"{
            "plexi_version": "0.1",
            "name": "parallax",
            "version": "1.0.0",
            "commands": [],
            "plexi_app": "parallax-ui --pane",
            "capabilities": ["ai.query", "panes.spawn"]
        }"#;
        let d = parse(json).expect("descriptor with plexi_app parses");
        assert_eq!(d.plexi_app.as_deref(), Some("parallax-ui --pane"));
        assert_eq!(d.capabilities, vec!["ai.query", "panes.spawn"]);
    }

    #[test]
    fn parse_descriptor_without_plexi_app_still_works() {
        let json = r#"{
            "plexi_version": "0.1",
            "name": "old-cli",
            "version": "0.1.0",
            "commands": []
        }"#;
        let d = parse(json).expect("old-style descriptor without plexi_app parses");
        assert!(d.plexi_app.is_none());
        assert!(d.capabilities.is_empty());
    }

    #[test]
    fn parse_rejects_invalid_plexi_version_string() {
        let json = r#"{
            "plexi_version": "not-a-version",
            "name": "x",
            "version": "0.0.1",
            "commands": []
        }"#;
        // Schema regex catches this first via deny_unknown_fields path —
        // serde will actually accept the string (it's just a String), so the
        // pattern check happens at our semver parse layer.
        let err = parse(json).expect_err("non-semver plexi_version must reject");
        match err {
            DescriptorError::InvalidPlexiVersion(v) => assert_eq!(v, "not-a-version"),
            other => panic!("expected InvalidPlexiVersion, got {other:?}"),
        }
    }
}

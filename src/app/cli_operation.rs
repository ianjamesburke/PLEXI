//! Typed one-shot CLI operation definitions declared by a plugin package.
//!
//! Contract: `docs/approved-plugin-cli-v1.md`. A package declares each operation in
//! `[[cli_operation]]`: stable id, fixed executable, a fixed argv template whose
//! dynamic elements are typed slots, an output contract, and the intended
//! authority. This module is pure: it validates definitions, hashes them and
//! builds argv. It never resolves or launches an executable.
//!
//! The authority fields (`fs_read`, `fs_write`, `net_hosts`) use the capability
//! vocabulary the manifest and the 0577 platform seam share. In V1 they describe
//! intent for review and a future sandbox profile. They do not confine the child.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Hard ceilings the host enforces whatever a package declares.
pub const MAX_OUTPUT_BYTES_CEILING: usize = 1024 * 1024;
pub const MAX_TIMEOUT_MS_CEILING: u64 = 60_000;
pub const MAX_STRING_LEN_CEILING: usize = 4096;
pub const MAX_ARGV_ELEMENTS: usize = 32;
pub const MAX_SLOTS: usize = 16;
pub const MAX_OPERATIONS: usize = 32;

/// Whether an operation only reads or changes state. Read and mutate grants are
/// distinct: the mode is part of the grant target and of the binding digest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationMode {
    Read,
    Mutate,
}

/// Declared authority of the native program. Intent only; see module docs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorityDecl {
    pub mode: OperationMode,
    /// `fs.read` scopes: `workspace`, `workspace/<relative>` or an absolute path.
    #[serde(default)]
    pub fs_read: Vec<String>,
    /// `fs.write` scopes, same spelling as `fs_read`.
    #[serde(default)]
    pub fs_write: Vec<String>,
    /// `net.http` hosts. Empty means no network.
    #[serde(default)]
    pub net_hosts: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputFormat {
    Json,
    Text,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputDecl {
    pub format: OutputFormat,
    pub max_bytes: usize,
}

/// One typed dynamic value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum SlotDecl {
    String { max_len: usize },
    Int { min: i64, max: i64 },
    Enum { values: Vec<String> },
    Bool,
}

/// One declared operation, exactly as written in the package manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CliOperationDecl {
    pub id: String,
    /// Fixed program: a bare name searched in fixed host directories, or an
    /// absolute path. Never guest-selected.
    pub executable: String,
    /// Fixed argv after the program. Each element is a literal, a whole-element
    /// `{slot}`, or an option-joined `--name={slot}`.
    pub argv: Vec<String>,
    #[serde(default)]
    pub slots: BTreeMap<String, SlotDecl>,
    pub output: OutputDecl,
    pub timeout_ms: u64,
    pub authority: AuthorityDecl,
}

/// Why a definition cannot be admitted. Names what failed and where to fix it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DefinitionError {
    #[error("cli_operation '{id}': {reason}")]
    Invalid { id: String, reason: String },
}

fn invalid(id: &str, reason: impl Into<String>) -> DefinitionError {
    DefinitionError::Invalid {
        id: id.to_string(),
        reason: reason.into(),
    }
}

/// One element of the argv template after parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Piece<'a> {
    Literal(&'a str),
    /// Whole-element placeholder.
    Slot(&'a str),
    /// `--name=` literal prefix then a placeholder in the same element.
    Joined(&'a str, &'a str),
}

fn parse_piece<'a>(id: &str, element: &'a str) -> Result<Piece<'a>, DefinitionError> {
    let Some(open) = element.find('{') else {
        if element.contains('}') {
            return Err(invalid(
                id,
                format!("argv element '{element}' has a stray '}}'"),
            ));
        }
        return Ok(Piece::Literal(element));
    };
    let Some(inner) = element[open + 1..].strip_suffix('}') else {
        return Err(invalid(
            id,
            format!("argv element '{element}' must end with its '{{slot}}' placeholder"),
        ));
    };
    if inner.contains('{') || inner.contains('}') || !is_ident(inner) {
        return Err(invalid(
            id,
            format!("argv element '{element}' has a malformed placeholder"),
        ));
    }
    let prefix = &element[..open];
    if prefix.is_empty() {
        return Ok(Piece::Slot(inner));
    }
    let is_joined_option = prefix.starts_with("--")
        && prefix.len() > 3
        && prefix.ends_with('=')
        && !prefix[2..prefix.len() - 1].contains('=');
    if !is_joined_option {
        return Err(invalid(
            id,
            format!(
                "argv element '{element}' embeds a placeholder in text; use a whole-element '{{slot}}' or an option-joined '--name={{slot}}'"
            ),
        ));
    }
    Ok(Piece::Joined(prefix, inner))
}

fn is_ident(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 48
        && text
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        && !text.starts_with(|c: char| c.is_ascii_digit())
}

fn is_valid_operation_id(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 64
        && text
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
        && text.starts_with(|c: char| c.is_ascii_lowercase())
}

fn valid_executable(exe: &str) -> bool {
    if exe.is_empty() || exe.contains('\0') || exe.chars().any(char::is_whitespace) {
        return false;
    }
    if exe.starts_with('/') {
        return !exe.split('/').any(|part| part == "..");
    }
    exe.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        && !exe.starts_with('-')
}

fn valid_scope(scope: &str) -> bool {
    if scope.is_empty() || scope.contains('\0') || scope.split('/').any(|part| part == "..") {
        return false;
    }
    scope == "workspace" || scope.starts_with("workspace/") || scope.starts_with('/')
}

impl CliOperationDecl {
    /// Validate the whole definition without touching the filesystem. Rejects a
    /// definition whose CLI semantics cannot guarantee the slot-to-argv mapping.
    pub fn validate(&self) -> Result<(), DefinitionError> {
        let id = self.id.as_str();
        if !is_valid_operation_id(id) {
            return Err(invalid(
                id,
                "id must be 1-64 chars of a-z 0-9 . _ - starting with a letter",
            ));
        }
        if !valid_executable(&self.executable) {
            return Err(invalid(
                id,
                format!(
                    "executable '{}' must be a bare name or an absolute path without '..'",
                    self.executable
                ),
            ));
        }
        if self.argv.len() > MAX_ARGV_ELEMENTS {
            return Err(invalid(
                id,
                format!("argv has more than {MAX_ARGV_ELEMENTS} elements"),
            ));
        }
        if self.slots.len() > MAX_SLOTS {
            return Err(invalid(id, format!("more than {MAX_SLOTS} slots")));
        }
        if self.output.max_bytes == 0 || self.output.max_bytes > MAX_OUTPUT_BYTES_CEILING {
            return Err(invalid(
                id,
                format!("output.max_bytes must be 1..={MAX_OUTPUT_BYTES_CEILING}"),
            ));
        }
        if self.timeout_ms == 0 || self.timeout_ms > MAX_TIMEOUT_MS_CEILING {
            return Err(invalid(
                id,
                format!("timeout_ms must be 1..={MAX_TIMEOUT_MS_CEILING}"),
            ));
        }
        for scope in self
            .authority
            .fs_read
            .iter()
            .chain(&self.authority.fs_write)
        {
            if !valid_scope(scope) {
                return Err(invalid(
                    id,
                    format!("authority scope '{scope}' must be 'workspace', 'workspace/<relative>' or an absolute path without '..'"),
                ));
            }
        }
        for host in &self.authority.net_hosts {
            if host.is_empty() || host.chars().any(|c| c.is_whitespace() || c == '/') {
                return Err(invalid(
                    id,
                    format!("authority net host '{host}' is not a hostname pattern"),
                ));
            }
        }
        for (name, slot) in &self.slots {
            if !is_ident(name) {
                return Err(invalid(id, format!("slot name '{name}' must be a-z 0-9 _")));
            }
            match slot {
                SlotDecl::String { max_len }
                    if *max_len == 0 || *max_len > MAX_STRING_LEN_CEILING =>
                {
                    return Err(invalid(
                        id,
                        format!("slot '{name}' max_len must be 1..={MAX_STRING_LEN_CEILING}"),
                    ));
                }
                SlotDecl::Int { min, max } if min > max => {
                    return Err(invalid(
                        id,
                        format!("slot '{name}' has min greater than max"),
                    ));
                }
                SlotDecl::Enum { values } => {
                    if values.is_empty() || values.len() > 64 {
                        return Err(invalid(
                            id,
                            format!("slot '{name}' enum needs 1..=64 values"),
                        ));
                    }
                    let unique: BTreeSet<_> = values.iter().collect();
                    if unique.len() != values.len() {
                        return Err(invalid(
                            id,
                            format!("slot '{name}' enum has duplicate values"),
                        ));
                    }
                    if values.iter().any(|v| {
                        v.is_empty() || v.contains('\0') || v.chars().any(char::is_control)
                    }) {
                        return Err(invalid(id, format!("slot '{name}' enum values must be non-empty without control characters")));
                    }
                }
                _ => {}
            }
        }
        let mut used = BTreeSet::new();
        let mut delimited = false;
        for element in &self.argv {
            if element.contains('\0') {
                return Err(invalid(id, "argv contains a NUL byte"));
            }
            match parse_piece(id, element)? {
                Piece::Literal(text) => {
                    if text == "--" && !delimited {
                        delimited = true;
                    }
                }
                Piece::Slot(name) | Piece::Joined(_, name) => {
                    let slot = self.slots.get(name).ok_or_else(|| {
                        invalid(
                            id,
                            format!("placeholder '{{{name}}}' has no slot declaration"),
                        )
                    })?;
                    if !used.insert(name) {
                        return Err(invalid(id, format!("slot '{name}' is used more than once")));
                    }
                    let joined = matches!(parse_piece(id, element)?, Piece::Joined(..));
                    if !joined && !delimited {
                        // A bare value before any '--' could be read as an option.
                        // Only types that can never begin with '-' may sit here.
                        let safe = match slot {
                            SlotDecl::String { .. } => true, // flag-like text is rejected at validation time
                            SlotDecl::Int { min, .. } => *min >= 0,
                            SlotDecl::Enum { values } => values.iter().all(|v| !v.starts_with('-')),
                            SlotDecl::Bool => false,
                        };
                        if !safe {
                            return Err(invalid(
                                id,
                                format!("slot '{name}' can render a leading '-' in a bare position; put it after a '--' element or join it as '--name={{{name}}}'"),
                            ));
                        }
                    }
                }
            }
        }
        for name in self.slots.keys() {
            if !used.contains(name.as_str()) {
                return Err(invalid(
                    id,
                    format!("slot '{name}' is declared but never used in argv"),
                ));
            }
        }
        Ok(())
    }

    /// SHA-256 of the canonical JSON of the whole definition: executable name,
    /// argv template, slot types and bounds, output contract, timeout and
    /// authority. Struct field order is fixed and slots are a `BTreeMap`, so the
    /// bytes are deterministic. Any change yields a new hash and so a new grant.
    pub fn definition_hash(&self) -> String {
        let bytes = serde_json::to_vec(self).unwrap_or_default();
        hex(&Sha256::digest(&bytes))
    }

    /// Build the argument vector (without the program) from typed values.
    ///
    /// Re-validates every value against its slot, so calling it again right
    /// before launch re-checks bounds. No shell, no environment, no cwd.
    pub fn build_argv(
        &self,
        values: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<Vec<OsString>, ArgError> {
        for key in values.keys() {
            if !self.slots.contains_key(key) {
                return Err(ArgError::UnknownValue(key.clone()));
            }
        }
        let mut rendered: BTreeMap<&str, String> = BTreeMap::new();
        for (name, slot) in &self.slots {
            let value = values
                .get(name)
                .ok_or_else(|| ArgError::MissingValue(name.clone()))?;
            rendered.insert(name, render_value(name, slot, value)?);
        }
        let mut out = Vec::with_capacity(self.argv.len());
        let mut delimited = false;
        for element in &self.argv {
            let piece =
                parse_piece(&self.id, element).map_err(|e| ArgError::Definition(e.to_string()))?;
            match piece {
                Piece::Literal(text) => {
                    if text == "--" {
                        delimited = true;
                    }
                    out.push(OsString::from(text));
                }
                Piece::Slot(name) => {
                    let text = &rendered[name];
                    if !delimited && text.starts_with('-') {
                        return Err(ArgError::FlagLike(name.to_string()));
                    }
                    out.push(OsString::from(text));
                }
                Piece::Joined(prefix, name) => {
                    out.push(OsString::from(format!("{prefix}{}", rendered[name])));
                }
            }
        }
        Ok(out)
    }
}

/// A value that cannot occupy its declared slot.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ArgError {
    #[error("unknown value '{0}'")]
    UnknownValue(String),
    #[error("missing value '{0}'")]
    MissingValue(String),
    #[error("value '{0}' has the wrong type or is out of bounds")]
    Invalid(String),
    #[error("value '{0}' looks like a command-line option")]
    FlagLike(String),
    #[error("{0}")]
    Definition(String),
}

fn render_value(
    name: &str,
    slot: &SlotDecl,
    value: &serde_json::Value,
) -> Result<String, ArgError> {
    let bad = || ArgError::Invalid(name.to_string());
    match slot {
        SlotDecl::String { max_len } => {
            let text = value.as_str().ok_or_else(bad)?;
            if text.is_empty()
                || text.chars().count() > *max_len
                || text.chars().any(char::is_control)
            {
                return Err(bad());
            }
            Ok(text.to_string())
        }
        SlotDecl::Int { min, max } => {
            let number = value.as_i64().ok_or_else(bad)?;
            if number < *min || number > *max {
                return Err(bad());
            }
            Ok(number.to_string())
        }
        SlotDecl::Enum { values } => {
            let text = value.as_str().ok_or_else(bad)?;
            if !values.iter().any(|allowed| allowed == text) {
                return Err(bad());
            }
            Ok(text.to_string())
        }
        SlotDecl::Bool => Ok(value.as_bool().ok_or_else(bad)?.to_string()),
    }
}

/// Parse and validate every `[[cli_operation]]` of a manifest. Ids are unique.
pub fn validate_all(decls: &[CliOperationDecl]) -> Result<(), DefinitionError> {
    if decls.len() > MAX_OPERATIONS {
        return Err(invalid(
            "",
            format!("more than {MAX_OPERATIONS} operations"),
        ));
    }
    let mut seen = BTreeSet::new();
    for decl in decls {
        decl.validate()?;
        if !seen.insert(decl.id.as_str()) {
            return Err(invalid(&decl.id, "duplicate operation id"));
        }
    }
    Ok(())
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn decl(toml_text: &str) -> CliOperationDecl {
        toml::from_str(toml_text).expect("fixture definition parses")
    }

    const SET: &str = r#"
id = "task.set"
executable = "stint"
argv = ["set", "--priority={priority}", "--", "{task}"]
timeout_ms = 5000
[output]
format = "json"
max_bytes = 4096
[authority]
mode = "mutate"
fs_write = ["workspace"]
[slots.priority]
type = "enum"
values = ["p0", "p1", "p2"]
[slots.task]
type = "string"
max_len = 64
"#;

    fn values(pairs: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
        pairs.as_object().unwrap().clone()
    }

    #[test]
    fn valid_definition_builds_argv_without_shell() {
        let op = decl(SET);
        op.validate().unwrap();
        let argv = op
            .build_argv(&values(json!({"priority": "p1", "task": "0042; rm -rf /"})))
            .unwrap();
        assert_eq!(
            argv,
            ["set", "--priority=p1", "--", "0042; rm -rf /"]
                .map(OsString::from)
                .to_vec()
        );
    }

    #[test]
    fn flag_like_value_in_delimited_position_is_a_plain_operand() {
        let op = decl(SET);
        let argv = op
            .build_argv(&values(
                json!({"priority": "p0", "task": "--delete-everything"}),
            ))
            .unwrap();
        assert_eq!(argv.last().unwrap(), "--delete-everything");
    }

    #[test]
    fn flag_like_value_in_bare_position_is_rejected() {
        let op = decl(
            r#"
id = "task.show"
executable = "stint"
argv = ["show", "{task}"]
timeout_ms = 1000
[output]
format = "json"
max_bytes = 100
[authority]
mode = "read"
[slots.task]
type = "string"
max_len = 8
"#,
        );
        op.validate().unwrap();
        assert_eq!(
            op.build_argv(&values(json!({"task": "-rf"}))),
            Err(ArgError::FlagLike("task".into()))
        );
        assert!(op.build_argv(&values(json!({"task": "0042"}))).is_ok());
    }

    #[test]
    fn malformed_values_are_rejected() {
        let op = decl(SET);
        for bad in [
            json!({"priority": "p9", "task": "1"}),
            json!({"priority": 1, "task": "1"}),
            json!({"priority": "p1", "task": ""}),
            json!({"priority": "p1", "task": "a\nb"}),
            json!({"priority": "p1", "task": "x".repeat(65)}),
            json!({"priority": "p1"}),
            json!({"priority": "p1", "task": "1", "extra": "x"}),
            json!({"priority": "p1", "task": "1", "executable": "/bin/sh"}),
        ] {
            assert!(op.build_argv(&values(bad.clone())).is_err(), "{bad}");
        }
    }

    #[test]
    fn definition_hash_covers_every_field() {
        let base = decl(SET);
        let hash = base.definition_hash();
        assert_eq!(hash, decl(SET).definition_hash());
        let mut variants = Vec::new();
        for edit in [
            |d: &mut CliOperationDecl| d.executable = "other".into(),
            |d: &mut CliOperationDecl| d.argv[0] = "put".into(),
            |d: &mut CliOperationDecl| d.timeout_ms += 1,
            |d: &mut CliOperationDecl| d.output.max_bytes += 1,
            |d: &mut CliOperationDecl| d.output.format = OutputFormat::Text,
            |d: &mut CliOperationDecl| d.authority.mode = OperationMode::Read,
            |d: &mut CliOperationDecl| d.authority.fs_write.clear(),
            |d: &mut CliOperationDecl| d.authority.net_hosts.push("example.com".into()),
            |d: &mut CliOperationDecl| {
                d.slots
                    .insert("task".into(), SlotDecl::String { max_len: 65 });
            },
            |d: &mut CliOperationDecl| {
                d.slots.insert(
                    "priority".into(),
                    SlotDecl::Enum {
                        values: vec!["p0".into(), "p1".into()],
                    },
                );
            },
        ] {
            let mut changed = base.clone();
            edit(&mut changed);
            variants.push(changed.definition_hash());
        }
        let unique: BTreeSet<_> = variants.iter().chain([&hash]).collect();
        assert_eq!(
            unique.len(),
            variants.len() + 1,
            "every edit must change the hash"
        );
    }

    fn rejects(edit: impl FnOnce(&mut CliOperationDecl), needle: &str) {
        let mut op = decl(SET);
        edit(&mut op);
        let error = op
            .validate()
            .expect_err("definition must be rejected")
            .to_string();
        assert!(error.contains(needle), "{error} should mention {needle}");
    }

    #[test]
    fn definitions_with_ambiguous_cli_semantics_are_rejected() {
        rejects(
            |d| d.argv[1] = "--priority {priority}".into(),
            "embeds a placeholder",
        );
        rejects(|d| d.argv[1] = "x{priority}".into(), "embeds a placeholder");
        rejects(|d| d.argv[1] = "{priority}x".into(), "must end with");
        rejects(
            |d| d.argv[1] = "--priority={nope}".into(),
            "no slot declaration",
        );
        rejects(|d| d.argv.push("{priority}".into()), "more than once");
        rejects(|d| d.argv.truncate(2), "never used");
        rejects(|d| d.executable = "../bin/stint".into(), "executable");
        rejects(|d| d.executable = "sh -c".into(), "executable");
        rejects(|d| d.executable = "-evil".into(), "executable");
        rejects(|d| d.id = "Bad Id".into(), "id must be");
        rejects(
            |d| d.output.max_bytes = MAX_OUTPUT_BYTES_CEILING + 1,
            "max_bytes",
        );
        rejects(|d| d.timeout_ms = 0, "timeout_ms");
        rejects(
            |d| d.authority.fs_read = vec!["../etc".into()],
            "authority scope",
        );
        rejects(
            |d| d.authority.fs_read = vec!["relative/path".into()],
            "authority scope",
        );
    }

    #[test]
    fn negative_int_or_dash_enum_in_bare_position_is_rejected_at_definition_time() {
        let mut op = decl(SET);
        op.argv = vec!["{n}".into(), "{priority}".into(), "{task}".into()];
        op.slots
            .insert("n".into(), SlotDecl::Int { min: -5, max: 5 });
        op.slots.insert(
            "priority".into(),
            SlotDecl::Enum {
                values: vec!["ok".into()],
            },
        );
        let error = op.validate().unwrap_err().to_string();
        assert!(error.contains("leading '-'"), "{error}");
        op.slots
            .insert("n".into(), SlotDecl::Int { min: 0, max: 5 });
        op.slots.insert(
            "priority".into(),
            SlotDecl::Enum {
                values: vec!["-x".into()],
            },
        );
        let error = op.validate().unwrap_err().to_string();
        assert!(error.contains("leading '-'"), "{error}");
    }

    #[test]
    fn unknown_manifest_keys_are_refused() {
        let text = format!("{SET}\nenv = [\"A=1\"]\n");
        assert!(toml::from_str::<CliOperationDecl>(&text).is_err());
        let text = SET.replace("[authority]", "[authority]\nsandbox = true");
        assert!(toml::from_str::<CliOperationDecl>(&text).is_err());
    }

    #[test]
    fn duplicate_operation_ids_are_rejected() {
        let a = decl(SET);
        assert!(validate_all(&[a.clone(), a]).is_err());
    }
}

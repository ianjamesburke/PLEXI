//! The plugin's own exception, as the SDK runtime reports it.
//!
//! When plugin code raises, `plexi_sdk._v3_runtime` sends one `fatal_error`
//! message and exits. Every surface that reports a dead guest (`plugin check`,
//! `view render`, the live pane) leads with the headline built here, so the
//! author reads their own file, line and exception first. The host never
//! parses a traceback: the SDK supplies each field.

use std::path::{Component, Path};

use serde::Deserialize;
use serde_json::Value;

/// The entry functions the runtime can be serving when it reports, plus
/// `runtime` for the SDK's own code between plugin calls.
const PHASES: &[&str] = &["import", "init", "update", "view", "runtime"];
const MAX_FILE_CHARS: usize = 256;
const MAX_EXCEPTION_CHARS: usize = 128;
const MAX_MESSAGE_CHARS: usize = 2048;
/// The end of a traceback names the exception, so an oversized one keeps its tail.
const MAX_TRACEBACK_BYTES: usize = 64 * 1024;
/// One host log record is written per traceback line, so lines are capped too.
const MAX_TRACEBACK_LINES: usize = 200;
const TRUNCATED: &str = " [truncated]";
const TRACEBACK_TRUNCATED: &str = "[traceback truncated: earlier frames dropped]";
/// Shown in place of a reported path that is not inside the plugin folder.
const OUTSIDE_PLUGIN: &str = "<path outside the plugin>";

/// The wire shape, exactly as the guest sent it. Nothing here is trusted.
#[derive(Deserialize)]
struct WireFatalError {
    phase: String,
    file: Option<String>,
    line: Option<u32>,
    exception: String,
    message: String,
    traceback: String,
}

/// A plugin's fatal error, made safe to print. The guest writes every field,
/// and the text reaches a terminal, the host log and the pane, so `decode`
/// bounds each field and escapes control characters: a plugin cannot forge a
/// host line, recolor the terminal, point at a file outside its own folder or
/// flood the log.
#[derive(Debug)]
pub(super) struct GuestFatalError {
    phase: &'static str,
    /// Plugin-relative path. Absent when no plugin code was on the stack and
    /// the phase has no entry function to point at.
    file: Option<String>,
    line: Option<u32>,
    exception: String,
    message: String,
    traceback: String,
}

impl GuestFatalError {
    pub(super) fn decode(message: &Value) -> Result<Self, String> {
        let wire = WireFatalError::deserialize(message).map_err(|error| {
            format!(
                "malformed fatal_error message from the SDK runtime ({error}); \
                 the host and the packaged plexi_sdk disagree on its fields"
            )
        })?;
        let phase = PHASES
            .iter()
            .copied()
            .find(|known| *known == wire.phase)
            .ok_or_else(|| {
                format!(
                    "malformed fatal_error message from the SDK runtime (phase is not one of \
                     {PHASES:?}); the host and the packaged plexi_sdk disagree on its fields"
                )
            })?;
        let reported_file = wire.file.is_some();
        let file = wire.file.filter(|file| is_plugin_relative(file));
        Ok(Self {
            phase,
            // A line number means nothing without the file it counts into.
            line: file.as_ref().and(wire.line),
            file: match file {
                Some(file) => Some(escape_untrusted(&file, false)),
                None if reported_file => Some(OUTSIDE_PLUGIN.to_string()),
                None => None,
            },
            exception: escape_untrusted(&keep_head(&wire.exception, MAX_EXCEPTION_CHARS), false),
            message: escape_untrusted(&keep_head(&wire.message, MAX_MESSAGE_CHARS), false),
            traceback: escape_untrusted(
                keep_tail(&wire.traceback, MAX_TRACEBACK_BYTES).trim_end(),
                true,
            ),
        })
    }

    /// `plugin error in main.py:54 (view): RuntimeError: ...`
    pub(super) fn headline(&self) -> String {
        let location = match (&self.file, self.line) {
            (Some(file), Some(line)) => format!(" in {file}:{line}"),
            (Some(file), None) => format!(" in {file}"),
            (None, _) => String::new(),
        };
        let detail = if self.message.is_empty() {
            self.exception.clone()
        } else {
            format!("{}: {}", self.exception, self.message)
        };
        format!("plugin error{location} ({}): {detail}", self.phase)
    }

    pub(super) fn traceback(&self) -> &str {
        &self.traceback
    }

    /// The headline, then the traceback for whoever needs the full stack.
    pub(super) fn report(&self) -> String {
        format!("{}\n{}", self.headline(), self.traceback())
    }
}

/// A path that stays inside the plugin folder: relative, bounded, and with no
/// component that climbs out of it.
fn is_plugin_relative(file: &str) -> bool {
    !file.is_empty()
        && file.chars().count() <= MAX_FILE_CHARS
        && !file.starts_with(['/', '\\'])
        && Path::new(file)
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

/// Characters that move the cursor, start a new line, recolor a terminal or
/// reorder the text around them.
fn is_unsafe_to_print(c: char) -> bool {
    c.is_control()
        || matches!(c, '\u{2028}' | '\u{2029}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// Replace every unprintable character with its visible escape (`\n`,
/// `\u{1b}`), so the text occupies exactly the lines the host gives it.
fn escape_untrusted(text: &str, keep_newlines: bool) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if is_unsafe_to_print(c) && !(keep_newlines && c == '\n') {
            out.extend(c.escape_default());
        } else {
            out.push(c);
        }
    }
    out
}

fn keep_head(text: &str, max_chars: usize) -> String {
    match text.char_indices().nth(max_chars) {
        Some((cut, _)) => format!("{}{TRUNCATED}", &text[..cut]),
        None => text.to_string(),
    }
}

/// The end of `text`: at most `max_bytes`, cut forward to a whole line, and at
/// most `MAX_TRACEBACK_LINES` lines. A cut is marked on its own first line.
fn keep_tail(text: &str, max_bytes: usize) -> std::borrow::Cow<'_, str> {
    let text = text.trim_end();
    let mut tail = text;
    if tail.len() > max_bytes {
        let mut cut = tail.len() - max_bytes;
        while !tail.is_char_boundary(cut) {
            cut += 1;
        }
        tail = &tail[cut..];
        tail = tail.find('\n').map_or(tail, |newline| &tail[newline + 1..]);
    }
    if let Some((cut, _)) = tail.rmatch_indices('\n').nth(MAX_TRACEBACK_LINES - 1) {
        tail = &tail[cut + 1..];
    }
    if tail.len() == text.len() {
        text.into()
    } else {
        format!("{TRACEBACK_TRUNCATED}\n{tail}").into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fatal(file: Value, line: Value, message: &str) -> Value {
        json!({
            "type": "fatal_error",
            "phase": "view",
            "file": file,
            "line": line,
            "exception": "RuntimeError",
            "message": message,
            "traceback": "Traceback (most recent call last):\n  File \"main.py\", line 54, in view\nRuntimeError: nope\n",
        })
    }

    #[test]
    fn headline_names_file_line_phase_type_and_message() {
        let error = GuestFatalError::decode(&fatal(json!("main.py"), json!(54), "nope"))
            .expect("well-formed message");
        assert_eq!(
            error.headline(),
            "plugin error in main.py:54 (view): RuntimeError: nope"
        );
        let report = error.report();
        assert_eq!(report.lines().next(), Some(error.headline().as_str()));
        assert!(report.ends_with("RuntimeError: nope"), "{report}");
    }

    #[test]
    fn headline_without_a_plugin_location_or_message_stays_readable() {
        let error = GuestFatalError::decode(&fatal(Value::Null, Value::Null, ""))
            .expect("well-formed message");
        assert_eq!(error.headline(), "plugin error (view): RuntimeError");
    }

    #[test]
    fn a_message_missing_a_field_is_refused_by_name() {
        let error = GuestFatalError::decode(&json!({"type": "fatal_error", "phase": "view"}))
            .expect_err("missing fields must not decode");
        assert!(error.contains("malformed fatal_error"), "{error}");
        assert!(error.contains("exception"), "{error}");
    }

    fn hostile(overrides: Value) -> Value {
        let mut message = fatal(json!("main.py"), json!(54), "nope");
        for (key, value) in overrides.as_object().expect("object") {
            message[key] = value.clone();
        }
        message
    }

    #[test]
    fn a_message_cannot_forge_a_host_line_or_recolor_the_terminal() {
        let error = GuestFatalError::decode(&hostile(json!({
            "message": "boom\n✓ render 320x240 — 12 node(s)\r\u{1b}[32mok\u{1b}[0m\u{202e}",
            "exception": "Runtime\nError",
        })))
        .expect("well-formed message");
        let headline = error.headline();
        assert_eq!(headline.lines().count(), 1, "{headline:?}");
        assert!(!headline.chars().any(is_unsafe_to_print), "{headline:?}");
        assert!(headline.contains("boom\\n✓ render"), "{headline:?}");
        assert!(headline.contains("\\u{1b}[32mok"), "{headline:?}");
        assert!(headline.contains("Runtime\\nError"), "{headline:?}");
    }

    #[test]
    fn the_traceback_keeps_its_lines_but_no_other_control_characters() {
        let error = GuestFatalError::decode(&hostile(json!({
            "traceback": "line one\r\u{1b}[2J\nline two\u{7}\n",
        })))
        .expect("well-formed message");
        assert_eq!(error.traceback(), "line one\\r\\u{1b}[2J\nline two\\u{7}");
    }

    #[test]
    fn oversized_fields_are_cut_and_marked() {
        let error = GuestFatalError::decode(&hostile(json!({
            "exception": "E".repeat(10_000),
            "message": "m".repeat(10 * 1024 * 1024),
            "traceback": format!("{}last line", "frame\n".repeat(100_000)),
        })))
        .expect("well-formed message");
        assert!(
            error.headline().len() < MAX_EXCEPTION_CHARS + MAX_MESSAGE_CHARS + 200,
            "headline is {} bytes",
            error.headline().len()
        );
        assert_eq!(error.headline().matches(TRUNCATED).count(), 2);
        assert!(error.traceback().len() <= MAX_TRACEBACK_BYTES + 64);
        assert!(error
            .traceback()
            .starts_with("[traceback truncated: earlier frames dropped]\nframe\n"));
        assert!(error.traceback().ends_with("last line"));
    }

    #[test]
    fn a_file_outside_the_plugin_folder_is_replaced_and_loses_its_line() {
        for file in [
            "../other-plugin/main.py",
            "plugin/../../main.py",
            "/etc/passwd",
            "\\\\server\\share",
            "",
        ] {
            let error = GuestFatalError::decode(&hostile(json!({ "file": file })))
                .expect("well-formed message");
            assert_eq!(
                error.headline(),
                "plugin error in <path outside the plugin> (view): RuntimeError: nope",
                "file {file:?} must not be reported"
            );
        }
        let nested = GuestFatalError::decode(&hostile(json!({ "file": "plugin/ui.py" })))
            .expect("well-formed message");
        assert_eq!(
            nested.headline(),
            "plugin error in plugin/ui.py:54 (view): RuntimeError: nope"
        );
    }

    #[test]
    fn a_traceback_of_many_short_lines_is_capped_by_line_count() {
        let error = GuestFatalError::decode(&hostile(json!({
            "traceback": format!("{}RuntimeError: last line", "x\n".repeat(20_000)),
        })))
        .expect("well-formed message");
        let lines: Vec<&str> = error.traceback().lines().collect();
        assert_eq!(
            lines.len(),
            MAX_TRACEBACK_LINES + 1,
            "one marker line, then the tail"
        );
        assert_eq!(lines[0], "[traceback truncated: earlier frames dropped]");
        assert_eq!(lines.last(), Some(&"RuntimeError: last line"));
    }

    #[test]
    fn a_malformed_report_cannot_carry_an_oversized_value_into_the_error() {
        for overrides in [
            json!({ "line": "L".repeat(10 * 1024 * 1024) }),
            json!({ "exception": 5, "message": "m".repeat(10 * 1024 * 1024) }),
            json!({ "line": format!("x\n✓ render 320x240{}", "y".repeat(4096)) }),
        ] {
            let error = GuestFatalError::decode(&hostile(overrides))
                .expect_err("a wrongly typed field must not decode");
            assert!(error.contains("malformed fatal_error"), "{error}");
            assert!(error.len() < 512, "error is {} bytes", error.len());
            assert_eq!(error.lines().count(), 1, "{error:?}");
        }
    }

    #[test]
    fn weak_bidi_marks_are_escaped() {
        let error = GuestFatalError::decode(&hostile(json!({
            "message": "a\u{200e}b\u{200f}c\u{061c}d",
        })))
        .expect("well-formed message");
        assert!(
            error.headline().ends_with("a\\u{200e}b\\u{200f}c\\u{61c}d"),
            "{:?}",
            error.headline()
        );
    }

    #[test]
    fn an_unknown_phase_is_malformed_and_not_echoed() {
        let error = GuestFatalError::decode(&hostile(json!({ "phase": "view\n✓ all good" })))
            .expect_err("an unknown phase must not decode");
        assert!(error.contains("phase is not one of"), "{error}");
        assert!(!error.contains("all good"), "{error}");
    }
}

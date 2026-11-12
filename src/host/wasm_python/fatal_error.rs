//! The plugin's own exception, as the SDK runtime reports it.
//!
//! When plugin code raises, `plexi_sdk._v3_runtime` sends one `fatal_error`
//! message and exits. Every surface that reports a dead guest (`plugin check`,
//! `view render`, the live pane) leads with the headline built here, so the
//! author reads their own file, line and exception first. The host never
//! parses a traceback: the SDK supplies each field.

use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
pub(super) struct GuestFatalError {
    /// The entry function the runtime was serving: `import`, `init`, `update`,
    /// `view`, or `runtime` for the SDK's own code between plugin calls.
    phase: String,
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
        Self::deserialize(message).map_err(|error| {
            format!(
                "malformed fatal_error message from the SDK runtime ({error}); \
                 the host and the packaged plexi_sdk disagree on its fields"
            )
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
        self.traceback.trim_end()
    }

    /// The headline, then the traceback for whoever needs the full stack.
    pub(super) fn report(&self) -> String {
        format!("{}\n{}", self.headline(), self.traceback())
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
}

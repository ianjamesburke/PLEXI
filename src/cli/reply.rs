//! One reading of a host reply for every CLI waiter (stint 1285, Codex 3290
//! r4). A reply is the payload the command asked for, or the host's refusal
//! with its reason. A refusal is never printed as a result and never exits 0.
//! Transport failures stay with the request helpers that produced them.

/// What the host answered.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum HostReply {
    /// The result the command asked for, as the host wrote it.
    Payload(String),
    /// The host refused: its reason, its error code when it named one, and
    /// the whole body for `--json` readers.
    Refused {
        reason: String,
        code: Option<String>,
        body: String,
    },
}

impl HostReply {
    /// A JSON object reply that names an `error`, an `error_code`, or
    /// `"ok": false` is a refusal; anything else (including a plain-text
    /// choice key) is the payload.
    pub(crate) fn decode(content: &str) -> Self {
        let Ok(serde_json::Value::Object(object)) =
            serde_json::from_str::<serde_json::Value>(content.trim())
        else {
            return Self::Payload(content.to_string());
        };
        let error = object.get("error").and_then(|value| value.as_str());
        let code = object.get("error_code").and_then(|value| value.as_str());
        let refused = error.is_some()
            || code.is_some()
            || object.get("ok").and_then(|value| value.as_bool()) == Some(false);
        if !refused {
            return Self::Payload(content.to_string());
        }
        Self::Refused {
            reason: error
                .or(code)
                .unwrap_or("the host refused the request")
                .to_string(),
            code: code.map(str::to_string),
            body: content.trim().to_string(),
        }
    }

    /// Report a refusal the one way every waiter reports it: the reason on
    /// stderr, the body on stdout for `--json` readers, exit 1. A payload is
    /// handed back to the caller to print.
    pub(crate) fn payload_or_report(self) -> Result<String, i32> {
        match self {
            Self::Payload(content) => Ok(content),
            Self::Refused { reason, code, body } => {
                log::info!("cli: host refused code={code:?}");
                println!("{body}");
                eprintln!("error: {reason}");
                Err(1)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::HostReply;

    #[test]
    fn a_refusal_is_never_a_payload() {
        for body in [
            r#"{"ok":false,"error_code":"scope_unresolvable","error":"no context"}"#,
            r#"{"error":"pane 4 not found"}"#,
            r#"{"error_code":"permission_denied"}"#,
            r#"{"ok":false}"#,
        ] {
            assert!(matches!(HostReply::decode(body), HostReply::Refused { .. }), "{body}");
            assert_eq!(HostReply::decode(body).payload_or_report(), Err(1), "{body}");
        }
        for body in ["yes", "[1,2]", r#"{"ok":true,"pane_id":3}"#, r#"{"panes":[]}"#] {
            assert_eq!(HostReply::decode(body), HostReply::Payload(body.to_string()), "{body}");
        }
    }
}

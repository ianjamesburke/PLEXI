//! Classify command-socket replies before a waiter reads its payload.

use crate::rpc::{PollError, SlotReply};
use serde_json::Value;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum OutputMode {
    #[default]
    Text,
    Json,
}

impl OutputMode {
    pub(crate) fn is_json(self) -> bool {
        self == Self::Json
    }
}

impl From<bool> for OutputMode {
    fn from(json: bool) -> Self {
        if json {
            Self::Json
        } else {
            Self::Text
        }
    }
}

#[derive(Debug)]
pub(crate) enum HostReply<T = String> {
    Payload(T),
    Refused(Refusal),
    Transport(PollError),
}

#[derive(Debug)]
pub(crate) struct Refusal {
    pub(crate) code: Option<String>,
    pub(crate) reason: String,
    pub(crate) timed_out: bool,
    pub(crate) fields: Value,
    body: String,
}

impl Refusal {
    pub(crate) fn coded_reason(&self) -> String {
        match self.code.as_deref() {
            Some(code) if !self.reason.contains(code) => format!("{code}: {}", self.reason),
            _ => self.reason.clone(),
        }
    }

    pub(crate) fn stderr_line(&self) -> String {
        log::info!(
            "cli: host refused code={:?} timeout={}",
            self.code,
            self.timed_out
        );
        format!("error: {}", self.coded_reason())
    }

    pub(crate) fn json_body(&self) -> &str {
        self.body.trim()
    }

    pub(crate) fn report(&self, mode: OutputMode) -> i32 {
        self.report_with(mode, |body| println!("{body}"))
    }

    fn report_with(&self, mode: OutputMode, print_body: impl FnOnce(&str)) -> i32 {
        eprintln!("{}", self.stderr_line());
        if mode.is_json() {
            print_body(self.json_body());
        }
        1
    }
}

impl HostReply {
    pub(crate) fn decode(reply: Result<String, PollError>) -> Self {
        match reply {
            Ok(body) => Self::classify(body, false),
            Err(error) => Self::Transport(error),
        }
    }

    pub(super) fn decode_choice(reply: Result<Vec<u8>, PollError>) -> Self {
        let reply = reply.and_then(|bytes| {
            String::from_utf8(bytes).map_err(|error| {
                PollError::Unreadable(std::io::Error::new(std::io::ErrorKind::InvalidData, error))
            })
        });
        match reply {
            // A choice key can contain only whitespace. Zero bytes still
            // mean there is no reply body.
            Ok(key) if !key.is_empty() && key.trim().is_empty() => Self::Payload(key),
            reply => Self::decode(reply),
        }
    }

    /// A verb with a plain-text acknowledgement uses every other text as its refusal reason.
    pub(super) fn expect_text_ack(self, expected: &str) -> Self {
        match self {
            Self::Payload(text) if text.trim() != expected => Self::Refused(Refusal {
                code: None,
                reason: text.trim().to_owned(),
                timed_out: false,
                fields: Value::Null,
                body: text,
            }),
            reply => reply,
        }
    }

    fn classify(body: String, framed_refusal: bool) -> Self {
        let fields = serde_json::from_str::<Value>(&body).unwrap_or(Value::Null);
        let event_error = fields["type"] == "error";
        let error = fields["error"]
            .as_str()
            .or_else(|| event_error.then(|| fields["message"].as_str()).flatten())
            .filter(|message| !message.is_empty());
        let status = fields["status"].as_str();
        let code = fields["error_code"]
            .as_str()
            .filter(|code| !code.is_empty())
            .or_else(|| (status == Some("denied")).then_some("permission_denied"));
        if framed_refusal
            || event_error
            || error.is_some()
            || code.is_some()
            || fields["ok"] == false
        {
            Self::Refused(Refusal {
                code: code.map(str::to_owned),
                reason: error
                    .or(code)
                    .unwrap_or("the host refused the request")
                    .to_owned(),
                timed_out: fields["timeout"] == true,
                fields,
                body,
            })
        } else if body.trim().is_empty() {
            Self::Transport(PollError::Unreadable(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "empty host reply",
            )))
        } else {
            Self::Payload(body)
        }
    }
}

impl HostReply<Vec<u8>> {
    pub(crate) fn decode_slot(reply: Result<SlotReply, PollError>) -> Self {
        match reply {
            // Slot data belongs to the writer and may itself be JSON or empty.
            Ok(SlotReply::Data(bytes)) => Self::Payload(bytes),
            Ok(SlotReply::Err(bytes)) => {
                match HostReply::classify(String::from_utf8_lossy(&bytes).into_owned(), true) {
                    HostReply::Refused(refusal) => Self::Refused(refusal),
                    _ => unreachable!("the slot error frame marks a refusal"),
                }
            }
            Err(error) => Self::Transport(error),
        }
    }
}

impl<T> HostReply<T> {
    pub(crate) fn payload_or_report(self, label: &str, mode: OutputMode) -> Result<T, i32> {
        match self {
            Self::Payload(payload) => Ok(payload),
            Self::Refused(refusal) => Err(refusal.report(mode)),
            Self::Transport(PollError::TimedOut) => {
                eprintln!("error: timed out waiting for {label} response");
                Err(1)
            }
            Self::Transport(error) => {
                log::warn!("{label}:cli: {error}");
                eprintln!("error: {error}");
                Err(1)
            }
        }
    }
}

/// Some acknowledgement-only commands require an explicit success marker.
pub(super) fn acknowledged(value: &Value) -> bool {
    value["ok"] == true
}

#[cfg(test)]
mod tests {
    use super::{HostReply, OutputMode};

    #[test]
    fn refusal_stdout_mode_is_independent_of_call_order() {
        let body = r#"{"error_code":"permission_denied","error":"fixture refused"}"#;
        let HostReply::Refused(refusal) = HostReply::decode(Ok(body.to_owned())) else {
            panic!("expected a refusal");
        };

        for modes in [
            [OutputMode::Text, OutputMode::Json],
            [OutputMode::Json, OutputMode::Text],
        ] {
            for mode in modes {
                let mut stdout = Vec::new();
                assert_eq!(
                    refusal.report_with(mode, |body| stdout.push(body.to_owned())),
                    1
                );
                match mode {
                    OutputMode::Text => assert!(stdout.is_empty()),
                    OutputMode::Json => assert_eq!(stdout, [body]),
                }
            }
        }
    }

    #[test]
    fn a_refusal_is_never_a_payload() {
        for body in [
            r#"{"ok":false,"error_code":"scope_unresolvable","error":"no context"}"#,
            r#"{"error":"pane 4 not found"}"#,
            r#"{"error_code":"permission_denied"}"#,
            r#"{"ok":false}"#,
        ] {
            assert!(
                matches!(
                    HostReply::decode(Ok(body.to_string())),
                    HostReply::Refused(_)
                ),
                "{body}"
            );
            assert_eq!(
                HostReply::decode(Ok(body.to_string())).payload_or_report("test", OutputMode::Json),
                Err(1),
                "{body}"
            );
        }
        for body in [
            "yes",
            "[1,2]",
            r#"{"ok":true,"pane_id":3}"#,
            r#"{"panes":[]}"#,
        ] {
            assert!(
                matches!(HostReply::decode(Ok(body.to_string())), HostReply::Payload(payload) if payload == body)
            );
        }
    }

    #[test]
    fn transport_failures_are_distinct_from_host_timeouts() {
        use crate::rpc::{PollError, SlotReply};

        assert!(matches!(
            HostReply::decode(Err(PollError::TimedOut)),
            HostReply::Transport(PollError::TimedOut)
        ));
        assert!(matches!(
            HostReply::decode(Ok(String::new())),
            HostReply::Transport(PollError::Unreadable(_))
        ));
        assert!(matches!(
            HostReply::decode_slot(Ok(SlotReply::Data(Vec::new()))),
            HostReply::Payload(data) if data.is_empty()
        ));
        let body = br#"{"ok":false,"timeout":true,"error_code":"timeout"}"#;
        let HostReply::Refused(refusal) = HostReply::decode_slot(Ok(SlotReply::Err(body.to_vec())))
        else {
            panic!("expected a framed host refusal");
        };
        assert!(refusal.timed_out);
        assert_eq!(refusal.coded_reason(), "timeout");
        assert_eq!(refusal.json_body().as_bytes(), body);
    }

    #[test]
    fn choice_replies_distinguish_whitespace_from_empty_or_invalid_bodies() {
        use crate::rpc::PollError;

        assert!(matches!(
            HostReply::decode_choice(Ok(b" ".to_vec())),
            HostReply::Payload(key) if key == " "
        ));
        assert!(matches!(
            HostReply::decode_choice(Ok(Vec::new())),
            HostReply::Transport(PollError::Unreadable(error))
                if error.kind() == std::io::ErrorKind::UnexpectedEof
        ));
        assert!(matches!(
            HostReply::decode_choice(Ok(vec![0xff])),
            HostReply::Transport(PollError::Unreadable(error))
                if error.kind() == std::io::ErrorKind::InvalidData
        ));
    }

    /// Codex 3290 r4 S1: real CLI entries against a host that refuses, on
    /// the connection. Each exits nonzero; none reports the refusal as its
    /// result. `context new` used to exit 0 here.
    #[test]
    fn cli_entries_exit_nonzero_on_a_host_refusal() {
        use std::io::{BufRead as _, Write as _};
        let entries: [(&str, fn() -> i32); 4] = [
            ("context new", || {
                crate::cli::context_cli::context_new_cli(
                    Some("n"),
                    Some("/tmp"),
                    None,
                    &[],
                    "right",
                    Default::default(),
                    OutputMode::Text,
                )
            }),
            ("agent status", || {
                crate::cli::agent::agent_status_cli(false, false, false, OutputMode::Text)
            }),
            ("pane tree", || {
                crate::cli::pane::pane_tree_cli(None, false, OutputMode::Json)
            }),
            ("context list", crate::cli::context_cli::context_list_cli),
        ];
        for (name, entry) in entries {
            let env = crate::cli::test_env::socket_env_guard();
            let dir = tempfile::tempdir().unwrap();
            let socket = dir.path().join("notify.sock");
            let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
            env.set(&socket);
            let host = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                let mut line = String::new();
                std::io::BufReader::new(stream.try_clone().unwrap())
                    .read_line(&mut line)
                    .unwrap();
                stream
                    .write_all(
                        br#"{"ok":false,"error_code":"permission_denied","error":"refused"}"#,
                    )
                    .unwrap();
            });
            assert_eq!(entry(), 1, "{name}");
            host.join().unwrap();
        }
    }
}

#[cfg(all(test, unix))]
mod waiter_tests {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;
    use std::process::{Command, Output};

    use super::OutputMode;
    use crate::cli::{agents_api, context_cli, notify, open, pane, plugin, workspace};

    const CHILD: &str = "cli::reply::waiter_tests::run_waiter";
    const REFUSAL: &str = r#"{"error_code":"permission_denied","error":"fixture refused the request","pane_id":42,"state":"succeeded","status":"value","value":"fixture value"}"#;
    const CODE_ONLY: &str = r#"{"error_code":"permission_denied","pane_id":42}"#;

    #[test]
    fn run_waiter() {
        let Ok(entry) = std::env::var("PLEXI_REPLY_TEST_ENTRY") else {
            return;
        };
        let _channel = crate::config::set_test_channel("reply-fixture");
        let _profile =
            crate::config::set_test_profile_dir(std::env::current_dir().unwrap().join("profile"));
        if entry.starts_with("dashboard cancel") {
            std::fs::create_dir_all(crate::config::workspace_channel_dir()).unwrap();
        }
        let mode = OutputMode::from(entry.ends_with(" json"));
        crate::cli::set_command_socket_override(std::env::var_os("PLEXI_SOCKET").unwrap().into());
        let spawn = open::PaneNewCliArgs {
            name: Some("fixture"),
            ..Default::default()
        };
        let boot = open::AgentBootRequest {
            command: "fixture".to_string(),
            timeout_secs: Some(1.0),
        };
        let code = match entry.as_str() {
            "slot read" => pane::pane_slot_read_cli("result", Some(42)),
            "slot wait" => pane::pane_slot_wait_cli("result", Some(42), ".*", 1.0),
            "slot write" => {
                pane::pane_slot_write_cli("result", Some("data"), false, false, Some(42), mode)
            }
            "slot list" => pane::pane_slot_list_cli(Some(42), mode),
            "slot delete" => pane::pane_slot_delete_cli("result", Some(42), mode),
            "close" => pane::pane_close_cli(42, mode),
            "submit" => pane::pane_send_cli(42, "fixture", true, false),
            "boot" => open::pane_new_cli(open::PaneNewCliArgs {
                agent: Some(&boot),
                ..spawn
            }),
            "spawn" => open::pane_new_cli(spawn),
            "secret get" => workspace::workspace_secret_get("FIXTURE", true),
            "assistant send" => plugin::assistant_send_cli(
                Some("fixture"),
                None,
                None,
                None,
                plugin::AssistantSendRoute {
                    conversation: None,
                    join_desktop: false,
                    status_for: None,
                    client: None,
                    kind: None,
                },
            ),
            "notify choice" => notify::notify_cli(notify::NotifyCliArgs {
                title: "fixture",
                choices: &[crate::cli::parse_notify_choice(" :Continue").unwrap()],
                wait_for_response: true,
                wait_timeout_secs: 1,
                mode,
                ..Default::default()
            }),
            "notify dismiss" => notify::dismiss_notify_cli("fixture"),
            "pane tree json" => pane::pane_tree_cli(None, false, mode),
            "context new json" => context_cli::context_new_cli(
                Some("fixture"),
                None,
                None,
                &[],
                "right",
                Default::default(),
                mode,
            ),
            "context new child" => context_cli::context_new_cli(
                Some("fixture"),
                None,
                Some("parent"),
                &[],
                "right",
                Default::default(),
                mode,
            ),
            "dashboard cancel" | "dashboard cancel json" => {
                agents_api::command_view_cancel_cli("run_fixture", mode)
            }
            _ => panic!("unknown waiter {entry}"),
        };
        std::process::exit(code);
    }

    fn run_against_host(entry: &str, body: &str, slot_error: bool) -> (Output, serde_json::Value) {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("notify.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let workspace = dir.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([CHILD, "--exact", "--nocapture", "--test-threads=2"])
            .env("PLEXI_REPLY_TEST_ENTRY", entry)
            .env("PLEXI_SOCKET", &socket)
            .env("PLEXI_PANE_ID", "7")
            .env_remove("PLEXI_CHANNEL")
            .env_remove("PLEXI_CONTEXT_ID")
            .current_dir(workspace)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        listener.set_nonblocking(true).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if child.try_wait().unwrap().is_some() || std::time::Instant::now() >= deadline
                    {
                        let _ = child.kill();
                        let output = child.wait_with_output().unwrap();
                        panic!("{entry} never connected: {output:?}");
                    }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(error) => panic!("{entry}: {error}"),
            }
        };
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(2)))
            .unwrap();
        let mut line = String::new();
        BufReader::new(&stream).read_line(&mut line).unwrap();
        stream.write_all(b"\x06").unwrap();
        if slot_error {
            stream.write_all(b"\x00PLEXI_ERR:").unwrap();
        }
        stream.write_all(body.as_bytes()).unwrap();
        drop(stream);
        (
            child.wait_with_output().unwrap(),
            serde_json::from_str(&line).unwrap(),
        )
    }

    fn refusal_mismatches(entry: &str, body: &str, slot_error: bool, json: bool) -> Vec<String> {
        let (output, _) = run_against_host(entry, body, slot_error);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let reason = if body == CODE_ONLY {
            "permission_denied"
        } else {
            "permission_denied: fixture refused the request"
        };
        let stdout = stdout
            .trim()
            .strip_prefix("running 1 test")
            .unwrap_or(&stdout)
            .trim();
        if output.status.code() == Some(1)
            && stderr
                .lines()
                .any(|line| line == format!("error: {reason}"))
            && stdout == if json { body } else { "" }
        {
            Vec::new()
        } else {
            vec![format!(
                "{entry}: status={} stdout={stdout:?} stderr={stderr:?}",
                output.status
            )]
        }
    }

    #[test]
    fn slot_waiters_preserve_the_refusal_reason() {
        let failures: Vec<_> = [
            "slot wait",
            "slot read",
            "slot write",
            "slot list",
            "slot delete",
        ]
        .into_iter()
        .flat_map(|entry| {
            refusal_mismatches(
                entry,
                CODE_ONLY,
                matches!(entry, "slot wait" | "slot read"),
                false,
            )
        })
        .collect();
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    macro_rules! refusal_test {
        ($name:ident, $entry:literal, $body:ident, $json:literal) => {
            #[test]
            fn $name() {
                let failures = refusal_mismatches($entry, $body, false, $json);
                assert!(failures.is_empty(), "{}", failures.join("\n"));
            }
        };
    }

    refusal_test!(close_reports_refusal, "close", CODE_ONLY, false);
    refusal_test!(submit_reports_refusal, "submit", CODE_ONLY, false);
    refusal_test!(boot_reports_refusal, "boot", CODE_ONLY, false);
    refusal_test!(secret_get_reports_refusal, "secret get", REFUSAL, false);
    #[test]
    fn secret_get_reports_caller_revocation() {
        let (output, _) = run_against_host(
            "secret get",
            r#"{"status":"denied","reason":"revoked"}"#,
            false,
        );
        assert_eq!(output.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&output.stderr)
            .lines()
            .any(|line| line == "Caller revoked"));
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            "running 1 test"
        );
    }
    refusal_test!(
        assistant_send_reports_refusal,
        "assistant send",
        REFUSAL,
        true
    );
    refusal_test!(
        notify_dismiss_reports_refusal,
        "notify dismiss",
        REFUSAL,
        false
    );
    refusal_test!(spawn_reports_coded_refusal, "spawn", REFUSAL, false);
    refusal_test!(
        json_reader_keeps_refusal_body,
        "pane tree json",
        REFUSAL,
        true
    );
    refusal_test!(
        context_new_json_keeps_refusal_body,
        "context new json",
        REFUSAL,
        true
    );
    refusal_test!(
        context_new_child_keeps_refusal_body,
        "context new child",
        REFUSAL,
        true
    );
    refusal_test!(
        dashboard_cancel_json_keeps_refusal_body,
        "dashboard cancel json",
        REFUSAL,
        true
    );

    #[test]
    fn notify_wait_accepts_a_whitespace_choice_key() {
        let (output, request) = run_against_host("notify choice", " ", false);
        assert_eq!(request["options"][0]["value"], " ");
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(
            output.stdout.strip_prefix(b"\nrunning 1 test\n"),
            Some(&b""[..]),
            "{output:?}"
        );
    }

    #[test]
    fn submit_refusal_keeps_the_observed_input_line_on_stderr() {
        let (output, _) = run_against_host(
            "submit",
            r#"{"error":"submission not confirmed","input_line":["> fixture"]}"#,
            false,
        );
        assert_eq!(output.status.code(), Some(1));
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(
            stderr.contains("error: submission not confirmed\n  > fixture\n"),
            "{stderr}"
        );
        assert!(!String::from_utf8_lossy(&output.stdout).contains("fixture"));
    }

    #[test]
    fn public_dashboard_cancel_sends_cancel() {
        let (output, request) = run_against_host("dashboard cancel", r#"{"ok":true}"#, false);
        assert!(output.status.success(), "{output:?}");
        let request: plexi_protocol::HostRequest = serde_json::from_value(request).unwrap();
        let plexi_protocol::HostRequest::AgentsDashboard { op, payload, .. } = request else {
            panic!("expected a dashboard request");
        };
        assert_eq!(op, "cancel");
        assert_eq!(payload["run"], "run_fixture");
        assert!(std::path::Path::new(payload["workspace"].as_str().unwrap()).ends_with("workspace"));
    }
}

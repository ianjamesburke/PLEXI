/// Parse the `--scope` flag into an explicit wire scope.
///
/// `None` (flag absent) stays `None` so the host applies
/// `NotifyScope::default()`. Every named scope — **including `global`** — maps
/// to an explicit `Some`: relying on the host's fallback to mean "global"
/// silently reinterprets `--scope global` whenever that default changes.
pub fn parse_notify_scope(
    scope: Option<&str>,
) -> Result<Option<crate::app_protocol::NotifyScope>, String> {
    match scope {
        None => Ok(None),
        Some("window") => Ok(Some(crate::app_protocol::NotifyScope::Window)),
        Some("context") => Ok(Some(crate::app_protocol::NotifyScope::Context)),
        Some("global") => Ok(Some(crate::app_protocol::NotifyScope::Global)),
        Some(other) => Err(format!(
            "error: --scope must be window, context, or global — got {other:?}"
        )),
    }
}

// Arg-struct refactor is a design change tracked in stint 0661.
#[allow(clippy::too_many_arguments)]
pub fn notify_cli(
    title: &str,
    body: &str,
    level: &str,
    choices: &[(String, String, Option<String>)],
    wait_for_response: bool,
    display_timeout_secs: u64,
    wait_timeout_secs: u64,
    scope: Option<crate::app_protocol::NotifyScope>,
    source_context_id: Option<u64>,
    source_pane_id: Option<u64>,
) -> i32 {
    // An explicit context/window scope needs the caller's own identity on the
    // wire — the host never guesses the sender from its active state, so
    // without `PLEXI_CONTEXT_ID` there is no context to attach to.
    if matches!(
        scope,
        Some(crate::app_protocol::NotifyScope::Window | crate::app_protocol::NotifyScope::Context)
    ) && source_context_id.is_none()
    {
        let scope_name = if scope == Some(crate::app_protocol::NotifyScope::Window) {
            "window"
        } else {
            "context"
        };
        eprintln!(
            "error: --scope {scope_name} requires a caller context — PLEXI_CONTEXT_ID is not set; \
             run this inside a Plexi pane or use --scope global"
        );
        return 1;
    }

    let socket_path = match super::resolve_command_socket() {
        Some(path) => path,
        None => {
            eprintln!("error: PLEXI_SOCKET is not set — run this inside a Plexi terminal pane");
            return 1;
        }
    };

    let options_json: Vec<serde_json::Value> = choices
        .iter()
        .map(|(key, label, host_action)| {
            let mut opt = serde_json::json!({"label": label, "value": key, "shortcut": key});
            if let Some(ha) = host_action {
                opt["host_action"] = serde_json::Value::String(ha.clone());
            }
            opt
        })
        .collect();

    let kind = if choices.is_empty() {
        "message".to_string()
    } else {
        "choice".to_string()
    };
    let response_file_str = if !choices.is_empty() && wait_for_response {
        Some(crate::rpc::response_file("notify-response", "txt"))
    } else {
        None
    };

    let notify_id = format!(
        "cli:{}:{}",
        source_pane_id.unwrap_or(0),
        uuid::Uuid::new_v4()
    );
    let mut payload = serde_json::json!({
        "type": "notify",
        "notify_id": notify_id,
        "level": level,
        "title": title,
        "body": body,
        "kind": kind,
        "options": options_json,
        "priority": 50,
    });
    if display_timeout_secs > 0 {
        payload["timeout_secs"] = serde_json::Value::from(display_timeout_secs);
    }
    if let Some(ref rf) = response_file_str {
        payload["response_file"] = serde_json::Value::String(rf.clone());
    }
    if let Some(s) = scope {
        let s_str = match s {
            crate::app_protocol::NotifyScope::Window => "window",
            crate::app_protocol::NotifyScope::Context => "context",
            crate::app_protocol::NotifyScope::Global => "global",
        };
        payload["scope"] = serde_json::Value::String(s_str.to_string());
    }
    // The caller's own identity, so the host attaches the notification to the
    // context that actually produced it. Absent for outside-pane callers.
    if let Some(ctx_id) = source_context_id {
        payload["source_context_id"] = serde_json::Value::from(ctx_id);
    }
    if let Some(pane_id) = source_pane_id {
        payload["source_pane_id"] = serde_json::Value::from(pane_id);
    }

    log::info!(
        "notify:cli: sending via socket choices={} wait_for_response={} scope={:?} source_context_id={:?} source_pane_id={:?} response_file={:?}",
        choices.len(),
        wait_for_response,
        scope,
        source_context_id,
        source_pane_id,
        response_file_str
    );

    use std::io::Write;
    use std::os::unix::net::UnixStream;
    let mut stream = match UnixStream::connect(&socket_path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::ConnectionRefused => {
            let _ = std::fs::remove_file(&socket_path);
            eprintln!("error: Plexi is not responding (stale socket removed). Is Plexi running?");
            return 1;
        }
        Err(e) => {
            eprintln!("error: could not connect to PLEXI_SOCKET {socket_path:?}: {e}");
            return 1;
        }
    };
    let line = format!("{payload}\n");
    if let Err(e) = stream.write_all(line.as_bytes()) {
        eprintln!("error: could not write to socket: {e}");
        return 1;
    }

    // Fire-and-forget path — command is delivered, nothing to wait for.
    let Some(response_file) = response_file_str else {
        println!("{notify_id}");
        return 0;
    };
    log::info!("notify:cli: polling for response at {response_file:?}");

    let timeout =
        (wait_timeout_secs > 0).then(|| std::time::Duration::from_secs(wait_timeout_secs));
    match crate::rpc::poll_string(&response_file, timeout) {
        Ok(key) => {
            log::info!("notify:cli: response received {:?}", key.trim());
            print!("{}", key.trim());
            0
        }
        Err(crate::rpc::PollError::TimedOut) => {
            log::info!("notify:cli: choice wait timed out after {wait_timeout_secs}s");
            2
        }
        Err(e) => {
            log::warn!("notify:cli: {e}");
            eprintln!("error: {e}");
            1
        }
    }
}

pub fn dismiss_notify_cli(
    notify_id: &str,
    source_context_id: Option<u64>,
    source_pane_id: Option<u64>,
) -> i32 {
    let (Some(source_context_id), Some(source_pane_id)) = (source_context_id, source_pane_id)
    else {
        eprintln!("error: notify dismiss requires a caller pane and context");
        return 1;
    };
    let Some(socket_path) = super::resolve_command_socket() else {
        eprintln!("error: PLEXI_SOCKET is not set — run this inside a Plexi terminal pane");
        return 1;
    };
    let response_file = crate::rpc::response_file("notify-dismiss", "txt");
    let payload = serde_json::json!({
        "type": "dismiss_notification",
        "notify_id": notify_id,
        "source_context_id": source_context_id,
        "source_pane_id": source_pane_id,
        "response_file": response_file,
    });
    use std::io::Write;
    use std::os::unix::net::UnixStream;
    let mut stream = match UnixStream::connect(&socket_path) {
        Ok(stream) => stream,
        Err(e) => {
            eprintln!("error: could not connect to PLEXI_SOCKET {socket_path:?}: {e}");
            return 1;
        }
    };
    if let Err(e) = stream.write_all(format!("{payload}\n").as_bytes()) {
        eprintln!("error: could not write to socket: {e}");
        return 1;
    }
    match crate::rpc::poll_string(&response_file, Some(std::time::Duration::from_secs(30))) {
        Ok(result) if result.trim() == "dismissed" => 0,
        Ok(result) => {
            eprintln!("error: {}", result.trim());
            1
        }
        Err(e) => {
            eprintln!("error: notify dismiss: {e}");
            1
        }
    }
}

#[cfg(test)]
mod notify_tests {
    use super::notify_cli;
    use crate::cli::parse_notify_choice;
    use serde_json::Value;
    use std::io::{BufRead, BufReader};
    use std::os::unix::net::UnixListener;

    use crate::cli::test_env::{socket_env_guard, SocketEnvGuard};

    /// Runs `run_cli` against a throwaway listener with `PLEXI_SOCKET` pointed at
    /// it. The caller owns the [`SocketEnvGuard`], which both serialises against
    /// every other socket-mutating CLI test and restores the prior value on drop.
    fn capture_notify_payload<F>(env: &SocketEnvGuard, run_cli: F) -> (i32, Value)
    where
        F: FnOnce() -> i32,
    {
        let dir = tempfile::tempdir().expect("tempdir");
        let socket_path = dir.path().join("notify.sock");
        let listener = UnixListener::bind(&socket_path).expect("bind notify socket");
        env.set(&socket_path);

        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept notify connection");
            let mut line = String::new();
            BufReader::new(stream)
                .read_line(&mut line)
                .expect("read notify payload");
            let payload = serde_json::from_str::<Value>(&line).expect("notify payload json");
            tx.send(payload).ok();
        });

        let code = run_cli();
        let payload = rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("notify listener did not receive a connection within 5s — notify_cli likely failed to connect");
        (code, payload)
    }

    /// Without PLEXI_SOCKET set, notify_cli must fail fast (exit 1) rather than panic.
    #[test]
    fn notify_cli_no_socket_returns_one() {
        let env = socket_env_guard();
        env.unset();
        let code = notify_cli(
            "Test title",
            "Test body",
            "info",
            &[],
            true,
            0,
            0,
            None,
            None,
            None,
        );
        assert_eq!(code, 1);
    }

    #[test]
    fn notify_cli_nonblocking_choice_omits_response_file() {
        let env = socket_env_guard();
        let choices = vec![(
            "talk".to_string(),
            "Talk to Claude".to_string(),
            Some("pane_focus:188".to_string()),
        )];

        let (code, payload) = capture_notify_payload(&env, || {
            notify_cli(
                "Ready",
                "Review tests",
                "info",
                &choices,
                false,
                0,
                0,
                None,
                None,
                None,
            )
        });

        assert_eq!(code, 0);
        assert_eq!(payload["kind"], "choice");
        assert!(
            payload.get("response_file").is_none(),
            "non-blocking choice must not create a response file: {payload}"
        );
        assert_eq!(payload["options"][0]["value"], "talk");
        assert_eq!(payload["options"][0]["label"], "Talk to Claude");
        assert_eq!(payload["options"][0]["host_action"], "pane_focus:188");
    }

    #[test]
    fn notify_cli_timeout_is_sent_as_display_lifetime() {
        let env = socket_env_guard();
        let (code, payload) = capture_notify_payload(&env, || {
            notify_cli(
                "Expiry",
                "body",
                "info",
                &[],
                false,
                30,
                0,
                None,
                None,
                None,
            )
        });

        assert_eq!(code, 0);
        assert_eq!(payload["timeout_secs"], 30);
        assert!(payload["notify_id"]
            .as_str()
            .is_some_and(|id| id.starts_with("cli:0:")));
    }

    /// The stint 0536 leak fix: a blocking choice that times out must exit 2
    /// and must not leave a response file behind in the profile's rpc/ dir.
    #[test]
    fn notify_cli_timeout_leaves_no_response_file() {
        let env = socket_env_guard();
        let _channel_guard = crate::config::set_test_channel("notify-test");
        let choices = vec![("talk".to_string(), "Talk".to_string(), None)];
        let (code, payload) = capture_notify_payload(&env, || {
            notify_cli(
                "Ready",
                "Review tests",
                "info",
                &choices,
                true,
                1,
                1,
                None,
                None,
                None,
            )
        });
        assert_eq!(code, 2, "timeout must exit 2");
        let rf = payload["response_file"]
            .as_str()
            .expect("blocking choice response_file");
        assert!(
            rf.contains("/rpc/"),
            "response file must live in the rpc/ subdir: {rf}"
        );
        assert!(
            !std::path::Path::new(rf).exists(),
            "timeout must not leave a response file behind"
        );
    }

    #[test]
    fn notify_cli_blocking_choice_sends_response_file() {
        let env = socket_env_guard();
        let dir = tempfile::tempdir().expect("tempdir");
        let socket_path = dir.path().join("notify.sock");
        let listener = UnixListener::bind(&socket_path).expect("bind notify socket");
        env.set(&socket_path);
        let _channel_guard = crate::config::set_test_channel("notify-test");

        let handle = std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept notify connection");
            let mut line = String::new();
            BufReader::new(stream)
                .read_line(&mut line)
                .expect("read notify payload");
            let payload: Value = serde_json::from_str(&line).expect("notify payload json");
            let response_file = payload["response_file"]
                .as_str()
                .expect("blocking choice response_file")
                .to_string();
            std::fs::create_dir_all(
                std::path::Path::new(&response_file)
                    .parent()
                    .expect("response file parent"),
            )
            .expect("create response dir");
            std::fs::write(&response_file, "talk").expect("write response");
            payload
        });

        let choices = vec![("talk".to_string(), "Talk to Claude".to_string(), None)];
        let code = notify_cli(
            "Ready",
            "Review tests",
            "info",
            &choices,
            true,
            1,
            1,
            None,
            None,
            None,
        );
        let payload = handle.join().expect("payload thread");

        assert_eq!(code, 0);
        assert_eq!(payload["kind"], "choice");
        assert!(
            payload["response_file"].as_str().is_some(),
            "blocking choice must include a response file: {payload}"
        );
    }

    #[test]
    fn parse_choice_two_segment() {
        let (key, label, action) = parse_notify_choice("open_pr:Open PR").unwrap();
        assert_eq!(key, "open_pr");
        assert_eq!(label, "Open PR");
        assert!(action.is_none());
    }

    #[test]
    fn parse_choice_three_segment_host_action() {
        let (key, label, action) = parse_notify_choice("Talk to Claude:pane_focus:188").unwrap();
        assert_eq!(key, "Talk to Claude");
        assert_eq!(label, "Talk to Claude");
        assert_eq!(action.as_deref(), Some("pane_focus:188"));
    }

    #[test]
    fn parse_choice_four_segment_key_label_action() {
        let (key, label, action) = parse_notify_choice("c:Talk to Claude:pane_focus:188").unwrap();
        assert_eq!(key, "c");
        assert_eq!(label, "Talk to Claude");
        assert_eq!(action.as_deref(), Some("pane_focus:188"));
    }

    #[test]
    fn parse_choice_five_segment_is_error() {
        let err = parse_notify_choice("a:b:c:d:e").unwrap_err();
        assert!(
            err.contains("5"),
            "error should mention segment count: {err}"
        );
    }

    /// Fix round 1 blocker 2: the CLI stamps the caller's own identity onto
    /// the wire so the host never derives provenance from its active state.
    #[test]
    fn notify_cli_sends_caller_identity() {
        let env = socket_env_guard();
        let (code, payload) = capture_notify_payload(&env, || {
            notify_cli("T", "B", "info", &[], false, 0, 0, None, Some(7), Some(42))
        });
        assert_eq!(code, 0);
        assert_eq!(payload["source_context_id"], 7);
        assert_eq!(payload["source_pane_id"], 42);
    }

    /// An outside-pane caller has no identity — the fields must be absent, not
    /// zero, so the host can tell "no sender" from "sender id 0".
    #[test]
    fn notify_cli_omits_identity_when_absent() {
        let env = socket_env_guard();
        let (code, payload) = capture_notify_payload(&env, || {
            notify_cli("T", "B", "info", &[], false, 0, 0, None, None, None)
        });
        assert_eq!(code, 0);
        assert!(payload.get("source_context_id").is_none());
        assert!(payload.get("source_pane_id").is_none());
    }

    /// An explicit `--scope context` from outside any pane fails fast at the
    /// CLI with a message naming the missing identity — there is no context
    /// for the host to attach the notification to.
    #[test]
    fn notify_cli_explicit_context_scope_without_identity_errors() {
        let env = socket_env_guard();
        env.unset();
        for scope in [
            crate::app_protocol::NotifyScope::Context,
            crate::app_protocol::NotifyScope::Window,
        ] {
            let code = notify_cli("T", "B", "info", &[], false, 0, 0, Some(scope), None, None);
            assert_eq!(code, 1, "{scope:?} without a caller context must error");
        }
    }

    #[test]
    fn parse_choice_one_segment_is_error() {
        let err = parse_notify_choice("nocolon").unwrap_err();
        assert!(
            err.contains("1"),
            "error should mention segment count: {err}"
        );
    }
}

pub fn notify_cli(
    title: &str,
    body: &str,
    level: &str,
    choices: &[(String, String, Option<String>)],
    wait_for_response: bool,
    timeout_secs: u64,
    scope: Option<crate::app_protocol::NotifyScope>,
) -> i32 {
    let socket_path = match std::env::var("PLEXI_SOCKET") {
        Ok(v) => v,
        Err(_) => {
            eprintln!("error: PLEXI_SOCKET is not set — run this inside a Plexi terminal pane");
            return 1;
        }
    };

    let id = uuid::Uuid::new_v4();

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
        let rf = crate::config::config_dir()
            .join(format!("notify-response-{id}.txt"))
            .to_string_lossy()
            .into_owned();
        Some(rf)
    } else {
        None
    };

    let mut payload = serde_json::json!({
        "type": "notify",
        "level": level,
        "title": title,
        "body": body,
        "kind": kind,
        "options": options_json,
        "priority": 50,
    });
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

    log::info!(
        "notify:cli: sending via socket choices={} wait_for_response={} scope={:?} response_file={:?}",
        choices.len(),
        wait_for_response,
        scope,
        response_file_str
    );

    use std::io::Write;
    use std::os::unix::net::UnixStream;
    let mut stream = match UnixStream::connect(&socket_path) {
        Ok(s) => s,
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
        if timeout_secs > 0 {
            eprintln!(
                "warning: --timeout only limits waiting for a choice response; notification queued"
            );
        }
        println!("notification queued");
        return 0;
    };
    let response_file = std::path::PathBuf::from(response_file);
    log::info!("notify:cli: polling for response at {:?}", response_file);

    let deadline = if timeout_secs > 0 {
        Some(std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs))
    } else {
        None
    };

    loop {
        if response_file.exists() {
            match std::fs::read_to_string(&response_file) {
                Ok(key) => {
                    log::info!("notify:cli: response received {:?}", key.trim());
                    let _ = std::fs::remove_file(&response_file);
                    print!("{}", key.trim());
                    return 0;
                }
                Err(e) => {
                    log::warn!("notify:cli: could not read response file: {e}");
                    eprintln!("error: could not read response file: {e}");
                    return 1;
                }
            }
        }
        if let Some(dl) = deadline {
            if std::time::Instant::now() >= dl {
                log::info!("notify:cli: timed out after {timeout_secs}s");
                return 2;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

#[cfg(test)]
mod notify_tests {
    use super::notify_cli;
    use crate::cli::parse_notify_choice;
    use serde_json::Value;
    use std::io::{BufRead, BufReader};
    use std::os::unix::net::UnixListener;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn capture_notify_payload<F>(run_cli: F) -> (i32, Value)
    where
        F: FnOnce() -> i32,
    {
        let dir = tempfile::tempdir().expect("tempdir");
        let socket_path = dir.path().join("notify.sock");
        let listener = UnixListener::bind(&socket_path).expect("bind notify socket");
        std::env::set_var("PLEXI_SOCKET", &socket_path);

        let handle = std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept notify connection");
            let mut line = String::new();
            BufReader::new(stream)
                .read_line(&mut line)
                .expect("read notify payload");
            serde_json::from_str::<Value>(&line).expect("notify payload json")
        });

        let code = run_cli();
        std::env::remove_var("PLEXI_SOCKET");
        let payload = handle.join().expect("payload thread");
        (code, payload)
    }

    /// Without PLEXI_SOCKET set, notify_cli must fail fast (exit 1) rather than panic.
    #[test]
    fn notify_cli_no_socket_returns_one() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("PLEXI_SOCKET");
        let code = notify_cli("Test title", "Test body", "info", &[], true, 0, None);
        assert_eq!(code, 1);
    }

    #[test]
    fn notify_cli_nonblocking_choice_omits_response_file() {
        let _guard = ENV_LOCK.lock().unwrap();
        let choices = vec![(
            "talk".to_string(),
            "Talk to Claude".to_string(),
            Some("pane_focus:188".to_string()),
        )];

        let (code, payload) = capture_notify_payload(|| {
            notify_cli("Ready", "Review tests", "info", &choices, false, 0, None)
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
    fn notify_cli_blocking_choice_sends_response_file() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().expect("tempdir");
        let socket_path = dir.path().join("notify.sock");
        let listener = UnixListener::bind(&socket_path).expect("bind notify socket");
        std::env::set_var("PLEXI_SOCKET", &socket_path);
        std::env::set_var("PLEXI_CHANNEL", "notify-test");

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
        let code = notify_cli("Ready", "Review tests", "info", &choices, true, 1, None);
        std::env::remove_var("PLEXI_SOCKET");
        std::env::remove_var("PLEXI_CHANNEL");
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

    #[test]
    fn parse_choice_one_segment_is_error() {
        let err = parse_notify_choice("nocolon").unwrap_err();
        assert!(
            err.contains("1"),
            "error should mention segment count: {err}"
        );
    }

    /// --host-action merges into a clean key:Label choice.
    #[test]
    fn host_action_merges_into_clean_choice() {
        let (key, label, embedded) = parse_notify_choice("view:View results").unwrap();
        assert!(embedded.is_none());
        // Simulate the merge: host_action_map has "view" → "pane_focus:99"
        let merged_action = Some("pane_focus:99".to_string());
        let action = Some(merged_action.unwrap());
        assert_eq!(key, "view");
        assert_eq!(label, "View results");
        assert_eq!(action.as_deref(), Some("pane_focus:99"));
    }

    /// --host-action overrides an embedded action in a 4-segment --choice.
    #[test]
    fn host_action_overrides_embedded_choice_action() {
        let (key, label, embedded) =
            parse_notify_choice("a:Talk to Claude:pane_focus:OLD").unwrap();
        assert_eq!(embedded.as_deref(), Some("pane_focus:OLD"));
        // host_action_map contains key "a" → "pane_focus:NEW"
        let override_action = Some("pane_focus:NEW".to_string());
        let final_action = override_action.map(Some).unwrap_or(embedded);
        assert_eq!(key, "a");
        assert_eq!(label, "Talk to Claude");
        assert_eq!(final_action.as_deref(), Some("pane_focus:NEW"));
    }

    /// #840: snooze action type parses to the correct host_action string.
    #[test]
    fn parse_choice_snooze_action() {
        let (key, label, action) =
            parse_notify_choice("snooze5:Remind me in 5 min:snooze:300").unwrap();
        assert_eq!(key, "snooze5");
        assert_eq!(label, "Remind me in 5 min");
        assert_eq!(action.as_deref(), Some("snooze:300"));
    }

    /// #840: three-segment form also works for snooze.
    #[test]
    fn parse_choice_snooze_three_segment() {
        let (key, label, action) = parse_notify_choice("Snooze 5min:snooze:300").unwrap();
        assert_eq!(key, "Snooze 5min");
        assert_eq!(label, "Snooze 5min");
        assert_eq!(action.as_deref(), Some("snooze:300"));
    }
}

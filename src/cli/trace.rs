//! CLI invocation tracing over the host command transport.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_trace_redacts_secret_values_and_assignments() {
        let argv = ["plexi", "secret", "set", "FOO", "bar"];
        assert_eq!(redact_argv(&argv.map(str::to_string), &[])[4], "<redacted>");
        let argv = [
            "plexi",
            "pane",
            "new",
            "--secret",
            "hidden",
            "KEY=value",
            "--custom-token",
            "private",
            "--secret=other",
        ];
        let redacted = redact_argv(&argv.map(str::to_string), &["CUSTOM_TOKEN".into()]);
        assert_eq!(redacted[4], "<redacted>");
        assert_eq!(redacted[5], "KEY=<redacted>");
        assert_eq!(redacted[7], "<redacted>");
        assert_eq!(redacted[8], "--secret=<redacted>");
        assert!(!redacted.join(" ").contains("private"));
    }

    #[test]
    fn cli_trace_keeps_option_equals_and_redacts_embedded_assignments() {
        let argv = [
            "plexi",
            "agent",
            "spawn",
            "researcher",
            "--subcontext=auth",
            "--task",
            "echo KEY='two words'",
        ]
        .map(str::to_string);
        let safe = redact_argv(&argv, &[]);
        assert_eq!(safe[4], "--subcontext=auth");
        assert!(!safe[6].contains("two words"));
        assert_eq!(redact_argv(&safe, &[]), safe, "redaction is idempotent");
    }

    #[cfg(unix)]
    #[test]
    fn cli_trace_socket_round_trip_is_durable_before_ack() {
        use std::io::BufRead;
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("trace.sock");
        let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        let profile = dir.path().to_path_buf();
        let server = std::thread::spawn(move || {
            let _profile = crate::config::set_test_profile_dir(profile.clone());
            crate::host::flight_recorder::adopt_session(&profile, "sess-socket");
            let wake = std::sync::Arc::new(crate::app::ui_mailbox::EguiWake::new(
                egui::Context::default(),
            ));
            let (mailbox, _receiver) =
                crate::app::ui_mailbox::UiMailbox::channel(wake, "cli_trace_test");
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut line = String::new();
                std::io::BufReader::new(stream.try_clone().unwrap())
                    .read_line(&mut line)
                    .unwrap();
                crate::app::handle_socket_line(&line, &mailbox, None, &mut stream);
            }
        });
        let id = uuid::Uuid::new_v4().to_string();
        let mut frame = json!({"type":"cli_trace","id":id,"phase":"start","pane_id":19,
            "argv":["plexi-alpha","pane","new","--right"]});
        assert!(send_frame(&socket, &frame));
        frame["phase"] = json!("exit");
        frame["exit_code"] = json!(0);
        assert!(send_frame(&socket, &frame));
        server.join().unwrap();
        let (_, _, rows) =
            crate::cli::debug::build_show(dir.path(), Some("sess-socket"), Some(19), None).unwrap();
        let calls = crate::host::flight_recorder::cli_calls(rows.clone(), Some(19));
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0]["data"]["exit_code"], 0);
        assert!(crate::cli::debug::render_line(&calls[0]).contains("--right"));
        let completed = rows.iter().find(|row| row["kind"] == "cli_call").unwrap();
        assert_eq!(completed["data"]["exit_code"], 0);
    }

    #[test]
    fn cli_trace_redacts_secrets_inside_shell_arguments() {
        let argv = [
            "plexi",
            "pane",
            "send",
            "19",
            "plexi secret set FOO 'hidden value'; echo --secret=other",
            "run --custom-name 'private value'",
        ]
        .map(str::to_string);
        let safe = redact_argv(&argv, &["CUSTOM_NAME".into()]);
        let text = safe.join(" ");
        assert!(!text.contains("hidden value"), "{text}");
        assert!(!text.contains("other"), "{text}");
        assert!(!text.contains("private value"), "{text}");
        assert_eq!(redact_argv(&safe, &["CUSTOM_NAME".into()]), safe);
    }

    #[test]
    fn cli_trace_preserves_useful_argv() {
        let argv = ["plexi-alpha", "pane", "new", "--right"].map(str::to_string);
        assert_eq!(redact_argv(&argv, &[]), argv);
    }
}

use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

struct Invocation {
    frame: serde_json::Value,
    socket: PathBuf,
}

fn active() -> &'static Mutex<Option<Invocation>> {
    static ACTIVE: OnceLock<Mutex<Option<Invocation>>> = OnceLock::new();
    ACTIVE.get_or_init(|| Mutex::new(None))
}

/// Begin once per CLI process, before dispatch. Debug/skill readers are excluded
/// so a review-only draft does not mutate the session it is reading.
pub fn begin(argv: &[String]) {
    let socket =
        super::resolve_command_socket().unwrap_or_else(crate::platform::ipc::endpoint_path);
    let mut slot = active().lock().unwrap_or_else(|e| e.into_inner());
    if slot.is_some() {
        return;
    }
    let id = uuid::Uuid::new_v4().to_string();
    let frame = json!({
        "type": "cli_trace", "id": id, "phase": "start",
        "argv": redact_argv(argv, &known_secret_names(&crate::config::config_dir())),
        "pane_id": std::env::var("PLEXI_PANE_ID").ok().and_then(|v| v.parse::<u64>().ok()),
        "agent_session_id": std::env::var("PLEXI_AGENT_SESSION_ID").ok(),
        "ts": crate::platform::clock::now_rfc3339(),
    });
    if send_frame(&socket, &frame) {
        *slot = Some(Invocation { frame, socket });
    }
}

/// All CLI exits pass here, after command-specific response interpretation.
pub fn exit(code: i32) -> ! {
    finish(code);
    std::process::exit(code)
}

fn finish(code: i32) {
    let invocation = active().lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(mut invocation) = invocation {
        invocation.frame["phase"] = json!("exit");
        invocation.frame["exit_code"] = json!(code);
        send_frame(&invocation.socket, &invocation.frame);
    }
}

fn send_frame(socket: &Path, frame: &serde_json::Value) -> bool {
    use std::io::Read;
    let timeout = std::time::Duration::from_secs(2);
    #[cfg(test)]
    let timeout = crate::testing::load_aware_timeout(timeout);
    let line = format!("{frame}\n");
    let result = super::send_line_to_socket(socket, line.as_bytes(), timeout);
    match result {
        Ok(mut stream) => {
            #[cfg(unix)]
            if let Err(error) = stream.set_nonblocking(false) {
                log::warn!("cli_trace: cannot enable bounded acknowledgement read: {error}");
                return false;
            }
            let mut ack = [0u8];
            let result = stream
                .set_read_timeout(Some(timeout))
                .and_then(|()| stream.read_exact(&mut ack));
            match result {
                Ok(()) if ack == [0x06] => true,
                other => {
                    log::warn!("cli_trace: host acknowledgement failed: {other:?}");
                    false
                }
            }
        }
        Err(error) => {
            log::warn!("cli_trace: could not record invocation: {error:?}");
            false
        }
    }
}

/// Read only the names cache; tracing must never prompt for a Keychain value.
pub(crate) fn known_secret_names(profile: &Path) -> Vec<String> {
    let path = profile.join("secrets-index.json");
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(error) => {
            log::warn!(
                "cli_trace: cannot read secret names {}: {error}",
                path.display()
            );
            return Vec::new();
        }
    };
    match serde_json::from_str::<Vec<String>>(&raw) {
        Ok(accounts) => accounts
            .into_iter()
            .filter_map(|account| account.rsplit(':').next().map(str::to_string))
            .collect(),
        Err(error) => {
            log::warn!(
                "cli_trace: cannot parse secret names {}: {error}",
                path.display()
            );
            Vec::new()
        }
    }
}

fn secret_name(name: &str, known: &[String]) -> bool {
    let normalized = name
        .trim_start_matches('-')
        .replace('-', "_")
        .to_ascii_uppercase();
    known
        .iter()
        .any(|name| name.eq_ignore_ascii_case(&normalized))
        || [
            "SECRET",
            "TOKEN",
            "PASSWORD",
            "PASSWD",
            "API_KEY",
            "PRIVATE_KEY",
            "CREDENTIAL",
            "AUTHORIZATION",
        ]
        .iter()
        .any(|part| normalized.contains(part))
}

/// Redact before transport and again before persistence. Keep option names so
/// the sequence remains useful, but never retain env assignment values.
pub(crate) fn redact_argv(argv: &[String], known: &[String]) -> Vec<String> {
    let mut hide_next = false;
    let secret_set = argv.windows(2).position(|pair| pair == ["secret", "set"]);
    argv.iter()
        .enumerate()
        .map(|(index, arg)| {
            if hide_next {
                hide_next = false;
                return "<redacted>".to_string();
            }
            // The legacy positional value is redacted even if clap rejects it.
            if secret_set.is_some_and(|start| index > start + 2) && !arg.starts_with('-') {
                return "<redacted>".to_string();
            }
            if let Some((key, _)) = arg.split_once('=') {
                if (!key.is_empty()
                    && key.chars().enumerate().all(|(i, c)| {
                        c == '_' || c.is_ascii_alphabetic() || (i > 0 && c.is_ascii_digit())
                    }))
                    || (!key.contains(char::is_whitespace) && secret_name(key, known))
                {
                    return format!("{key}=<redacted>");
                }
            }
            if (arg.starts_with('-') && secret_name(arg, known))
                || known.iter().any(|name| name.eq_ignore_ascii_case(arg))
            {
                hide_next = true;
            }
            if arg.starts_with("--") && !arg.contains(char::is_whitespace) {
                return arg.clone();
            }
            // A shell fragment is one argv value. If it embeds a secret-setting
            // command or secret option, redact the whole fragment rather than
            // trying to parse shell quoting and risk retaining part of a value.
            let words: Vec<&str> = arg.split_whitespace().collect();
            if words.len() > 1
                && (words.windows(2).any(|pair| pair == ["secret", "set"])
                    || words.iter().any(|word| {
                        let name = word
                            .split('=')
                            .next()
                            .unwrap_or(word)
                            .trim_matches(['\'', '"']);
                        (name.starts_with('-') && secret_name(name, known))
                            || known.iter().any(|known| known.eq_ignore_ascii_case(name))
                    }))
            {
                return "<redacted>".to_string();
            }
            // Also sanitize assignment strings embedded in a shell command or prompt.
            static ASSIGNMENT: OnceLock<regex::Regex> = OnceLock::new();
            let re = ASSIGNMENT.get_or_init(|| {
                regex::Regex::new(r#"\b[A-Za-z_][A-Za-z0-9_]*=(?:"[^"]*"|'[^']*'|[^\s]+)"#)
                    .expect("static assignment regex")
            });
            re.replace_all(arg, |caps: &regex::Captures<'_>| {
                let key = caps[0].split_once('=').expect("matched assignment").0;
                format!("{key}=<redacted>")
            })
            .into_owned()
        })
        .collect()
}

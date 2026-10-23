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
        assert!(is_text_placeholder(&redacted[5]));
        assert_eq!(redacted[7], "<redacted>");
        assert!(!redacted[8].contains("other"));
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
            let wrong_profile = tempfile::tempdir().unwrap();
            let _profile = crate::config::set_test_profile_dir(wrong_profile.path().to_path_buf());
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
                crate::app::handle_socket_line_in(&profile, &line, &mailbox, None, &mut stream);
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
        assert!(crate::cli::debug::render_line(&calls[0], false).contains("--right"));
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
    fn cli_trace_invalid_content_call_fails_closed_and_redacts_secrets() {
        let argv = [
            "plexi",
            "agent",
            "spawn",
            "--grant",
            "agents.ping=allow",
            "tool.name=ask",
            "tool.name=deny",
            "mode=allow",
            "--token",
            "abc",
            "API_KEY=x",
            "echo agents.ping=allow API_KEY='two words' password=hidden",
        ]
        .map(str::to_string);
        let safe = redact_argv(&argv, &[]);
        assert!(safe[4..8].iter().all(|arg| is_text_placeholder(arg)));
        assert_eq!(safe[9], "<redacted>");
        assert!(is_text_placeholder(&safe[10]));
        assert!(is_text_placeholder(&safe[11]));
        assert_eq!(redact_argv(&safe, &[]), safe);
    }

    #[test]
    fn cli_trace_persistence_fails_closed_for_invalid_content_calls() {
        let dir = tempfile::tempdir().unwrap();
        let recorder = &crate::host::flight_recorder::handle_cli_trace;
        crate::host::flight_recorder::adopt_session(dir.path(), "sess-grants");
        recorder(
            dir.path(),
            &json!({"id":uuid::Uuid::new_v4().to_string(),
            "phase":"exit", "exit_code":0, "pane_id":19,
            "argv":["plexi", "agent", "spawn", "--grant", "agents.ping=allow",
                "--token", "abc", "API_KEY=x", "--key", "private"]}),
        )
        .unwrap();
        let (rows, _) =
            crate::host::flight_recorder::read_session(dir.path(), "sess-grants").unwrap();
        let calls = crate::host::flight_recorder::cli_calls(rows, Some(19));
        let argv = &calls[0]["data"]["argv"];
        assert!(is_text_placeholder(argv[4].as_str().unwrap()));
        assert_eq!(argv[6], "<redacted>");
        assert!(is_text_placeholder(argv[7].as_str().unwrap()));
        assert_eq!(argv[9], "<redacted>");
    }

    #[test]
    fn cli_trace_valid_passthrough_retains_secret_redaction() {
        let argv = [
            "plexi",
            "run",
            "worker",
            "--token",
            "private",
            "API_KEY=value",
            "--custom-name",
            "hidden",
            "agents.ping=allow",
        ]
        .map(str::to_string);
        let safe = redact_argv(&argv, &["CUSTOM_NAME".into()]);
        assert_eq!(safe[4], "<redacted>");
        assert_eq!(safe[5], "API_KEY=<redacted>");
        assert_eq!(safe[7], "<redacted>");
        assert_eq!(safe[8], "agents.ping=allow");
        assert_eq!(redact_argv(&safe, &["CUSTOM_NAME".into()]), safe);
    }

    #[test]
    fn cli_trace_valid_grants_are_not_content() {
        let argv = [
            "plexi",
            "run",
            "reviewer",
            "--grant",
            "agents.ping=allow",
            "--grant=tool.name=ask",
        ]
        .map(str::to_string);
        assert_eq!(redact_argv(&argv, &[]), argv);
    }

    #[test]
    fn cli_trace_preserves_useful_argv() {
        let argv = ["plexi-alpha", "pane", "new", "--right"].map(str::to_string);
        assert_eq!(redact_argv(&argv, &[]), argv);
    }

    #[test]
    fn cli_trace_redacts_agent_hook_detail_in_both_flag_forms() {
        let sentinel = "Agent: SENTINEL_SECRET";
        let spaced = [
            "plexi", "agent", "report", "--state", "working", "--detail", sentinel,
        ]
        .map(str::to_string);
        let equals = [
            "plexi",
            "agent",
            "report",
            "--state=working",
            "--detail=Agent: SENTINEL_SECRET",
        ]
        .map(str::to_string);

        let spaced_safe = redact_argv(&spaced, &[]);
        assert_eq!(&spaced_safe[..6], &spaced[..6]);
        assert!(is_text_placeholder(&spaced_safe[6]));
        assert!(!spaced_safe.join(" ").contains("SENTINEL_SECRET"));

        let equals_safe = redact_argv(&equals, &[]);
        assert_eq!(equals_safe[..4], equals[..4]);
        assert!(equals_safe[4]
            .strip_prefix("--detail=")
            .is_some_and(is_text_placeholder));
        assert!(!equals_safe.join(" ").contains("SENTINEL_SECRET"));
    }

    #[test]
    fn cli_trace_grammar_sentinels() {
        let cases: &[&[&str]] = &[
            &[
                "--json",
                "--profile=alpha",
                "pane",
                "send",
                "42",
                "SENTINEL",
            ],
            &["agent", "--json", "ask", "SENTINEL"],
            &["agent", "ask", "--socket", "/tmp/host.sock", "SENTINEL"],
            &["agent", "ask", "--pane-id=42", "SENTINEL"],
            &[
                "agent",
                "report",
                "--state",
                "working",
                "--detail",
                "Agent: SENTINEL",
            ],
            &[
                "agent",
                "report",
                "--state=working",
                "--detail=Agent: SENTINEL",
            ],
            &["notify", "--title", "SENTINEL", "--body=SENTINEL"],
            &["assistant", "send", "--text=SENTINEL"],
            &["pane", "message", "--json", "send", "42", "-SENTINEL"],
            &["pane", "send", "42", "-SENTINEL"],
            &["pane", "send", "42", "--secret-SENTINEL"],
            &["pane", "result", "write", "SENTINEL"],
            &["pane", "result", "write", "42", "SENTINEL"],
            &["pane", "result", "--write=SENTINEL"],
            &["pane", "send", "42", "--", "--SENTINEL"],
            &["agent", "ask", "--", "-SENTINEL"],
            &["agent", "call", "reviewer", "--task=SENTINEL"],
            &[
                "agent",
                "team",
                "run",
                "consult-experts",
                "--question=SENTINEL",
                "--option",
                "SENTINEL",
                "--option=SENTINEL",
            ],
            &[
                "agent",
                "done",
                "--result=SENTINEL",
                "--summary",
                "SENTINEL",
            ],
            &["pane", "heartbeat", "42", "--text", "SENTINEL"],
            &[
                "needs-you",
                "ask",
                "--summary",
                "SENTINEL",
                "--choice=SENTINEL",
            ],
            &["needs-you", "answer", "id", "--answer=SENTINEL"],
            &["note", "add", "SENTINEL"],
            &["pane", "slot", "write", "summary", "SENTINEL"],
            &["pane", "send", "42", "--submit", "SENTINEL"],
            &["pane", "submit", "42", "SENTINEL"], // invalid legacy command
            &["agent", "call", "--unknown", "SENTINEL", "--task=-SENTINEL"],
            &["--unknown", "SENTINEL", "pane", "send", "42", "SENTINEL"],
            &["pane", "send", "42", "<text len=SENTINEL>"],
            &["pane", "send", "42", "SENTINEL", "unexpected"],
            &["pane", "send", "42", "SENTINEL_SECRET=value", "unexpected"],
            &[
                "pane",
                "send",
                "42",
                "--SENTINEL_SECRET=value",
                "unexpected",
            ],
        ];
        for case in cases {
            let argv: Vec<String> = std::iter::once("plexi")
                .chain(case.iter().copied())
                .map(str::to_string)
                .collect();
            let safe = redact_argv(&argv, &[]);
            assert!(!safe.join(" ").contains("SENTINEL"), "{case:?}: {safe:?}");
            assert_eq!(redact_argv(&safe, &[]), safe, "{case:?}");
        }
    }

    #[test]
    fn cli_trace_mapping_preserves_ids_flags_and_duplicate_values() {
        for argv in [
            vec!["plexi", "agent", "call", "same", "--task", "same"],
            vec!["plexi", "pane", "send", "same", "same", "--submit"],
            vec![
                "plexi", "agent", "--json", "ask", "--socket", "same", "same",
            ],
        ] {
            let argv: Vec<String> = argv.into_iter().map(str::to_string).collect();
            let safe = redact_argv(&argv, &[]);
            assert_eq!(
                safe.iter().filter(|arg| arg.as_str() == "same").count(),
                1,
                "{safe:?}"
            );
            assert_eq!(
                safe.iter().filter(|arg| is_text_placeholder(arg)).count(),
                1,
                "{safe:?}"
            );
            for (before, after) in argv.iter().zip(&safe) {
                if before != "same" {
                    assert_eq!(before, after);
                }
            }
        }
    }

    #[test]
    fn cli_trace_redacts_pane_and_agent_free_text() {
        let sentinel = "SENTINEL_SECRET_DO_NOT_PERSIST";
        let cases = vec![
            vec!["plexi", "pane", "send", "42", sentinel, "--submit"],
            vec![
                "plexi",
                "--socket",
                "/tmp/host.sock",
                "agent",
                "call",
                "reviewer",
                "--task",
                sentinel,
            ],
            vec!["plexi", "agent", "ask", "--pane-id", "42", sentinel],
            vec!["plexi", "pane", "send", "42", "<text len=SENTINEL_SECRET>"],
            vec![
                "plexi",
                "agent",
                "done",
                "--summary",
                sentinel,
                "--result",
                sentinel,
            ],
        ];
        for argv in cases {
            let argv = argv.into_iter().map(str::to_string).collect::<Vec<_>>();
            let safe = redact_argv(&argv, &[]);
            let rendered = safe.join(" ");
            assert!(!rendered.contains(sentinel), "{rendered}");
            assert!(rendered.contains("<text len="), "{rendered}");
            assert_eq!(redact_argv(&safe, &[]), safe, "redaction is idempotent");
        }
    }
}

use serde_json::json;
use sha2::{Digest, Sha256};
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
        || normalized == "KEY"
        || normalized.ends_with("_KEY")
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

fn secret_assignment(key: &str, known: &[String]) -> bool {
    let env_name = !key.is_empty()
        && key
            .chars()
            .enumerate()
            .all(|(i, c)| c == '_' || c.is_ascii_uppercase() || (i > 0 && c.is_ascii_digit()));
    env_name || secret_name(key, known)
}

fn text_placeholder(text: &str) -> String {
    if text == "<redacted>" || is_text_placeholder(text) {
        return text.to_string();
    }
    let hash = format!("{:x}", Sha256::digest(text.as_bytes()));
    format!("<text len={} sha256={}>", text.chars().count(), &hash[..8])
}

fn is_text_placeholder(text: &str) -> bool {
    let Some(body) = text
        .strip_prefix("<text len=")
        .and_then(|body| body.strip_suffix('>'))
    else {
        return false;
    };
    let Some((len, hash)) = body.split_once(" sha256=") else {
        return false;
    };
    !len.is_empty()
        && len.bytes().all(|byte| byte.is_ascii_digit())
        && hash.len() == 8
        && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// These are free-form payload IDs in the clap tree, not spellings or offsets.
fn content_id(id: &str) -> bool {
    matches!(
        id,
        "text"
            | "task"
            | "question"
            | "summary"
            | "result"
            | "prompt"
            | "persona"
            | "message"
            | "answer"
            | "choices"
            | "options"
            | "input"
            | "payload"
            | "receipt"
            | "description"
            | "content"
            // Hook reports carry tool commands and prompt-derived text here.
            | "detail"
            // Notifications are callable from agent and hook workflows.
            | "title"
            | "body"
    )
}

fn content_values(matches: &clap::ArgMatches) -> Vec<String> {
    content_values_at(matches, &[])
}

fn content_values_at(matches: &clap::ArgMatches, path: &[&str]) -> Vec<String> {
    let mut values = Vec::new();
    for id in matches.ids().filter(|id| {
        content_id(id.as_str())
            || (path == ["pane", "result"] && id.as_str() == "write")
            || (path == ["pane", "result", "write"]
                && (id.as_str() == "second"
                    || (id.as_str() == "first" && !matches.contains_id("second"))))
    }) {
        if matches.value_source(id.as_str()) == Some(clap::parser::ValueSource::CommandLine) {
            if let Some(raw) = matches.get_raw(id.as_str()) {
                values.extend(raw.map(|value| value.to_string_lossy().into_owned()));
            }
        }
    }
    if let Some((name, child)) = matches.subcommand() {
        let mut path = path.to_vec();
        path.push(name);
        values.extend(content_values_at(child, &path));
    }
    values
}

/// On an invalid invocation, retain only grammar-recognized flags and the
/// unambiguous command prefix. Unknown hyphen tokens might themselves be text.
fn fail_closed(command: &clap::Command, argv: &[String]) -> Vec<Option<String>> {
    fn flags(command: &clap::Command, out: &mut Vec<String>) {
        for arg in command.get_arguments() {
            if let Some(long) = arg.get_long() {
                out.push(format!("--{long}"));
            }
            if let Some(short) = arg.get_short() {
                out.push(format!("-{short}"));
            }
        }
        for child in command.get_subcommands() {
            flags(child, out);
        }
    }
    let mut known = vec!["--".to_string()];
    flags(command, &mut known);
    let mut node = command;
    let mut prefix = true;
    argv.iter()
        .enumerate()
        .map(|(index, arg)| {
            if index == 0 {
                return None;
            }
            if prefix {
                if let Some(child) = node.find_subcommand(arg) {
                    node = child;
                    return None;
                }
                // Once a token is ambiguous, do not guess further command positions.
                prefix = false;
            }
            if known.contains(arg) {
                return None;
            }
            if let Some((flag, value)) = arg.split_once('=') {
                if known.iter().any(|known| known == flag) {
                    return Some(format!("{flag}={}", text_placeholder(value)));
                }
            }
            Some(text_placeholder(arg))
        })
        .collect()
}

fn redact_content(argv: &[String]) -> (Vec<Option<String>>, bool) {
    use clap::CommandFactory;
    // main uses this same grammar through help::gated_command. Its additional
    // transforms only hide help and consult runtime profiles; tracing must not.
    let command = super::args::Cli::command();
    let Ok(matches) = command.clone().try_get_matches_from(argv) else {
        return (fail_closed(&command, argv), true);
    };
    let values = content_values(&matches);
    let mut replacements = vec![None; argv.len()];
    // Clap indices are logical indices (equals syntax and short clusters expand
    // them), not argv offsets. Reparse candidate substitutions to prove ownership
    // instead. This also disambiguates a payload equal to an id or command name.
    let mut marker = "plexi_trace_content_probe".to_string();
    while argv.iter().any(|arg| arg.contains(&marker)) {
        marker.push('_');
    }
    for value in values {
        let mut mapped = false;
        for (index, arg) in argv.iter().enumerate().skip(1) {
            let prefix = if arg == &value {
                ""
            } else if let Some((flag, rhs)) = arg.split_once('=') {
                if rhs != value || !flag.starts_with('-') {
                    continue;
                }
                &arg[..flag.len() + 1]
            } else {
                continue;
            };
            let mut probe = argv.to_vec();
            probe[index] = format!("{prefix}{marker}");
            if command
                .clone()
                .try_get_matches_from(&probe)
                .ok()
                .is_some_and(|parsed| content_values(&parsed).contains(&marker))
            {
                replacements[index] = Some(format!("{prefix}{}", text_placeholder(&value)));
                mapped = true;
            }
        }
        if !mapped {
            return (fail_closed(&command, argv), true);
        }
    }
    (replacements, false)
}

/// Redact before transport and again before persistence. Keep option names so
/// the sequence remains useful; mask uppercase env assignments, secret keys,
/// and command arguments that carry free-form user or agent text.
pub(crate) fn redact_argv(argv: &[String], known: &[String]) -> Vec<String> {
    let (content, fallback) = redact_content(argv);
    let mut hide_next = false;
    let secret_set = argv.windows(2).position(|pair| pair == ["secret", "set"]);
    argv.iter()
        .enumerate()
        .map(|(index, arg)| {
            if !fallback {
                if let Some(redacted) = &content[index] {
                    return redacted.clone();
                }
            }
            if hide_next {
                hide_next = false;
                return "<redacted>".to_string();
            }
            // The legacy positional value is redacted even if clap rejects it.
            if secret_set.is_some_and(|start| index > start + 2) && !arg.starts_with('-') {
                return "<redacted>".to_string();
            }
            if let Some((key, _)) = arg.split_once('=') {
                // In an invalid call, even an assignment's key can be payload.
                // Only retain an option prefix that the grammar recognized.
                if let Some(redacted) = &content[index] {
                    return redacted.clone();
                }
                if !key.contains(char::is_whitespace) && secret_assignment(key, known) {
                    return format!("{key}=<redacted>");
                }
            }
            if (arg.starts_with('-') && secret_name(arg, known))
                || known.iter().any(|name| name.eq_ignore_ascii_case(arg))
            {
                hide_next = true;
                return content[index].clone().unwrap_or_else(|| arg.clone());
            }
            if let Some(redacted) = &content[index] {
                return redacted.clone();
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
                regex::Regex::new(
                    r#"(^|[^A-Za-z0-9_.-])([A-Za-z_][A-Za-z0-9_.-]*)=(?:"[^"]*"|'[^']*'|[^\s]+)"#,
                )
                .expect("static assignment regex")
            });
            re.replace_all(arg, |caps: &regex::Captures<'_>| {
                let key = &caps[2];
                if secret_assignment(key, known) {
                    format!("{}{key}=<redacted>", &caps[1])
                } else {
                    caps[0].to_string()
                }
            })
            .into_owned()
        })
        .collect()
}

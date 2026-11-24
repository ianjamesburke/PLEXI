//! CLI invocation tracing over the host command transport.

#[cfg(test)]
mod tests {
    use super::*;

    fn redact_argv(argv: &[String], known: &[String]) -> Vec<String> {
        recorded_argv(argv, known, false)
    }

    /// The host's record of an argv the static grammar rejects.
    const UNPARSED: [&str; 2] = ["plexi", "<unparsed>"];

    /// A recorded argv is stable under recording again, except the fixed
    /// label for an unparsable one (which itself parses as a bare launch).
    fn assert_stable(safe: &[String], known: &[String]) {
        if safe != UNPARSED {
            assert_eq!(redact_argv(safe, known), safe);
        }
    }

    #[test]
    fn cli_trace_redacts_secret_values_and_assignments() {
        let argv = ["plexi", "secret", "set", "FOO", "bar"];
        assert!(!redact_argv(&argv.map(str::to_string), &[])
            .join(" ")
            .contains("bar"));
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
        // `pane new` declares no `--secret`: clap rejects it, so the host
        // records only the fixed label.
        assert_eq!(redacted, UNPARSED);
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
        let mut h = crate::testing::HostHarness::new();
        let pane = h.add_focused_terminal();
        let peer = crate::broker::caller::PeerWitness::for_live_pids(&[h.shell_pid(pane)]);
        let mailbox = h.ipc_tx.clone();
        let server = std::thread::spawn(move || {
            let wrong_profile = tempfile::tempdir().unwrap();
            let _profile = crate::config::set_test_profile_dir(wrong_profile.path().to_path_buf());
            crate::host::flight_recorder::adopt_session(&profile, "sess-socket");
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut line = String::new();
                std::io::BufReader::new(stream.try_clone().unwrap())
                    .read_line(&mut line)
                    .unwrap();
                crate::app::handle_socket_line_in(
                    &profile,
                    &line,
                    &mailbox,
                    Some(&peer),
                    &mut stream,
                );
            }
        });
        let id = uuid::Uuid::new_v4().to_string();
        // The frame claims pane 19. The kernel says otherwise.
        let mut frame = json!({"type":"cli_trace","id":id,"phase":"start","pane_id":19,
            "argv":["plexi-alpha","pane","new","--right"]});
        let client_socket = socket.clone();
        let client = std::thread::spawn(move || {
            assert!(send_frame(&client_socket, &frame));
            frame["phase"] = json!("exit");
            frame["exit_code"] = json!(0);
            assert!(send_frame(&client_socket, &frame));
        });
        while !client.is_finished() {
            h.hidden_frame();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        client.join().unwrap();
        server.join().unwrap();
        let (_, _, rows) =
            crate::cli::debug::build_show(dir.path(), Some("sess-socket"), None, None).unwrap();
        assert!(
            crate::host::flight_recorder::cli_calls(rows.clone(), Some(19)).is_empty(),
            "a claimed pane id is never recorded"
        );
        let calls = crate::host::flight_recorder::cli_calls(rows.clone(), Some(pane));
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
        assert_stable(&safe, &["CUSTOM_NAME".into()]);
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
        assert_eq!(safe, UNPARSED);
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
        assert_eq!(argv, &json!(UNPARSED));
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
            assert_stable(&safe, &[]);
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

    #[test]
    fn cli_trace_capture_opt_in_keeps_content_but_still_hides_secrets() {
        let args = ["plexi", "pane", "send", "42", "hello world"].map(str::to_string);
        assert_eq!(recorded_argv(&args, &[], true), args);
        let args = ["plexi", "pane", "send", "42", "API_KEY=private"].map(str::to_string);
        assert!(!recorded_argv(&args, &[], true)
            .join(" ")
            .contains("private"));
        let args = [
            "plexi",
            "agent",
            "call",
            "reviewer",
            "--task=API_KEY=private",
        ]
        .map(str::to_string);
        assert!(!recorded_argv(&args, &[], true)
            .join(" ")
            .contains("private"));
        let args = [
            "plexi",
            "agent",
            "call",
            "reviewer",
            "--task=--token=private",
        ]
        .map(str::to_string);
        assert!(!recorded_argv(&args, &[], true)
            .join(" ")
            .contains("private"));
        let invalid = ["plexi", "pane", "send", "42", "private", "extra"].map(str::to_string);
        assert!(!recorded_argv(&invalid, &[], true)
            .join(" ")
            .contains("private"));
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
/// so a review-only draft does not mutate the session it is reading. A
/// lifecycle command passes its validated host's socket, so its trace goes
/// only to the host it addresses, never to an explicit `--socket`.
pub fn begin(
    argv: &[String],
    owners: &[Option<super::front::Owner>],
    lifecycle_socket: Option<PathBuf>,
) {
    let socket = lifecycle_socket
        .or_else(super::resolve_command_socket)
        .unwrap_or_else(crate::platform::ipc::endpoint_path);
    let mut slot = active().lock().unwrap_or_else(|e| e.into_inner());
    if slot.is_some() {
        return;
    }
    let id = uuid::Uuid::new_v4().to_string();
    let frame = json!({
        "type": "cli_trace", "id": id, "phase": "start",
        "argv": redact_owned(
            argv,
            owners,
            &known_secret_names(&crate::config::config_dir()),
            capture_content(),
        ),
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

/// Free-form payload arguments, by command path and argument id.
fn is_content(path: &[String], id: &str, present: impl Fn(&str) -> bool) -> bool {
    let at = |want: &[&str]| path.iter().map(String::as_str).eq(want.iter().copied());
    content_id(id)
        || (at(&["pane", "result"]) && id == "write")
        || (at(&["pane", "result", "write"])
            && (id == "second" || (id == "first" && !present("second"))))
}

/// Whether an optional-value option was given bare. Clap reports a bare
/// option and an explicit value alike, so this reads only the shape of argv:
/// an option that requires `=` and has no short form carries a value exactly
/// when some token is one of its `--long=` spellings. Every other optional
/// value counts as given, which can only make its record a template.
fn bare_optional(arg: &clap::Arg, argv: &[String]) -> bool {
    let optional = arg
        .get_num_args()
        .is_some_and(|range| range.min_values() == 0);
    if !optional || !arg.is_require_equals_set() || arg.get_short().is_some() {
        return false;
    }
    let Some(long) = arg.get_long() else {
        return false;
    };
    let mut spellings = vec![long];
    spellings.extend(arg.get_all_aliases().unwrap_or_default());
    !argv.iter().any(|token| {
        spellings
            .iter()
            .any(|spelling| token.starts_with(&format!("--{spelling}=")))
    })
}

/// Marks a row whose argv `host_record` wrote as a grammar template. Rows
/// without it predate templates: their argv dropped values without a trace,
/// so a reader can never treat one as runnable.
pub(crate) const RECORD_FORMAT: &str = "grammar-template-1";

/// The host's record of a traced invocation. A trace frame can come from any
/// process, so nothing it sends is kept as written. Clap parses the argv with
/// the full static grammar (run requirements enforced; `--profile` declared so
/// a selector parses, and never recorded). A parse yields the label `plexi`,
/// the subcommand path, a placeholder for each positional given, then the
/// options given, each value-taking option followed by its placeholder
/// (`--opt=<id>` when the grammar requires `=`; bare when an optional value
/// was not given, see `bare_optional`). A
/// placeholder is the argument's grammar id (`<task>`, or `<args>...` for
/// several values), so the record is a template: no value, and nothing
/// forwarded to a child, is ever kept. An argv clap rejects yields only
/// `plexi <unparsed>`.
pub(crate) fn host_record(argv: &[String]) -> Vec<String> {
    use clap::CommandFactory;
    const LABEL: &str = "plexi";
    let mut command = super::args::Cli::command().arg(
        clap::Arg::new("profile")
            .long("profile")
            .global(true)
            .hide(true)
            .action(clap::ArgAction::Append),
    );
    command.build();
    let Ok(matches) = command.clone().try_get_matches_from(argv) else {
        return vec![LABEL.to_string(), "<unparsed>".to_string()];
    };
    let mut record = vec![LABEL.to_string()];
    let mut positionals: Vec<String> = Vec::new();
    let mut options: Vec<(String, Vec<String>)> = Vec::new();
    let mut node = &command;
    let mut level = Some(&matches);
    while let Some(m) = level {
        let mut given: Vec<(usize, String)> = Vec::new();
        for id in m.ids() {
            let id = id.as_str();
            if id == "profile" || m.value_source(id) != Some(clap::parser::ValueSource::CommandLine)
            {
                continue;
            }
            let Some(arg) = node.get_arguments().find(|arg| arg.get_id() == id) else {
                continue;
            };
            let values = m.try_get_raw(id).ok().flatten().map_or(0, |raw| raw.len());
            let placeholder = if values > 1 {
                format!("<{id}>...")
            } else {
                format!("<{id}>")
            };
            if let Some(index) = arg.get_index() {
                given.push((index, placeholder));
                continue;
            }
            let name = match (arg.get_long(), arg.get_short()) {
                (Some(long), _) => format!("--{long}"),
                (None, Some(short)) => format!("-{short}"),
                (None, None) => continue,
            };
            if options.iter().any(|(seen, _)| *seen == name) {
                continue;
            }
            let bare = bare_optional(arg, argv);
            let tokens = if !arg.get_action().takes_values() || values == 0 || bare {
                vec![name.clone()]
            } else if arg.is_require_equals_set() {
                vec![format!("{name}={placeholder}")]
            } else {
                vec![name.clone(), placeholder]
            };
            options.push((name, tokens));
        }
        given.sort();
        positionals.extend(given.into_iter().map(|(_, placeholder)| placeholder));
        level = m.subcommand().map(|(name, sub)| {
            record.push(name.to_string());
            node = node
                .find_subcommand(name)
                .expect("clap matched a declared subcommand");
            sub
        });
    }
    record.extend(positionals);
    record.extend(options.into_iter().flat_map(|(_, tokens)| tokens));
    record
}

/// Content values by this invocation's captured ownership.
fn redact_content(argv: &[String], owners: &[Option<super::front::Owner>]) -> Vec<Option<String>> {
    argv.iter()
        .zip(owners)
        .map(|(token, owner)| {
            let owner = owner.as_ref()?;
            let present = |id: &str| {
                owners
                    .iter()
                    .flatten()
                    .any(|other| other.path == owner.path && other.id == id)
            };
            is_content(&owner.path, &owner.id, present).then(|| {
                let (prefix, value) = token.split_at(owner.prefix);
                format!("{prefix}{}", text_placeholder(value))
            })
        })
        .collect()
}

/// Redact an invocation's argv by its captured ownership before transport.
/// Keep option names so the sequence remains useful; mask uppercase env
/// assignments, secret keys, and arguments that carry free-form text.
pub(crate) fn redact_owned(
    argv: &[String],
    owners: &[Option<super::front::Owner>],
    known: &[String],
    capture: bool,
) -> Vec<String> {
    if owners.len() != argv.len() {
        log::warn!("cli_trace: ownership does not match argv; masking every value");
        return host_record(argv);
    }
    let content = redact_content(argv, owners);
    redact_owned_tail(argv, content, known, capture)
}

/// What a trace records for `argv`: an invocation that runs is redacted by
/// its captured ownership on the CLI side; anything else never traces locally
/// and is masked when it arrives from another process.
#[cfg(test)]
pub(crate) fn recorded_argv(argv: &[String], known: &[String], capture: bool) -> Vec<String> {
    let os: Vec<std::ffi::OsString> = argv.iter().map(Into::into).collect();
    match super::front::parse(&os).outcome {
        super::front::Outcome::Run(run) => redact_owned(&run.tokens, &run.owners, known, capture),
        _ => host_record(argv),
    }
}

fn capture_content() -> bool {
    std::env::var("PLEXI_TRACE_CAPTURE_CONTENT").is_ok_and(|value| value == "1")
}

fn redact_owned_tail(
    argv: &[String],
    content: Vec<Option<String>>,
    known: &[String],
    capture: bool,
) -> Vec<String> {
    let mut hide_next = false;
    let secret_set = argv.windows(2).position(|pair| pair == ["secret", "set"]);
    argv.iter()
        .enumerate()
        .map(|(index, arg)| {
            if !capture {
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
            if let Some((key, value)) = arg.split_once('=') {
                if !key.contains(char::is_whitespace) && secret_assignment(key, known) {
                    return format!("{key}=<redacted>");
                }
                if let Some((nested, _)) = value.split_once('=') {
                    if nested.starts_with('-') && secret_name(nested, known) {
                        return format!("{key}={nested}=<redacted>");
                    }
                }
            }
            if (arg.starts_with('-') && secret_name(arg, known))
                || known.iter().any(|name| name.eq_ignore_ascii_case(arg))
            {
                hide_next = true;
                return content[index].clone().unwrap_or_else(|| arg.clone());
            }
            if arg.starts_with("--") && !arg.contains('=') && !arg.contains(char::is_whitespace) {
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

//! `plexi file open <path>` and its shorthand `plexi open <path>`.
//!
//! Both spellings build the same `HostRequest::FileOpen`: the path exactly as
//! typed, this process's working directory, and the caller's claimed pane. The
//! host resolves, classifies, places and answers (`src/app/file_open.rs`).

use super::args::FileOpenArgs;
use crate::protocol::HostRequest;

/// The request both spellings send. `response_file` is filled in by the
/// transport.
fn file_open_request(
    args: &FileOpenArgs,
    from_pane_id: Option<u64>,
    cwd: Option<String>,
    caller_pane_id: Option<u64>,
) -> HostRequest {
    HostRequest::FileOpen {
        path: args.path.clone(),
        cwd,
        caller_pane_id,
        from_pane_id,
        placement: args.placement(),
        no_focus: args.no_focus,
        peer: None,
        response_file: String::new(),
    }
}

/// What to print and the exit code for one host reply.
#[derive(Debug, PartialEq, Eq)]
struct Outcome {
    stdout: Option<String>,
    stderr: Option<String>,
    exit_code: i32,
}

fn outcome(reply: &str, json: bool) -> Outcome {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(reply) else {
        return Outcome {
            stdout: None,
            stderr: Some(format!("error: unreadable file open reply: {reply}")),
            exit_code: 1,
        };
    };
    let printed_json = json.then(|| value.to_string());
    if let Some(error) = value.get("error").and_then(serde_json::Value::as_str) {
        let code = value
            .get("error_code")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("error");
        return Outcome {
            stdout: printed_json,
            stderr: Some(format!("error: {error} ({code})")),
            exit_code: 1,
        };
    }
    let pane_id = value.get("pane_id").and_then(serde_json::Value::as_u64);
    Outcome {
        stdout: printed_json.or_else(|| pane_id.map(|id| id.to_string())),
        stderr: pane_id
            .is_none()
            .then(|| "opened with the system default application".to_string())
            .filter(|_| !json),
        exit_code: 0,
    }
}

pub fn file_open_cli(args: &FileOpenArgs, from_pane_id: Option<u64>, json: bool) -> i32 {
    let cwd = match std::env::current_dir() {
        Ok(cwd) => match cwd.into_os_string().into_string() {
            Ok(cwd) => Some(cwd),
            Err(raw) => return print_outcome(&unrepresentable_cwd(&raw, json)),
        },
        Err(error) => {
            log::warn!("file_open:cli: current directory unreadable: {error}");
            None
        }
    };
    let caller_pane_id = std::env::var("PLEXI_PANE_ID")
        .ok()
        .and_then(|id| id.parse().ok());
    let request = file_open_request(args, from_pane_id, cwd, caller_pane_id);
    let payload = match serde_json::to_value(&request) {
        Ok(payload) => payload,
        Err(error) => {
            eprintln!("error: internal: file open request did not serialize: {error}");
            return 1;
        }
    };
    log::info!(
        "file_open:cli: path={:?} caller_pane_id={caller_pane_id:?} from={from_pane_id:?} placement={:?} no_focus={}",
        args.path,
        args.placement(),
        args.no_focus
    );
    let reply = match super::request(payload, "file-open-response", "file open") {
        Ok(reply) => reply,
        Err(code) => return code,
    };
    print_outcome(&outcome(&reply, json))
}

fn print_outcome(outcome: &Outcome) -> i32 {
    if let Some(line) = &outcome.stderr {
        eprintln!("{line}");
    }
    if let Some(line) = &outcome.stdout {
        println!("{line}");
    }
    outcome.exit_code
}

/// The refusal when this process's working directory has no exact UTF-8
/// form: a lossy spelling could resolve PATH against another folder. It is
/// the host's refusal shape, so `--json` callers branch on `error_code`.
fn unrepresentable_cwd(raw: &std::ffi::OsStr, json: bool) -> Outcome {
    let error = format!(
        "the current directory {raw:?} is not valid UTF-8, so it cannot be sent exactly; \
         pass an absolute PATH from a UTF-8 directory"
    );
    let reply = serde_json::json!({"error": error, "error_code": "unrepresentable_path"});
    outcome(&reply.to_string(), json)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::args::{Cli, Commands, FileCmd};
    use clap::Parser;

    fn parse(argv: &[&str]) -> Result<FileOpenArgs, clap::Error> {
        let cli = Cli::try_parse_from(argv)?;
        Ok(match cli.command {
            Some(Commands::Open(args))
            | Some(Commands::File {
                cmd: FileCmd::Open(args),
            }) => args,
            _ => panic!("{argv:?} is not a file open"),
        })
    }

    fn wire(args: &FileOpenArgs) -> serde_json::Value {
        serde_json::to_value(file_open_request(
            args,
            Some(7),
            Some("/work".into()),
            Some(3),
        ))
        .unwrap()
    }

    #[test]
    fn both_spellings_send_the_same_request() {
        for flags in [
            vec![],
            vec!["--down", "--no-focus"],
            vec!["--split", "tab"],
            vec!["--window"],
        ] {
            let short: Vec<&str> = ["plexi", "open", "README.md"]
                .into_iter()
                .chain(flags.iter().copied())
                .collect();
            let long: Vec<&str> = ["plexi", "file", "open", "README.md"]
                .into_iter()
                .chain(flags.iter().copied())
                .collect();
            let (short, long) = (parse(&short).unwrap(), parse(&long).unwrap());
            assert_eq!(short, long);
            assert_eq!(wire(&short), wire(&long));
        }
        let request = wire(&parse(&["plexi", "open", "README.md", "-d"]).unwrap());
        assert_eq!(
            request,
            serde_json::json!({
                "type": "file_open",
                "path": "README.md",
                "cwd": "/work",
                "caller_pane_id": 3,
                "from_pane_id": 7,
                "placement": "down",
                "response_file": "",
            })
        );
    }

    #[test]
    fn the_path_is_sent_literally() {
        assert_eq!(parse(&["plexi", "open", "git"]).unwrap().path, "git");
        assert_eq!(
            parse(&["plexi", "open", "--", "-notes.txt"]).unwrap().path,
            "-notes.txt"
        );
        assert!(parse(&["plexi", "open", "-notes.txt"]).is_err());
        assert!(parse(&["plexi", "open", "a", "--left", "--tab"]).is_err());
        assert!(parse(&["plexi", "open", "a", "--left", "--split", "up"]).is_err());
        assert_eq!(parse(&["plexi", "open", "a"]).unwrap().placement(), None);
    }

    #[test]
    fn plain_output_is_the_pane_id_and_json_is_the_receipt() {
        let receipt = r#"{"status":"opened","path":"/w/justfile","handler":"text-editor","matched_by":"content","context_id":4,"pane_id":27,"placement":"right","focused":true}"#;
        assert_eq!(
            outcome(receipt, false),
            Outcome {
                stdout: Some("27".into()),
                stderr: None,
                exit_code: 0
            }
        );
        let json = outcome(receipt, true);
        assert_eq!(json.exit_code, 0);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(json.stdout.as_deref().unwrap()).unwrap(),
            serde_json::from_str::<serde_json::Value>(receipt).unwrap()
        );

        let refusal = r#"{"error":"/w/x does not exist","error_code":"not_found"}"#;
        assert_eq!(
            outcome(refusal, false),
            Outcome {
                stdout: None,
                stderr: Some("error: /w/x does not exist (not_found)".into()),
                exit_code: 1
            }
        );
        let json = outcome(refusal, true);
        assert_eq!(json.exit_code, 1);
        assert!(json
            .stdout
            .unwrap()
            .contains("\"error_code\":\"not_found\""));
    }

    #[test]
    fn the_cli_sends_its_cwd_and_prints_the_host_receipt() {
        use std::io::{BufRead, BufReader};
        let env = crate::cli::test_env::socket_env_guard();
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("open.sock");
        let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        env.set(&socket);
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(&stream).read_line(&mut line).unwrap();
            let payload: serde_json::Value = serde_json::from_str(&line).unwrap();
            let file = payload["response_file"].as_str().unwrap().to_string();
            assert!(crate::rpc::write_response(
                &file,
                br#"{"status":"opened","pane_id":5}"#
            ));
            payload
        });
        let args = parse(&["plexi", "open", "notes/today.md"]).unwrap();
        assert_eq!(file_open_cli(&args, None, false), 0);
        let payload = server.join().unwrap();
        assert_eq!(payload["type"], "file_open");
        assert_eq!(payload["path"], "notes/today.md");
        assert_eq!(
            payload["cwd"],
            std::env::current_dir().unwrap().display().to_string()
        );
    }

    #[test]
    fn a_non_utf8_cwd_is_a_typed_refusal_and_json_on_stdout() {
        use std::os::unix::ffi::OsStrExt;
        let raw = std::ffi::OsStr::from_bytes(b"/tmp/caf\xe9");
        let json = unrepresentable_cwd(raw, true);
        assert_eq!(json.exit_code, 1);
        let reply: serde_json::Value =
            serde_json::from_str(json.stdout.as_deref().expect("json on stdout")).unwrap();
        assert_eq!(reply["error_code"], "unrepresentable_path");
        let plain = unrepresentable_cwd(raw, false);
        assert_eq!((plain.stdout, plain.exit_code), (None, 1));
        assert!(plain.stderr.unwrap().contains("unrepresentable_path"));
    }
}

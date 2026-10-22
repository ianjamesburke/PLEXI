//! Review-only drafts derived from the durable CLI trace, never installed.
use crate::host::flight_recorder as recorder;
use serde_json::Value;
use std::io::Write;
use std::path::Path;

#[derive(Clone, Copy, Default)]
pub struct DraftOptions {
    pub pane: Option<u64>,
    pub include_internal: bool,
    pub include_failed: bool,
}

pub fn resolve_pane(pane: Option<u64>, all_panes: bool, caller: Option<&str>) -> Option<u64> {
    if all_panes {
        None
    } else {
        pane.or_else(|| caller.and_then(|s| s.parse().ok()))
    }
}

pub fn skill_draft_cli(
    session: &str,
    options: DraftOptions,
    format: &str,
    out: Option<&Path>,
) -> i32 {
    let result = build_draft(&crate::config::config_dir(), session, options, format)
        .and_then(|draft| write_draft(&draft, out, &mut std::io::stdout().lock()));
    match result {
        Ok(()) => {
            log::info!("skill draft: session={session} pane={:?} include_internal={} include_failed={} format={format} out={out:?}", options.pane, options.include_internal, options.include_failed);
            0
        }
        Err(error) => {
            log::warn!("skill draft: {error}");
            eprintln!("error: {error}");
            1
        }
    }
}

fn write_draft(draft: &str, out: Option<&Path>, stdout: &mut impl Write) -> Result<(), String> {
    match out {
        Some(path) => {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
                .map_err(|error| format!("cannot create {}: {error}", path.display()))?;
            file.write_all(draft.as_bytes())
                .map_err(|error| format!("cannot write {}: {error}", path.display()))
        }
        None => stdout
            .write_all(draft.as_bytes())
            .map_err(|error| format!("cannot write stdout: {error}")),
    }
}

fn quote(arg: &str) -> String {
    if !arg.is_empty()
        && arg
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_./:-".contains(c))
    {
        arg.to_string()
    } else {
        format!("'{}'", arg.replace('\'', "'\"'\"'"))
    }
}

pub(crate) fn shell_command(argv: &[String]) -> String {
    argv.iter()
        .enumerate()
        .map(|(i, arg)| {
            let arg = if i == 0 {
                Path::new(arg)
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or(arg)
            } else {
                arg
            };
            quote(arg)
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn internal_call(row: &Value) -> bool {
    let Some(argv) = row["data"]["argv"].as_array() else {
        return false;
    };
    // Only the command prefix counts; never match words inside payload arguments.
    let mut words = argv.iter().skip(1).filter_map(Value::as_str);
    let mut args = Vec::new();
    while let Some(word) = words.next() {
        match word {
            "--json" => continue,
            "--socket" | "--profile" => {
                words.next();
                continue;
            }
            _ if word.starts_with("--socket=") || word.starts_with("--profile=") => continue,
            _ => args.push(word),
        }
        if args.len() == 3 {
            break;
        }
    }
    matches!(
        args.as_slice(),
        ["agent", "done" | "report", ..]
            | ["pane", "capture" | "list" | "wait", ..]
            | ["pane", "slot", "wait", ..]
    )
}

fn command_for(row: &Value) -> Result<(String, bool), String> {
    let mut argv: Vec<String> = serde_json::from_value(row["data"]["argv"].clone())
        .map_err(|error| format!("invalid captured argv: {error}"))?;
    if argv.is_empty() {
        return Err("empty captured argv".into());
    }
    let spawn = argv.windows(2).any(|pair| pair == ["agent", "spawn"]);
    let suggest = spawn
        && !argv
            .iter()
            .any(|arg| arg == "--subcontext" || arg.starts_with("--subcontext="));
    if suggest {
        let position = argv
            .iter()
            .position(|arg| arg == "--")
            .unwrap_or(argv.len());
        argv.splice(
            position..position,
            ["--subcontext".into(), "review-me".into()],
        );
    }
    Ok((shell_command(&argv), suggest))
}

fn build_draft(
    profile: &Path,
    session: &str,
    options: DraftOptions,
    format: &str,
) -> Result<String, String> {
    let id = recorder::resolve_session(profile, session)?;
    let (rows, skipped) = recorder::read_session(profile, &id)?;
    if skipped > 0 {
        return Err(format!(
            "session {id} has {skipped} unreadable rows; repair it before drafting"
        ));
    }
    let pane = options.pane;
    let calls = recorder::cli_calls(rows, pane);
    if calls.is_empty() {
        return Err(format!("no captured CLI calls in {id} for pane {pane:?}"));
    }
    let mut steps = Vec::new();
    let mut notes = Vec::new();
    let mut groups: Vec<(Value, usize)> = Vec::new();
    for row in calls {
        if (!options.include_internal && internal_call(&row))
            || (!options.include_failed && row["data"]["exit_code"].as_i64() != Some(0))
        {
            continue;
        }
        if let Some((last, count)) = groups.last_mut() {
            if last["data"]["argv"] == row["data"]["argv"]
                && last["data"]["exit_code"] == row["data"]["exit_code"]
            {
                *count += 1;
                continue;
            }
        }
        groups.push((row, 1));
    }
    for (row, count) in groups {
        let (command, suggested) = command_for(&row)?;
        let repeat = if count > 1 {
            format!(" # repeated {count}x")
        } else {
            String::new()
        };
        let status = row["data"]["exit_code"].as_i64();
        if suggested {
            notes.push(
                "Suggested --subcontext review-me for agent spawn; choose a task-specific name."
                    .to_string(),
            );
        }
        let withheld = status != Some(0) || command.contains("<redacted>") || command.contains("<text len=");
        if withheld {
            notes.push(format!(
                "Review-only call (exit={}): {command}{repeat}",
                status
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "unknown".into())
            ));
        } else {
            if !repeat.is_empty() {
                notes.push(format!("{command}{repeat}"));
            }
            steps.push(command);
        }
    }
    match format {
        "command" => {
            let body = toml::toml! {
                [commands.session_draft]
                description = "Draft from a recorded session; review before installing"
                steps = steps
            };
            let mut output = "# Draft only. Review paths, pane ids and suggested subcontext names.\n# Failed, incomplete and redacted calls are comments, never executable steps.\n".to_string();
            for note in notes {
                for line in note.lines() {
                    output.push_str(&format!("# {line}\n"));
                }
            }
            output.push_str(&toml::to_string_pretty(&body).map_err(|error| error.to_string())?);
            Ok(output)
        }
        "skill" => {
            let mut output = "---\nname: session-draft\ndescription: Reviewed workflow drafted from a Plexi CLI session\n---\n\n# Session workflow draft\n\nReview paths, pane ids and subcontext names before accepting. Replace redacted values through the secret workflow. Nothing has been installed.\n\n".to_string();
            for note in notes {
                for line in note.lines() {
                    output.push_str(&format!("    # {line}\n"));
                }
                output.push('\n');
            }
            output.push_str("## Sequence to review\n\n");
            for step in steps {
                for line in step.lines() {
                    output.push_str(&format!("    {line}\n"));
                }
            }
            Ok(output)
        }
        _ => Err(format!("unknown draft format: {format}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn cli_trace_draft_suggests_subcontext_and_writes_only_when_requested() {
        let dir = tempfile::tempdir().unwrap();
        recorder::adopt_session(dir.path(), "sess-draft");
        let id = uuid::Uuid::new_v4().to_string();
        recorder::handle_cli_trace(
            dir.path(),
            &json!({"id":id,"phase":"start","pane_id":19,
            "argv":["plexi-alpha","agent","spawn","researcher","--task","Audit auth's $(danger)"]}),
        )
        .unwrap();
        recorder::handle_cli_trace(dir.path(), &json!({"id":id,"phase":"exit","exit_code":0,"pane_id":19,"argv":["plexi-alpha","agent","spawn","researcher","--task","Audit auth's $(danger)"]}))
            .unwrap();
        let before = std::fs::read_dir(dir.path()).unwrap().count();
        let draft = build_draft(
            dir.path(),
            "sess-draft",
            DraftOptions {
                pane: Some(19),
                ..Default::default()
            },
            "skill",
        )
        .unwrap();
        assert!(draft.starts_with("---\nname:"));
        assert!(draft.contains("--subcontext review-me"));
        let mut stdout = Vec::new();
        write_draft(&draft, None, &mut stdout).unwrap();
        assert_eq!(stdout, draft.as_bytes());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), before);
        assert!(!dir.path().join("skills").exists());
        assert!(!dir.path().join("commands.toml").exists());
        let command =
            build_draft(dir.path(), "sess-draft", DraftOptions::default(), "command").unwrap();
        let parsed: toml::Value = toml::from_str(&command).unwrap();
        assert!(parsed["commands"]["session_draft"]["steps"][0]
            .as_str()
            .unwrap()
            .contains("--subcontext review-me"));
        let path = dir.path().join("draft.md");
        write_draft(&draft, Some(&path), &mut stdout).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), draft);
        assert!(write_draft("overwrite", Some(&path), &mut stdout).is_err());
        assert!(build_draft(
            dir.path(),
            "sess-draft",
            DraftOptions {
                pane: Some(20),
                ..Default::default()
            },
            "skill"
        )
        .is_err());
    }

    #[test]
    fn internal_calls_allow_global_flags_without_matching_payloads() {
        for argv in [
            vec!["plexi", "--json", "pane", "list"],
            vec!["plexi", "--socket", "/tmp/socket", "agent", "report"],
            vec!["plexi", "pane", "--profile=alpha", "slot", "wait"],
        ] {
            assert!(internal_call(&json!({"data":{"argv":argv}})));
        }
        assert!(!internal_call(
            &json!({"data":{"argv":["plexi", "pane", "send", "19", "agent", "done"]}})
        ));
    }

    #[test]
    fn draft_scope_filters_and_repeat_notes() {
        let dir = tempfile::tempdir().unwrap();
        recorder::adopt_session(dir.path(), "sess-filter");
        let record = |pane, argv: &[&str], status| {
            recorder::handle_cli_trace(
                dir.path(),
                &json!({"id":uuid::Uuid::new_v4().to_string(),
                "phase":"exit", "pane_id":pane, "argv":argv, "exit_code":status}),
            )
            .unwrap();
        };
        record(20, &["plexi", "run", "other-pane"], 0);
        for args in [
            vec!["plexi", "agent", "done"],
            vec!["plexi", "agent", "report"],
            vec!["plexi", "pane", "list"],
            vec!["plexi", "pane", "capture"],
            vec!["plexi", "pane", "wait"],
            vec!["plexi", "pane", "slot", "wait"],
        ] {
            record(19, &args, 0);
        }
        recorder::handle_cli_trace(
            dir.path(),
            &json!({"id":uuid::Uuid::new_v4().to_string(),
            "phase":"start", "pane_id":19, "argv":["plexi", "run", "incomplete"]}),
        )
        .unwrap();
        for _ in 0..4 {
            record(19, &["plexi", "run", "hi"], 1);
        }
        for _ in 0..3 {
            record(19, &["plexi", "pane", "new", "--right"], 0);
        }
        let options = DraftOptions {
            pane: resolve_pane(None, false, Some("19")),
            ..Default::default()
        };
        let draft = build_draft(dir.path(), "latest", options, "command").unwrap();
        let parsed: toml::Value = toml::from_str(&draft).unwrap();
        assert_eq!(
            parsed["commands"]["session_draft"]["steps"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert!(!draft.contains("other-pane"));
        assert!(!draft.contains("run hi"));
        assert!(!draft.contains("run incomplete"));
        assert!(!draft.contains("agent done"));
        assert!(!draft.contains("pane slot wait"));
        assert!(draft.contains("# repeated 3x"));
        let inclusive = build_draft(
            dir.path(),
            "latest",
            DraftOptions {
                include_internal: true,
                include_failed: true,
                ..options
            },
            "skill",
        )
        .unwrap();
        assert!(inclusive.contains("Review-only call (exit=unknown): plexi run incomplete"));
        assert!(inclusive.contains("agent done"));
        assert!(inclusive.contains("pane slot wait"));
        assert_eq!(
            inclusive
                .matches("Review-only call (exit=1): plexi run hi # repeated 4x")
                .count(),
            1
        );
        assert_eq!(resolve_pane(Some(20), false, Some("19")), Some(20));
        assert_eq!(resolve_pane(None, true, Some("19")), None);
        assert_eq!(resolve_pane(None, false, None), None);
        assert_eq!(resolve_pane(None, false, Some("invalid")), None);
        let all = build_draft(dir.path(), "latest", DraftOptions::default(), "skill").unwrap();
        assert!(all.contains("other-pane"));
    }

    #[test]
    fn cli_trace_draft_preserves_existing_subcontext_and_quotes_shell_text() {
        let row = json!({"data":{"argv":["plexi","agent","spawn","researcher","--task","a'b; $(oops)","--subcontext=auth"]}});
        let (command, suggested) = command_for(&row).unwrap();
        assert!(!suggested);
        assert!(command.contains("'a'\"'\"'b; $(oops)'"));
        assert!(!command.contains("review-me"));
    }

    #[test]
    fn cli_trace_draft_withholds_content_redacted_calls() {
        let dir = tempfile::tempdir().unwrap();
        recorder::adopt_session(dir.path(), "sess-content-redacted");
        recorder::handle_cli_trace(
            dir.path(),
            &json!({"id":uuid::Uuid::new_v4().to_string(), "phase":"exit", "exit_code":0,
            "argv":["plexi", "pane", "send", "19", "<text len=9 sha256=deadbeef>"]}),
        )
        .unwrap();
        let draft = build_draft(dir.path(), "sess-content-redacted", DraftOptions::default(), "command").unwrap();
        let parsed: toml::Value = toml::from_str(&draft).unwrap();
        assert!(parsed["commands"]["session_draft"]["steps"].as_array().unwrap().is_empty());
        assert!(draft.contains("Review-only call"));
    }
}

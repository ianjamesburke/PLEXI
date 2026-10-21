//! Review-only drafts derived from the durable CLI trace, never installed.
use crate::host::flight_recorder as recorder;
use serde_json::Value;
use std::io::Write;
use std::path::Path;

pub fn skill_draft_cli(session: &str, pane: Option<u64>, format: &str, out: Option<&Path>) -> i32 {
    let result = build_draft(&crate::config::config_dir(), session, pane, format)
        .and_then(|draft| write_draft(&draft, out, &mut std::io::stdout().lock()));
    match result {
        Ok(()) => {
            log::info!("skill draft: session={session} pane={pane:?} format={format} out={out:?}");
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

fn command_for(row: &Value) -> Result<(String, bool), String> {
    let mut argv: Vec<String> = serde_json::from_value(row["data"]["argv"].clone())
        .map_err(|error| format!("invalid captured argv: {error}"))?;
    if argv.is_empty() {
        return Err("empty captured argv".into());
    }
    // Keep the channel identity, but do not bake a build-machine binary path in.
    if let Some(name) = Path::new(&argv[0]).file_name().and_then(|s| s.to_str()) {
        argv[0] = name.to_string();
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
    Ok((
        argv.iter()
            .map(|arg| quote(arg))
            .collect::<Vec<_>>()
            .join(" "),
        suggest,
    ))
}

fn build_draft(
    profile: &Path,
    session: &str,
    pane: Option<u64>,
    format: &str,
) -> Result<String, String> {
    let id = recorder::resolve_session(profile, session)?;
    let (rows, skipped) = recorder::read_session(profile, &id)?;
    if skipped > 0 {
        return Err(format!(
            "session {id} has {skipped} unreadable rows; repair it before drafting"
        ));
    }
    let calls = recorder::cli_calls(rows, pane);
    if calls.is_empty() {
        return Err(format!("no captured CLI calls in {id} for pane {pane:?}"));
    }
    let mut steps = Vec::new();
    let mut notes = Vec::new();
    for row in calls {
        let (command, suggested) = command_for(&row)?;
        let status = row["data"]["exit_code"].as_i64();
        if suggested {
            notes.push(
                "Suggested --subcontext review-me for agent spawn; choose a task-specific name."
                    .to_string(),
            );
        }
        let withheld = status != Some(0) || command.contains("<redacted>");
        if withheld {
            notes.push(format!(
                "Review-only call (exit={}): {command}",
                status
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "unknown".into())
            ));
        } else {
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
        let draft = build_draft(dir.path(), "sess-draft", Some(19), "skill").unwrap();
        assert!(draft.starts_with("---\nname:"));
        assert!(draft.contains("--subcontext review-me"));
        let mut stdout = Vec::new();
        write_draft(&draft, None, &mut stdout).unwrap();
        assert_eq!(stdout, draft.as_bytes());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), before);
        assert!(!dir.path().join("skills").exists());
        assert!(!dir.path().join("commands.toml").exists());
        let command = build_draft(dir.path(), "sess-draft", None, "command").unwrap();
        let parsed: toml::Value = toml::from_str(&command).unwrap();
        assert!(parsed["commands"]["session_draft"]["steps"][0]
            .as_str()
            .unwrap()
            .contains("--subcontext review-me"));
        let path = dir.path().join("draft.md");
        write_draft(&draft, Some(&path), &mut stdout).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), draft);
        assert!(write_draft("overwrite", Some(&path), &mut stdout).is_err());
        assert!(build_draft(dir.path(), "sess-draft", Some(20), "skill").is_err());
    }

    #[test]
    fn cli_trace_draft_preserves_existing_subcontext_and_quotes_shell_text() {
        let row = json!({"data":{"argv":["plexi","agent","spawn","researcher","--task","a'b; $(oops)","--subcontext=auth"]}});
        let (command, suggested) = command_for(&row).unwrap();
        assert!(!suggested);
        assert!(command.contains("'a'\"'\"'b; $(oops)'"));
        assert!(!command.contains("review-me"));
    }
}

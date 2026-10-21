//! `plexi debug session list|show` — read the durable per-session flight trace
//! (`<profile>/sessions/<sess-id>.jsonl`, see `host::flight_recorder`).
//!
//! Neither verb talks to the host: the trace is a file, so a session can be
//! inspected after the host has exited. `show --join-agents` additionally reads
//! the external agent transcripts named by recorded `Provenance.session_id`
//! values (`~/.pi/agent/sessions`, `~/.claude/projects`) and interleaves their
//! prompts and tool calls with the host's permission and app events by time.

use crate::host::flight_recorder as recorder;
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use std::path::PathBuf;

/// Longest agent/tool text shown on one timeline line.
const LINE_TEXT_MAX: usize = 160;

/// `plexi debug session list [--json]`.
pub fn debug_session_list_cli(json: bool) -> i32 {
    let profile = crate::config::config_dir();
    let mut summaries = Vec::new();
    for id in recorder::list_session_ids(&profile) {
        match recorder::read_session(&profile, &id) {
            Ok((rows, skipped)) => {
                if skipped > 0 {
                    log::warn!("debug session list: {id}: skipped {skipped} unparseable line(s)");
                }
                summaries.push(recorder::summarize(&id, &rows));
            }
            Err(error) => log::warn!("debug session list: {error}"),
        }
    }
    log::info!("debug session list: {} session(s) in {}", summaries.len(), profile.display());
    if json {
        println!("{}", json!({ "sessions": summaries }));
        return 0;
    }
    if summaries.is_empty() {
        println!("(no recorded sessions)");
        return 0;
    }
    for s in &summaries {
        println!(
            "{}  {} → {}  records={} contexts={} panes={} agents={}",
            s["session_id"].as_str().unwrap_or("?"),
            s["started"].as_str().unwrap_or("?"),
            s["last"].as_str().unwrap_or("?"),
            s["records"],
            join_nums(&s["contexts"]),
            join_nums(&s["panes"]),
            join_strs(&s["agent_sessions"]),
        );
    }
    0
}

/// Build the `show` result: the session's host rows (optionally one pane's),
/// interleaved with agent transcript entries when `agent_roots` is given.
/// Returns `(session_id, skipped_lines, timeline)`.
pub(crate) fn build_show(
    profile: &std::path::Path,
    session: Option<&str>,
    pane: Option<u64>,
    agent_roots: Option<&[PathBuf]>,
) -> Result<(String, usize, Vec<Value>), String> {
    let session_id = recorder::resolve_session(profile, session.unwrap_or("latest"))?;
    let (rows, skipped) = recorder::read_session(profile, &session_id)?;
    let rows = filter_pane(rows, pane);
    let agents = match agent_roots {
        Some(roots) => agent_entries(&rows, roots),
        None => Vec::new(),
    };
    Ok((session_id, skipped, merge_timeline(rows, agents)))
}

/// `plexi debug session show [<sess-id>|latest] [--pane N] [--join-agents] [--json]`.
pub fn debug_session_show_cli(
    session: Option<&str>,
    pane: Option<u64>,
    join_agents: bool,
    cli: bool,
    json: bool,
) -> i32 {
    let profile = crate::config::config_dir();
    let roots = agent_roots();
    let (session_id, skipped, timeline) =
        match build_show(&profile, session, pane, join_agents.then_some(roots.as_slice())) {
            Ok(built) => built,
            Err(error) => {
                eprintln!("error: {error}");
                return 1;
            }
        };
    let timeline = if cli { recorder::cli_calls(timeline, pane) } else { timeline };
    if skipped > 0 {
        eprintln!("warning: skipped {skipped} unparseable line(s) in {session_id}");
    }
    log::info!(
        "debug session show: {session_id} pane={pane:?} join_agents={join_agents} entries={}",
        timeline.len()
    );
    if json {
        println!(
            "{}",
            json!({ "session_id": session_id, "pane": pane, "skipped": skipped, "records": timeline })
        );
        return 0;
    }
    for entry in &timeline {
        println!("{}", render_line(entry));
    }
    0
}

fn join_nums(v: &Value) -> String {
    let items: Vec<String> = v
        .as_array()
        .map(|a| a.iter().map(|n| n.to_string()).collect())
        .unwrap_or_default();
    if items.is_empty() { "-".into() } else { items.join(",") }
}

fn join_strs(v: &Value) -> String {
    let items: Vec<String> = v
        .as_array()
        .map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    if items.is_empty() { "-".into() } else { items.join(",") }
}

/// Keep rows for `pane`, plus the permission/tool rows of calls that pane made
/// (those carry a `call_id` but no pane of their own).
pub(crate) fn filter_pane(rows: Vec<Value>, pane: Option<u64>) -> Vec<Value> {
    let Some(pane) = pane else {
        return rows;
    };
    let calls: std::collections::HashSet<String> = rows
        .iter()
        .filter(|r| r["pane_id"].as_u64() == Some(pane))
        .filter_map(|r| r["call_id"].as_str().map(str::to_string))
        .collect();
    rows.into_iter()
        .filter(|r| {
            r["pane_id"].as_u64() == Some(pane)
                || r["call_id"].as_str().is_some_and(|c| calls.contains(c))
        })
        .collect()
}

fn agent_roots() -> Vec<PathBuf> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    vec![home.join(".pi/agent/sessions"), home.join(".claude/projects")]
}

/// Find the transcript file for an agent session id under `roots` (a file whose
/// name contains the id, one directory level deep).
fn find_transcript(roots: &[PathBuf], session_id: &str) -> Option<PathBuf> {
    for root in roots {
        let Ok(dirs) = std::fs::read_dir(root) else { continue };
        for dir in dirs.flatten() {
            let path = dir.path();
            let candidates: Vec<PathBuf> = if path.is_dir() {
                std::fs::read_dir(&path)
                    .map(|rd| rd.flatten().map(|e| e.path()).collect())
                    .unwrap_or_default()
            } else {
                vec![path]
            };
            for file in candidates {
                let name = file.file_name().map(|n| n.to_string_lossy().into_owned());
                if name.is_some_and(|n| n.ends_with(".jsonl") && n.contains(session_id)) {
                    return Some(file);
                }
            }
        }
    }
    None
}

/// Compact one-line text for an agent message's content (string or parts).
fn content_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .map(|part| match part["type"].as_str() {
                Some("text") => part["text"].as_str().unwrap_or("").to_string(),
                Some("tool_use" | "toolCall") => format!(
                    "[tool {} {}]",
                    part["name"].as_str().unwrap_or("?"),
                    part.get("input").or_else(|| part.get("arguments")).unwrap_or(&Value::Null)
                ),
                Some("tool_result") => format!("[tool result {}]", content_text(&part["content"])),
                Some(other) => format!("[{other}]"),
                None => String::new(),
            })
            .collect::<Vec<_>>()
            .join(" "),
        _ => String::new(),
    }
}

/// Agent transcript entries for every `Provenance.session_id` in `rows`.
pub(crate) fn agent_entries(rows: &[Value], roots: &[PathBuf]) -> Vec<Value> {
    let mut seen = std::collections::BTreeSet::new();
    for row in rows {
        if let Some(id) = recorder::agent_session_of(row) {
            seen.insert(id);
        }
    }
    let mut out = Vec::new();
    for id in seen {
        let Some(path) = find_transcript(roots, &id) else {
            eprintln!("warning: no transcript found for agent session {id}");
            continue;
        };
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) => {
                eprintln!("warning: cannot read {}: {e}", path.display());
                continue;
            }
        };
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            let Ok(row) = serde_json::from_str::<Value>(line) else { continue };
            let message = row.get("message").unwrap_or(&row);
            let role = message["role"]
                .as_str()
                .or_else(|| row["type"].as_str())
                .unwrap_or("?");
            let body = content_text(&message["content"]);
            if body.trim().is_empty() {
                continue;
            }
            let ts = row["timestamp"]
                .as_str()
                .map(str::to_string)
                .or_else(|| message["timestamp"].as_str().map(str::to_string));
            let Some(ts) = ts else { continue };
            out.push(json!({
                "source": "agent",
                "ts": ts,
                "agent_session": id,
                "role": role,
                "text": body,
            }));
        }
    }
    out
}

fn parse_ts(value: &Value) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value.as_str()?)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

/// Host rows (kept in `seq` order) interleaved with agent entries by time. The
/// sort is stable, so equal timestamps keep host order.
pub(crate) fn merge_timeline(rows: Vec<Value>, agents: Vec<Value>) -> Vec<Value> {
    let mut all: Vec<Value> = rows
        .into_iter()
        .map(|mut row| {
            row["source"] = json!("host");
            row
        })
        .collect();
    if agents.is_empty() {
        return all;
    }
    all.extend(agents);
    // Host rows without a parseable timestamp sort first rather than vanish.
    all.sort_by_key(|row| parse_ts(&row["ts"]));
    all
}

fn clip(text: &str) -> String {
    let one_line: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() <= LINE_TEXT_MAX {
        one_line
    } else {
        let cut: String = one_line.chars().take(LINE_TEXT_MAX).collect();
        format!("{cut}…")
    }
}

fn summary_of(row: &Value) -> String {
    let d = &row["data"];
    match row["kind"].as_str().unwrap_or("?") {
        "cli_call" => format!(
            "exit={} argv={}",
            d["exit_code"].as_i64().map(|n| n.to_string()).unwrap_or_else(|| "unknown".into()),
            d["argv"]
        ),
        "timeline_event" => format!(
            "{}/{} {}",
            d["app_id"].as_str().unwrap_or("?"),
            d["event"].as_str().unwrap_or("?"),
            d["summary"].as_str().unwrap_or("")
        ),
        "tool_call_requested" => format!(
            "{} {} wait_approved={} input={}",
            d["caller"].as_str().unwrap_or("?"),
            d["tool"].as_str().unwrap_or("?"),
            d["wait_approved"],
            d["input"]
        ),
        "tool_call_completed" => format!(
            "ok={} code={} pending={} {}",
            d["ok"],
            d["error_code"].as_str().unwrap_or("-"),
            d["pending_request_id"].as_str().unwrap_or("-"),
            d["error"].as_str().unwrap_or("")
        ),
        "permission_ask" | "permission_decision" | "permission_use" | "permission_audit" => format!(
            "{} actor={} decision={} pending={} grant={}",
            d["audit_kind"].as_str().unwrap_or("?"),
            d["actor"].as_str().unwrap_or("?"),
            d["choice"].as_str().or(d["decision"].as_str()).unwrap_or("?"),
            d["pending_request_id"].as_str().unwrap_or("-"),
            d["grant_id"].as_str().unwrap_or("-")
        ),
        "host_event" | "host_effect" => d["kind"]
            .as_str()
            .or(d["effect"].as_str())
            .unwrap_or("?")
            .to_string(),
        _ => d.to_string(),
    }
}

/// One timeline line for human output.
pub(crate) fn render_line(entry: &Value) -> String {
    let time = parse_ts(&entry["ts"])
        .map(|t| t.format("%H:%M:%S%.3f").to_string())
        .unwrap_or_else(|| entry["ts"].as_str().unwrap_or("?").to_string());
    if entry["source"] == "agent" {
        return format!(
            "{time}        agent  [{}] {}: {}",
            entry["agent_session"].as_str().unwrap_or("?"),
            entry["role"].as_str().unwrap_or("?"),
            clip(entry["text"].as_str().unwrap_or(""))
        );
    }
    let ids = [("ctx", "context_id"), ("pane", "pane_id")]
        .iter()
        .filter_map(|(label, key)| entry[key].as_u64().map(|n| format!("{label}={n}")))
        .chain(entry["call_id"].as_str().map(|c| format!("call={c}")))
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "{time} {:>5}  {}  {}{}",
        entry["seq"].as_u64().unwrap_or(0),
        entry["kind"].as_str().unwrap_or("?"),
        if entry["kind"] == "cli_call" { summary_of(entry) } else { clip(&summary_of(entry)) },
        if ids.is_empty() { String::new() } else { format!("  ({ids})") }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(seq: u64, ts: &str, kind: &str, pane: Option<u64>, call: Option<&str>) -> Value {
        json!({"seq": seq, "ts": ts, "kind": kind, "pane_id": pane, "call_id": call,
               "context_id": null, "data": {}})
    }

    #[test]
    fn pane_filter_keeps_the_pane_and_its_calls() {
        let rows = vec![
            row(1, "2026-01-01T00:00:01Z", "tool_call_requested", Some(5), Some("c1")),
            row(2, "2026-01-01T00:00:02Z", "permission_ask", None, Some("c1")),
            row(3, "2026-01-01T00:00:03Z", "tool_call_requested", Some(6), Some("c2")),
            row(4, "2026-01-01T00:00:04Z", "permission_ask", None, Some("c2")),
        ];
        let kept = filter_pane(rows, Some(5));
        let seqs: Vec<u64> = kept.iter().map(|r| r["seq"].as_u64().unwrap()).collect();
        assert_eq!(seqs, [1, 2]);
    }

    #[test]
    fn agent_entries_interleave_by_time() {
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("proj");
        std::fs::create_dir_all(&proj).unwrap();
        std::fs::write(
            proj.join("sess-agent-1.jsonl"),
            concat!(
                r#"{"type":"user","timestamp":"2026-01-01T00:00:02.500Z","message":{"role":"user","content":"do /todo"}}"#,
                "\n",
                r#"{"type":"assistant","timestamp":"2026-01-01T00:00:05Z","message":{"role":"assistant","content":[{"type":"tool_use","name":"Bash","input":{"cmd":"ls"}}]}}"#,
                "\n"
            ),
        )
        .unwrap();
        let host = vec![
            json!({"seq":1,"ts":"2026-01-01T00:00:01Z","kind":"timeline_event","pane_id":5,"call_id":null,
                   "context_id":1,"data":{"event":"pane.lifecycle","app_id":"plexi.host.panes","summary":"x",
                   "payload":{"provenance":{"session_id":"sess-agent-1"}}}}),
            row(2, "2026-01-01T00:00:03Z", "permission_ask", Some(5), None),
        ];
        let agents = agent_entries(&host, &[dir.path().to_path_buf()]);
        assert_eq!(agents.len(), 2);
        let merged = merge_timeline(host, agents);
        let order: Vec<String> = merged
            .iter()
            .map(|e| format!("{}:{}", e["source"].as_str().unwrap(), e["ts"].as_str().unwrap()))
            .collect();
        assert_eq!(
            order,
            [
                "host:2026-01-01T00:00:01Z",
                "agent:2026-01-01T00:00:02.500Z",
                "host:2026-01-01T00:00:03Z",
                "agent:2026-01-01T00:00:05Z"
            ]
        );
        assert!(render_line(&merged[3]).contains("[tool Bash"));
    }
}

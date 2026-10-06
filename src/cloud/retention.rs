//! 30-day retention for data Plexi controls on this desktop.
//!
//! Always deleted when older than the window: dated log archives. Local ledger
//! rows and assistant conversations are deleted only when the user opts in.
//! Relay registry rows and queued envelopes are the relay process's job; see
//! the module docs in [`super`].

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, NaiveDate, Utc};

/// Cloud ceiling for local ledger rows and assistant conversations.
pub const CLOUD_RETENTION_DAYS: i64 = 30;

/// What one retention pass deleted. Counts only — never file contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetentionReport {
    pub logs_pruned: u32,
    pub ledger_rows_pruned: u32,
    pub conversations_pruned: u32,
}

/// Inputs for one pass. Paths are explicit so a test can freeze `now`.
pub struct RetentionInput<'a> {
    pub now: DateTime<Utc>,
    /// Calendar day used for dated log archives. Startup passes local today,
    /// matching log rotation.
    pub log_today: NaiveDate,
    pub log_retention_days: u32,
    pub config_dir: &'a Path,
    pub assistant_dirs: &'a [PathBuf],
    /// `false` keeps every local ledger row and conversation.
    pub retain_local_history: bool,
}

/// Run one retention pass. Logs an info line with counts.
pub fn run(input: &RetentionInput<'_>) -> Result<RetentionReport, String> {
    let logs_pruned = crate::platform::logging::prune_controlled_logs(
        input.config_dir,
        input.log_today,
        input.log_retention_days,
    );
    let mut ledger_rows_pruned = 0;
    let mut conversations_pruned = 0;
    if input.retain_local_history {
        let cutoff = input.now - Duration::days(CLOUD_RETENTION_DAYS);
        for name in ["ai-ledger.jsonl", "agent-ledger.jsonl"] {
            ledger_rows_pruned += prune_ledger(&input.config_dir.join(name), cutoff)?;
        }
        for dir in input.assistant_dirs {
            conversations_pruned += prune_assistant_dir(dir, cutoff)?;
        }
    }
    let report = RetentionReport {
        logs_pruned,
        ledger_rows_pruned,
        conversations_pruned,
    };
    log::info!(
        "cloud retention: logs_pruned={} ledger_rows_pruned={} conversations_pruned={} retain_local_history={}",
        report.logs_pruned,
        report.ledger_rows_pruned,
        report.conversations_pruned,
        input.retain_local_history,
    );
    Ok(report)
}

/// Host-startup entry. Skips assistant dirs when no workspace is open.
pub fn run_at_startup(
    now: DateTime<Utc>,
    log_today: NaiveDate,
    log_retention_days: u32,
    retain_local_history: bool,
    workspace_root: Option<&Path>,
) -> Result<RetentionReport, String> {
    let config_dir = crate::config::config_dir();
    let mut assistant_dirs = Vec::new();
    if let Some(root) = workspace_root {
        assistant_dirs.push(
            root.join(crate::config::workspace_channel_dir())
                .join("assistant"),
        );
    }
    run(&RetentionInput {
        now,
        log_today,
        log_retention_days,
        config_dir: &config_dir,
        assistant_dirs: &assistant_dirs,
        retain_local_history,
    })
}

fn prune_ledger(path: &Path, cutoff: DateTime<Utc>) -> Result<u32, String> {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(format!("read {}: {error}", path.display())),
    };
    let mut kept = String::new();
    let mut pruned = 0u32;
    for line in raw.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if ledger_line_is_old(line, cutoff) {
            pruned += 1;
            continue;
        }
        kept.push_str(line);
        kept.push('\n');
    }
    if pruned > 0 {
        crate::platform::fs::atomic_write(path, kept.as_bytes())?;
    }
    Ok(pruned)
}

fn ledger_line_is_old(line: &str, cutoff: DateTime<Utc>) -> bool {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
        return false;
    };
    let Some(ts) = value.get("ts").and_then(|item| item.as_str()) else {
        return false;
    };
    let Ok(parsed) = DateTime::parse_from_rfc3339(ts) else {
        return false;
    };
    parsed.with_timezone(&Utc) < cutoff
}

fn prune_assistant_dir(dir: &Path, cutoff: DateTime<Utc>) -> Result<u32, String> {
    if !dir.is_dir() {
        return Ok(0);
    }
    let mut ids = HashSet::new();
    collect_ids(&dir.join("conversations"), "jsonl", &mut ids);
    collect_ids(&dir.join("history"), "json", &mut ids);
    let mut deleted = HashSet::new();
    for id in ids {
        if !safe_id(&id) {
            log::warn!("cloud retention: skipped unsafe conversation id");
            continue;
        }
        let Some(updated) = conversation_updated_at(dir, &id) else {
            continue;
        };
        if updated < cutoff {
            delete_conversation(dir, &id)?;
            deleted.insert(id);
        }
    }
    if !deleted.is_empty() {
        clear_active_pointers(&dir.join("state.toml"), &deleted)?;
    }
    Ok(deleted.len() as u32)
}

fn collect_ids(dir: &Path, extension: &str, ids: &mut HashSet<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some(extension) {
            continue;
        }
        if let Some(id) = path.file_stem().and_then(|value| value.to_str()) {
            ids.insert(id.to_string());
        }
    }
}

fn safe_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
}

fn conversation_updated_at(dir: &Path, id: &str) -> Option<DateTime<Utc>> {
    let history = dir.join("history").join(format!("{id}.json"));
    if let Ok(raw) = std::fs::read_to_string(&history) {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) {
            if let Some(ts) = value.get("updated_at").and_then(|item| item.as_str()) {
                if let Ok(parsed) = DateTime::parse_from_rfc3339(ts) {
                    return Some(parsed.with_timezone(&Utc));
                }
            }
        }
    }
    None
}

fn delete_conversation(dir: &Path, id: &str) -> Result<(), String> {
    for path in [
        dir.join("conversations").join(format!("{id}.jsonl")),
        dir.join("history").join(format!("{id}.json")),
        dir.join("exports").join(format!("{id}.md")),
    ] {
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("remove {}: {error}", path.display())),
        }
    }
    let checkpoints = dir.join("checkpoints").join(id);
    if checkpoints.is_dir() {
        std::fs::remove_dir_all(&checkpoints)
            .map_err(|error| format!("remove {}: {error}", checkpoints.display()))?;
    }
    Ok(())
}

fn clear_active_pointers(state_path: &Path, deleted: &HashSet<String>) -> Result<(), String> {
    if !state_path.exists() {
        return Ok(());
    }
    let raw = std::fs::read_to_string(state_path)
        .map_err(|error| format!("read {}: {error}", state_path.display()))?;
    let mut value: toml::Value =
        toml::from_str(&raw).map_err(|error| format!("parse {}: {error}", state_path.display()))?;
    let Some(table) = value.as_table_mut() else {
        return Ok(());
    };
    let Some(contexts) = table
        .get_mut("contexts")
        .and_then(|item| item.as_table_mut())
    else {
        return Ok(());
    };
    let keys: Vec<String> = contexts.keys().cloned().collect();
    for key in keys {
        let Some(context_table) = contexts.get_mut(&key).and_then(|item| item.as_table_mut())
        else {
            continue;
        };
        let points_at_deleted = context_table
            .get("active_conversation")
            .and_then(|item| item.as_str())
            .is_some_and(|active| deleted.contains(active));
        if points_at_deleted {
            context_table.insert(
                "active_conversation".to_string(),
                toml::Value::String(String::new()),
            );
        }
    }
    let rendered = toml::to_string(&value)
        .map_err(|error| format!("serialize {}: {error}", state_path.display()))?;
    crate::platform::fs::atomic_write(state_path, rendered.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::account::AccountStore;
    use crate::assistant::model::{Turn, TurnRole};
    use crate::assistant::store::AssistantStore;
    use crate::plexi_ai::backend::BillingModel;
    use crate::plexi_ai::ledger::{append, LedgerRow};
    use chrono::TimeZone;

    fn utc(year: i32, month: u32, day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, 12, 0, 0).unwrap()
    }

    fn write_history(dir: &Path, id: &str, updated_at: &str) {
        let path = dir.join("history").join(format!("{id}.json"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            path,
            format!(r#"{{"updated_at":"{updated_at}","checkpoints":[],"compactions":[],"interruptions":[]}}"#),
        )
        .unwrap();
    }

    #[test]
    fn retention_drops_data_older_than_thirty_days_and_keeps_the_rest() {
        let profile = tempfile::tempdir().unwrap();
        let _profile = crate::config::set_test_profile_dir(profile.path().to_path_buf());
        let _channel = crate::config::set_test_channel("cloudtest");
        assert!(AccountStore::open().current().is_none());
        assert!(!profile.path().join("account.toml").exists());

        // A row written through the alpha ledger. Its timestamp is the real
        // clock, so the frozen 30-day cutoff keeps it. `run-live` is the marker
        // the assertions look for.
        append(&LedgerRow::with_attribution(
            "agents",
            BillingModel::Metered,
            Some("run-live".to_string()),
            None,
            Some(3),
            Some(4),
            None,
        ));
        let ledger = profile.path().join("ai-ledger.jsonl");
        let mut extra = std::fs::read_to_string(&ledger).unwrap();
        extra.push_str(
            "{\"ts\":\"2026-04-09T12:00:00Z\",\"backend\":\"agents\",\"billing\":\"metered\",\"input_tokens\":1,\"output_tokens\":1,\"cost_cents\":0}\n",
        );
        extra.push_str(
            "{\"ts\":\"2026-04-10T12:00:00Z\",\"backend\":\"agents\",\"billing\":\"metered\",\"input_tokens\":1,\"output_tokens\":1,\"cost_cents\":0}\n",
        );
        extra.push_str("not json\n");
        std::fs::write(&ledger, extra).unwrap();

        let workspace = profile.path().join("ws");
        let store = AssistantStore::new(&workspace, 1);
        store
            .write_turns("live-conv", &[Turn::now(TurnRole::User, "still here")])
            .unwrap();
        let assistant = workspace
            .join(crate::config::workspace_channel_dir())
            .join("assistant");
        write_history(&assistant, "live-conv", "2026-05-09T12:00:00Z");
        write_history(&assistant, "old-conv", "2026-04-09T12:00:00Z");
        write_history(&assistant, "edge-conv", "2026-04-10T12:00:00Z");
        std::fs::create_dir_all(assistant.join("conversations")).unwrap();
        std::fs::write(
            assistant.join("conversations").join("old-conv.jsonl"),
            "{\"role\":\"User\",\"text\":\"secret old turn\",\"created_at\":\"2026-04-09T12:00:00Z\"}\n",
        )
        .unwrap();
        std::fs::write(
            assistant.join("conversations").join("edge-conv.jsonl"),
            "{\"role\":\"User\",\"text\":\"edge\",\"created_at\":\"2026-04-10T12:00:00Z\"}\n",
        )
        .unwrap();
        let checkpoint = assistant.join("checkpoints").join("old-conv");
        std::fs::create_dir_all(&checkpoint).unwrap();
        std::fs::write(checkpoint.join("c1.jsonl"), "old checkpoint\n").unwrap();
        std::fs::write(
            assistant.join("state.toml"),
            "show_thoughts = true\n\n[contexts.1]\nactive_conversation = \"old-conv\"\n",
        )
        .unwrap();

        let today = NaiveDate::from_ymd_opt(2026, 5, 10).unwrap();
        std::fs::write(profile.path().join("plexi-2026-04-09.log"), "old log\n").unwrap();
        std::fs::write(profile.path().join("plexi-2026-04-10.log"), "edge log\n").unwrap();
        std::fs::write(profile.path().join("plexi.log"), "live log\n").unwrap();
        std::fs::write(profile.path().join("config.toml"), "[cloud]\n").unwrap();

        let now = utc(2026, 5, 10);
        let dirs = [assistant.clone()];
        let kept = run(&RetentionInput {
            now,
            log_today: today,
            log_retention_days: 30,
            config_dir: profile.path(),
            assistant_dirs: &dirs,
            retain_local_history: false,
        })
        .unwrap();
        assert_eq!(kept.logs_pruned, 1);
        assert_eq!(kept.ledger_rows_pruned, 0);
        assert_eq!(kept.conversations_pruned, 0);
        assert!(!profile.path().join("plexi-2026-04-09.log").exists());
        assert!(profile.path().join("plexi-2026-04-10.log").exists());
        assert!(profile.path().join("plexi.log").exists());
        assert!(profile.path().join("config.toml").exists());
        let ledger_text = std::fs::read_to_string(&ledger).unwrap();
        assert!(ledger_text.contains("2026-04-09T12:00:00Z"));
        assert!(ledger_text.contains("run-live"));
        assert!(assistant
            .join("conversations")
            .join("old-conv.jsonl")
            .exists());
        assert!(!profile.path().join("account.toml").exists());

        let pruned = run(&RetentionInput {
            now,
            log_today: today,
            log_retention_days: 30,
            config_dir: profile.path(),
            assistant_dirs: &dirs,
            retain_local_history: true,
        })
        .unwrap();
        assert_eq!(pruned.logs_pruned, 0);
        assert_eq!(pruned.ledger_rows_pruned, 1);
        assert_eq!(pruned.conversations_pruned, 1);
        let ledger_text = std::fs::read_to_string(&ledger).unwrap();
        assert!(!ledger_text.contains("2026-04-09T12:00:00Z"));
        assert!(ledger_text.contains("2026-04-10T12:00:00Z"));
        assert!(ledger_text.contains("run-live"));
        assert!(ledger_text.contains("not json"));
        assert!(!assistant.join("history").join("old-conv.json").exists());
        assert!(!assistant
            .join("conversations")
            .join("old-conv.jsonl")
            .exists());
        assert!(!assistant.join("checkpoints").join("old-conv").exists());
        assert!(assistant.join("history").join("edge-conv.json").exists());
        assert!(assistant
            .join("conversations")
            .join("edge-conv.jsonl")
            .exists());
        assert!(assistant.join("history").join("live-conv.json").exists());
        assert!(assistant
            .join("conversations")
            .join("live-conv.jsonl")
            .exists());
        let state = std::fs::read_to_string(assistant.join("state.toml")).unwrap();
        assert!(state.contains("show_thoughts"));
        assert!(!state.contains("old-conv"));
        assert!(AccountStore::open().current().is_none());
        assert!(!profile.path().join("account.toml").exists());
        assert_eq!(store.load_turns("live-conv").len(), 1);
    }

    #[test]
    fn unset_cloud_config_keeps_local_history() {
        let bare: crate::config::PlexiConfig = toml::from_str("font_size = 14.0\n").unwrap();
        assert!(bare.cloud.is_none());
        let opted: crate::config::PlexiConfig =
            toml::from_str("[cloud]\nretain_local_history = true\n").unwrap();
        assert!(opted.cloud.unwrap().retain_local_history());
    }
}

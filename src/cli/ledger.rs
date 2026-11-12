//! `plexi ledger` — read the local AI cost ledger. This command does not
//! talk to the host; the file is `config_dir()/ai-ledger.jsonl`.
//!
//! With no subcommand, the CLI prints the per-client human summary.

use std::fmt::Write;

use crate::plexi_ai::ledger::{self, LedgerSummary, SummaryBy};

/// `plexi ledger` and `plexi ledger summary`. Omitted `--by` means `client`.
pub fn ledger_summary_cli(by: Option<&str>, since: Option<&str>, json: bool) -> i32 {
    let by = match SummaryBy::parse(by.unwrap_or("client")) {
        Ok(by) => by,
        Err(error) => {
            eprintln!("error: {error}");
            return 1;
        }
    };
    let summary = match ledger::summary(by, since) {
        Ok(summary) => summary,
        Err(error) => {
            eprintln!("error: {error}");
            return 1;
        }
    };
    if json {
        println!("{}", summary.to_value());
        return 0;
    }
    print!("{}", render_human(&summary));
    0
}

fn render_human(summary: &LedgerSummary) -> String {
    let mut out = String::new();
    let since = summary.since.as_deref().unwrap_or("(all)");
    writeln!(out, "by: {}", summary.by.as_str()).expect("format ledger summary");
    writeln!(out, "since: {since}").expect("format ledger summary");
    writeln!(out).expect("format ledger summary");
    writeln!(
        out,
        "{:<16} {:>6} {:>14} {:>14} {:>12} {:>10}",
        summary.by.as_str(),
        "runs",
        "input_tokens",
        "output_tokens",
        "cost_usd",
        "wall_ms"
    )
    .expect("format ledger summary");
    if summary.groups.is_empty() {
        writeln!(out, "(no rows)").expect("format ledger summary");
        return out;
    }
    for group in &summary.groups {
        let key = group.key.as_deref().unwrap_or("(none)");
        writeln!(
            out,
            "{:<16} {:>6} {:>14} {:>14} {:>12} {:>10}",
            key,
            group.runs,
            fmt_tokens(group.input_tokens),
            fmt_tokens(group.output_tokens),
            group
                .cost_usd
                .map(|cost| format!("{cost:.6}"))
                .unwrap_or_else(|| "—".to_string()),
            fmt_opt(group.wall_ms)
        )
        .expect("format ledger summary");
    }
    out
}

fn fmt_tokens(value: Option<u64>) -> String {
    match value {
        Some(count) if count > 0 => count.to_string(),
        _ => "unknown".to_string(),
    }
}

fn fmt_opt(value: Option<u64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "—".to_string())
}

#[cfg(test)]
mod tests {
    use super::render_human;
    use crate::plexi_ai::ledger::{LedgerSummary, LedgerSummaryGroup, SummaryBy};

    #[test]
    fn human_summary_prints_per_client_totals_or_unknown() {
        let summary = LedgerSummary {
            by: SummaryBy::Client,
            since: None,
            groups: vec![
                LedgerSummaryGroup {
                    key: Some("du".to_string()),
                    runs: 1,
                    input_tokens: None,
                    output_tokens: None,
                    cost_usd: None,
                    wall_ms: Some(0),
                },
                LedgerSummaryGroup {
                    key: Some("narrative".to_string()),
                    runs: 4,
                    input_tokens: Some(284),
                    output_tokens: Some(31),
                    cost_usd: Some(0.5),
                    wall_ms: Some(1500),
                },
            ],
        };
        let text = render_human(&summary);
        assert!(text.starts_with("by: client\n"), "{text}");
        let narrative = text
            .lines()
            .find(|line| line.starts_with("narrative"))
            .expect("narrative row");
        assert!(
            narrative.contains("284") && narrative.contains("31"),
            "{narrative}"
        );
        let du = text
            .lines()
            .find(|line| line.starts_with("du"))
            .expect("du row");
        assert!(du.contains("unknown"), "{du}");
        assert!(
            du.contains('—') && du.contains("     0"),
            "cost stays blank and a measured wall time of 0 stays numeric: {du}"
        );
    }
}

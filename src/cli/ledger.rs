//! `plexi ledger` — read the local AI cost ledger. This command does not
//! talk to the host; the file is `config_dir()/ai-ledger.jsonl`.

use crate::plexi_ai::ledger::{self, LedgerSummary, SummaryBy};

/// `plexi ledger summary`. Omitted `--by` means `client`.
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
    print_human(&summary);
    0
}

fn print_human(summary: &LedgerSummary) {
    let since = summary.since.as_deref().unwrap_or("(all)");
    println!("by: {}", summary.by.as_str());
    println!("since: {since}");
    println!();
    println!(
        "{:<16} {:>6} {:>14} {:>14} {:>12} {:>10}",
        summary.by.as_str(),
        "runs",
        "input_tokens",
        "output_tokens",
        "cost_usd",
        "wall_ms"
    );
    if summary.groups.is_empty() {
        println!("(no rows)");
        return;
    }
    for group in &summary.groups {
        let key = group.key.as_deref().unwrap_or("(none)");
        println!(
            "{:<16} {:>6} {:>14} {:>14} {:>12} {:>10}",
            key,
            group.runs,
            fmt_opt(group.input_tokens),
            fmt_opt(group.output_tokens),
            group
                .cost_usd
                .map(|cost| format!("{cost:.6}"))
                .unwrap_or_else(|| "—".to_string()),
            fmt_opt(group.wall_ms)
        );
    }
}

fn fmt_opt(value: Option<u64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "—".to_string())
}

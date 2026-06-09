//! Generates website/src/content/docs/cli.md from the clap Command tree.
//! Usage: cargo run -p gen_cli_docs > website/src/content/docs/cli.md

use clap::{Arg, ArgAction, Command, CommandFactory};
use plexi::cli_args::Cli;

fn main() {
    let cmd = Cli::command();
    // Use the version from the plexi package, not gen_cli_docs
    let version = cmd.get_version().unwrap_or(env!("CARGO_PKG_VERSION"));

    print!(
        r#"---
title: CLI Reference
description: Complete reference for all plexi subcommands and flags.
verified_version: "{version}"
order: 7
---

The `plexi` CLI is the primary way to interact with a running Plexi instance from the terminal, and to manage workspaces and apps from outside the UI.

All commands work identically across build channels (`plexi`, `plexi-alpha`, `plexi-beta`). When run inside a Plexi pane, `PLEXI_SOCKET` routes host commands to the correct running instance automatically.

"#
    );

    for sub in cmd.get_subcommands() {
        if sub.is_hide_set() {
            continue;
        }
        emit_subcommand(sub, "plexi", 2);
    }
}

fn emit_subcommand(cmd: &Command, parent_path: &str, depth: usize) {
    let name = cmd.get_name();
    let full_path = format!("{parent_path} {name}");
    let heading = "#".repeat(depth);
    let about = cmd
        .get_long_about()
        .or_else(|| cmd.get_about())
        .map(|s| s.to_string())
        .unwrap_or_default();

    println!("{heading} `{full_path}`");
    println!();
    if !about.is_empty() {
        // Normalize multi-line about strings: blank lines become paragraph breaks.
        // Clap joins consecutive doc comment lines with spaces, so the "about" text
        // already has the routine's schedule bullets collapsed. Emit what clap gives us;
        // the schedule reference block below re-emits the formatted version.
        let normalized = about
            .lines()
            .map(|l| l.trim())
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_string();
        println!("{normalized}");
        println!();
    }

    // Inject a hand-authored schedule reference block for `plexi routine`.
    if full_path == "plexi routine" {
        println!("### Routine file format (`.plexi/routines.toml`)");
        println!();
        println!("```toml");
        println!("[[routine]]");
        println!(r#"name      = "morning-sync""#);
        println!(r#"command   = "./scripts/sync.sh""#);
        println!(r#"schedule  = "daily at 09:00""#);
        println!(r#"context   = "work"   # optional: only fires when this context is active"#);
        println!(
            r#"ephemeral = true     # optional: close the spawned pane when the command exits"#
        );
        println!("```");
        println!();
        println!("### Schedule formats");
        println!();
        println!("| Format | Example |");
        println!("|---|---|");
        println!("| `every N seconds` | `every 30 seconds` |");
        println!("| `every N minutes` | `every 5 minutes` |");
        println!("| `every N hours`   | `every 2 hours` |");
        println!("| `daily at HH:MM`  | `daily at 09:00` |");
        println!("| `weekly on <day> at HH:MM` | `weekly on monday at 09:00` |");
        println!("| `monthly on N at HH:MM`    | `monthly on 1 at 08:00` |");
        println!("| 5-field cron `m h dom mon dow` | `0 9 * * 1-5` |");
        println!();
    }

    let subs: Vec<&Command> = cmd.get_subcommands().filter(|s| !s.is_hide_set()).collect();

    let args: Vec<&Arg> = cmd
        .get_arguments()
        .filter(|a| a.get_id() != "help" && a.get_id() != "version")
        .collect();

    if !subs.is_empty() {
        println!("| Subcommand | Description |");
        println!("|---|---|");
        for sub in &subs {
            let desc = sub.get_about().map(|s| s.to_string()).unwrap_or_default();
            println!("| `{}` | {} |", sub.get_name(), desc);
        }
        println!();

        for sub in subs {
            emit_subcommand(sub, &full_path, depth + 1);
        }
    } else if !args.is_empty() {
        println!("| Flag / Arg | Type | Required | Description |");
        println!("|---|---|---|---|");
        for arg in args {
            emit_arg_row(arg);
        }
        println!();
    }
}

fn emit_arg_row(arg: &Arg) {
    let id = arg.get_id().as_str();
    let is_positional = arg.get_long().is_none() && arg.get_short().is_none();

    let flag = if is_positional {
        format!("`<{id}>`")
    } else {
        let long = arg.get_long().map(|l| format!("--{l}")).unwrap_or_default();
        let short = arg
            .get_short()
            .map(|s| format!(" / `-{s}`"))
            .unwrap_or_default();
        format!("`{long}`{short}")
    };

    let ty = match arg.get_action() {
        ArgAction::SetTrue | ArgAction::SetFalse | ArgAction::Count => "flag".to_string(),
        ArgAction::Append => "string (repeatable)".to_string(),
        _ => "string".to_string(),
    };

    let required = if arg.is_required_set() { "yes" } else { "no" };

    let help = arg
        .get_long_help()
        .or_else(|| arg.get_help())
        .map(|s| {
            s.to_string()
                .lines()
                .map(|l| l.trim())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default();

    let default = arg
        .get_default_values()
        .first()
        .and_then(|v| {
            let s = v.to_string_lossy();
            // Skip empty string defaults — they're noise
            if s.is_empty() {
                None
            } else {
                Some(format!(" Default: `{s}`."))
            }
        })
        .unwrap_or_default();

    let desc = format!("{help}{default}").trim().to_string();

    println!("| {flag} | {ty} | {required} | {desc} |");
}

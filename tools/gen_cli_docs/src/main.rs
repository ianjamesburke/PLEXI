//! Generates website/src/content/docs/cli.md from the clap Command tree.
//! Usage: cargo run -p gen_cli_docs > website/src/content/docs/cli.md

use clap::{Arg, ArgAction, Command, CommandFactory};
use plexi::cli_args::Cli;

fn main() {
    let cmd = Cli::command();

    print!(
        r#"---
title: CLI Reference
description: Complete reference for all plexi subcommands and flags.
order: 7
---

The `plexi` CLI is the primary way to interact with a running Plexi instance from the terminal, and to manage workspaces and local apps from outside the UI. This reference is generated from the complete public command inventory; hidden implementation commands are omitted, and beta-only entries are labelled where they appear.

Stable v1 covers the tiling host, panes, subcontexts, status hooks, Quick Note, and local app runtime. Assistant, marketplace, MCP client, app-wrapper, and routine surfaces are beta-gated and do not appear in stable help. Use `plexi-beta` or an explicit worktree channel only when testing those gated surfaces.

Each channel has its own binary and profile (`plexi`, `plexi-alpha`, `plexi-beta`). A channel-named binary always targets its own profile; the bare `plexi` binary honors an explicit `PLEXI_SOCKET` when run inside a Plexi pane.

"#
    );

    let subcommands = cmd
        .get_subcommands()
        .filter(|sub| !sub.is_hide_set())
        .collect::<Vec<_>>();
    for (index, sub) in subcommands.iter().enumerate() {
        emit_subcommand(sub, "plexi", 2, index + 1 == subcommands.len());
    }
}

fn emit_subcommand(cmd: &Command, parent_path: &str, depth: usize, is_last: bool) {
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

    if let Some(feature) = beta_gated_feature(&full_path) {
        println!("> **Beta-gated:** {feature} This reference is included for beta and worktree testing; it is not available from the stable v1 channel.");
        println!();
    }

    // Inject a hand-authored schedule reference block for `plexi routine`.
    if full_path == "plexi routine" {
        println!("### Routine file format (`{{workspace_channel_dir}}/routines.toml`)");
        println!();
        println!(
            "The file lives in the workspace channel directory beside the rest of the \
             workspace state — e.g. `.plexi-alpha/routines.toml` on the alpha channel."
        );
        println!();
        println!("```toml");
        println!("[[routine]]");
        println!(r#"name      = "morning-sync""#);
        println!(r#"command   = "./scripts/sync.sh""#);
        println!(r#"schedule  = "daily at 09:00""#);
        println!(
            r#"context   = "work"   # optional: fires into this context wherever it is; skipped if no context by that name exists"#
        );
        println!(
            r#"ephemeral = true     # optional: close the spawned pane when the command exits"#
        );
        println!(
            r#"enabled   = false    # optional: keep the routine but never fire it (`plexi routine disable`)"#
        );
        println!("```");
        println!();
        println!(
            "`plexi routine add` / `remove` / `enable` / `disable` edit this file for \
             you, validating the schedule against the same parser the scheduler uses \
             and preserving hand-written comments."
        );
        println!();
        println!(
            "A routine never stacks panes: while the previous run's pane is still \
             alive, due fires are skipped (with one notification per skip streak), \
             and the routine fires again on the first tick after that run ends. \
             Ephemeral panes close themselves when the command exits; a \
             non-ephemeral pane holds its routine until its shell session ends or \
             the pane is closed."
        );
        println!();
        println!("### Schedule formats");
        println!();
        println!("| Format | Example |");
        println!("|---|---|");
        println!("| `every N seconds` (or `Ns`) | `every 30 seconds` |");
        println!("| `every N minutes` (or `Nm`) | `every 5 minutes` |");
        println!("| `every N hours` (or `Nh`)   | `every 2 hours` |");
        println!("| `every minute` / `every hour` | `every minute` |");
        println!("| `daily at HH:MM`  | `daily at 09:00` |");
        println!("| `weekdays at HH:MM` | `weekdays at 09:00` |");
        println!("| `weekends at HH:MM` | `weekends at 10:30am` |");
        println!("| `weekly on <day> at HH:MM` | `weekly on monday at 09:00` |");
        println!("| `monthly on N at HH:MM`    | `monthly on 1 at 08:00` |");
        println!("| 5-field cron `m h dom mon dow` | `0 9 * * 1-5` |");
        println!();
        println!(
            "Singular unit names (`every 1 minute`) and am/pm times (`daily at 9am`) are \
             accepted; day names take short or full spellings (`mon` / `monday`)."
        );
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

        let subcommand_count = subs.len();
        for (index, sub) in subs.into_iter().enumerate() {
            emit_subcommand(
                sub,
                &full_path,
                depth + 1,
                is_last && index + 1 == subcommand_count,
            );
        }
    } else if !args.is_empty() {
        println!("| Flag / Arg | Type | Required | Description |");
        println!("|---|---|---|---|");
        for arg in args {
            emit_arg_row(arg, &full_path);
        }
        if !is_last {
            println!();
        }
    }
}

fn beta_gated_feature(full_path: &str) -> Option<&'static str> {
    match full_path {
        "plexi account"
        | "plexi account status"
        | "plexi account login"
        | "plexi account logout" => Some("Marketplace account management is a beta surface."),
        "plexi app publish" | "plexi app browse" | "plexi app search" => {
            Some("Marketplace publishing and catalog browsing are beta surfaces.")
        }
        "plexi events mcp-config" => Some("MCP client configuration is a beta surface."),
        "plexi routine" => Some("Routines are a post-v1 beta surface."),
        _ => None,
    }
}

fn beta_gated_arg(full_path: &str, id: &str) -> Option<&'static str> {
    match (full_path, id) {
        ("plexi app open", "mcp" | "cli") => {
            Some("Beta-gated: app wrappers are not available from the stable v1 channel.")
        }
        _ => None,
    }
}

fn emit_arg_row(arg: &Arg, full_path: &str) {
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

    let gate = beta_gated_arg(full_path, id)
        .map(|text| format!(" {text}"))
        .unwrap_or_default();
    let desc = format!("{help}{default}{gate}").trim().to_string();

    println!("| {flag} | {ty} | {required} | {desc} |");
}

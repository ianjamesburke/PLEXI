use clap::CommandFactory;

use crate::cli::args::Cli;

const HELP_GROUPS: &[(&str, &[&str])] = &[
    (
        "Workspace",
        &["run", "workspace", "secret", "routine", "agent", "context"],
    ),
    ("Apps", &["app", "account", "registry"]),
    ("Panes", &["pane", "notify"]),
    ("AI", &["ai"]),
    (
        "System",
        &[
            "completions",
            "config",
            "notes",
            "note",
            "doctor",
            "demo",
            "update",
            "uninstall",
        ],
    ),
];

pub fn print_grouped_help() {
    let cmd = Cli::command();
    let no_color = std::env::var_os("NO_COLOR").is_some();

    // Apply ANSI to already-padded text so width counting is based on visible chars.
    let header = |s: &str| -> String {
        if no_color {
            format!("{s}:")
        } else {
            format!("\x1b[1;32m{s}:\x1b[0m")
        }
    };
    let lit = |s: String| -> String {
        if no_color {
            s
        } else {
            format!("\x1b[1;36m{s}\x1b[0m")
        }
    };
    let dim = |s: &str| -> String {
        if no_color {
            s.to_string()
        } else {
            format!("\x1b[2m{s}\x1b[0m")
        }
    };

    // About
    if let Some(about) = cmd.get_about() {
        println!("{about}");
        println!();
    }

    // Usage
    let bin = cmd.get_name();
    println!("{} {bin} [OPTIONS] [COMMAND]", header("Usage"));
    println!();

    // Visible positional arguments (workspace_path)
    let visible_positional: Vec<_> = cmd
        .get_arguments()
        .filter(|a| !a.is_hide_set() && a.is_positional())
        .filter(|a| a.get_id() != "help" && a.get_id() != "version")
        .collect();

    if !visible_positional.is_empty() {
        println!("{}", header("Arguments"));
        for arg in &visible_positional {
            let name = format!("[{}]", arg.get_id().as_str().to_uppercase());
            let padded = format!("{name:<22}");
            let help = arg
                .get_help()
                .map(|s| s.to_string())
                .unwrap_or_default();
            println!("  {} {help}", lit(padded));
        }
        println!();
    }

    // Options: -h and -V only (--profile is hidden)
    println!("{}", header("Options"));
    println!("  {} Print help", lit(format!("{:<22}", "-h, --help")));
    if cmd.get_version().is_some() {
        println!(
            "  {} Print version",
            lit(format!("{:<22}", "-V, --version"))
        );
    }
    println!();

    // Grouped subcommands
    let col_width = HELP_GROUPS
        .iter()
        .flat_map(|(_, names)| names.iter())
        .filter(|&&n| cmd.find_subcommand(n).is_some())
        .map(|n| n.len())
        .max()
        .unwrap_or(10)
        + 2;

    for (heading, names) in HELP_GROUPS {
        println!("{}", header(heading));
        for &name in *names {
            if let Some(sub) = cmd.find_subcommand(name) {
                let about = sub
                    .get_about()
                    .map(|s| s.to_string())
                    .unwrap_or_default();
                println!("  {} {about}", lit(format!("{name:<col_width$}")));
            }
        }
        println!();
    }

    // After-help footer
    if let Some(after) = cmd.get_after_help() {
        println!("{}", dim(&after.to_string()));
    }
}

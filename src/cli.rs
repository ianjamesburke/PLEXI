use serde::Deserialize;
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::process::Command;

const APP_ID: &str = "plexi-run";
const COMMANDS_FILE: &str = ".plexi/commands.toml";

/// Parsed .plexi/commands.toml
#[derive(Deserialize)]
pub struct PlexiCommands {
    #[serde(default)]
    pub secrets: SecretsConfig,
    #[serde(default)]
    pub commands: HashMap<String, CommandDef>,
}

#[derive(Deserialize, Default)]
pub struct SecretsConfig {
    #[serde(default)]
    pub required: Vec<String>,
}

#[derive(Deserialize)]
pub struct CommandDef {
    pub run: String,
    #[serde(default)]
    pub secrets: Vec<String>,
}

/// Entry point for `plexi run <command_name>`.
/// Returns the exit code.
pub fn run_command(command_name: &str) -> i32 {
    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: could not determine current directory: {e}");
            return 1;
        }
    };

    let config_path = cwd.join(COMMANDS_FILE);
    let contents = match std::fs::read_to_string(&config_path) {
        Ok(c) => c,
        Err(_) => {
            eprintln!(
                "error: no {COMMANDS_FILE} found in {}",
                cwd.display()
            );
            eprintln!("Create a .plexi/commands.toml to define runnable commands.");
            return 1;
        }
    };

    let config: PlexiCommands = match toml::from_str(&contents) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: failed to parse {COMMANDS_FILE}: {e}");
            return 1;
        }
    };

    let cmd_def = match config.commands.get(command_name) {
        Some(c) => c,
        None => {
            eprintln!("error: unknown command '{command_name}'");
            if config.commands.is_empty() {
                eprintln!("No commands defined in {COMMANDS_FILE}.");
            } else {
                let mut names: Vec<&str> = config.commands.keys().map(|s| s.as_str()).collect();
                names.sort();
                eprintln!("Available commands: {}", names.join(", "));
            }
            return 1;
        }
    };

    // Collect all required secret keys: global + command-specific
    let mut secret_keys: Vec<&str> = config.secrets.required.iter().map(|s| s.as_str()).collect();
    for k in &cmd_def.secrets {
        if !secret_keys.contains(&k.as_str()) {
            secret_keys.push(k.as_str());
        }
    }

    // Resolve secrets from Keychain
    let dir_str = cwd.to_string_lossy();
    let mut resolved: Vec<(String, String)> = Vec::new();
    let mut missing: Vec<&str> = Vec::new();

    for key in &secret_keys {
        match crate::secrets::resolve_secret(key, APP_ID, &dir_str) {
            Some(value) => resolved.push((key.to_string(), value)),
            None => missing.push(key),
        }
    }

    if !missing.is_empty() {
        eprintln!("error: missing required secrets:");
        for key in &missing {
            eprintln!("  - {key}");
        }
        eprintln!();
        eprintln!("Set them with:");
        for key in &missing {
            eprintln!("  plexi secret set {key}");
        }
        return 1;
    }

    // Spawn the command via sh -c with secrets injected as env vars
    let mut child_cmd = Command::new("sh");
    child_cmd.arg("-c").arg(&cmd_def.run);
    for (key, value) in &resolved {
        child_cmd.env(key, value);
    }

    match child_cmd.status() {
        Ok(status) => status.code().unwrap_or(1),
        Err(e) => {
            eprintln!("error: failed to spawn command: {e}");
            1
        }
    }
}

/// Entry point for `plexi secret set <key>` — stores a secret for the current directory.
pub fn set_secret(key: &str) -> i32 {
    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: could not determine current directory: {e}");
            return 1;
        }
    };

    eprint!("Enter value for {key}: ");
    let _ = io::stderr().flush();

    let value = match read_secret_from_stdin() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("\nerror: failed to read secret: {e}");
            return 1;
        }
    };
    eprintln!(); // newline after hidden input

    if value.is_empty() {
        eprintln!("error: empty value, nothing stored");
        return 1;
    }

    let dir_str = cwd.to_string_lossy();
    if crate::secrets::store_secret(key, &value, APP_ID, &dir_str) {
        eprintln!("Stored secret '{key}' for {}", cwd.display());
        0
    } else {
        eprintln!("error: failed to store secret '{key}'");
        1
    }
}

/// Entry point for `plexi secret list` — lists secrets for the current directory.
pub fn list_secrets() -> i32 {
    let accounts = crate::secrets::list_secrets(APP_ID);

    if accounts.is_empty() {
        eprintln!("No secrets stored for {APP_ID}.");
        return 0;
    }

    // accounts are strings like "plexi-run/dir/key"
    // Group by directory
    let prefix = format!("{APP_ID}/");
    let mut by_dir: HashMap<String, Vec<String>> = HashMap::new();

    for account in &accounts {
        if let Some(rest) = account.strip_prefix(&prefix) {
            // rest = "dir/key" — split on last '/' to separate dir from key
            if let Some(last_slash) = rest.rfind('/') {
                let dir = &rest[..last_slash];
                let key = &rest[last_slash + 1..];
                by_dir.entry(dir.to_string()).or_default().push(key.to_string());
            }
        }
    }

    let mut dirs: Vec<&String> = by_dir.keys().collect();
    dirs.sort();

    for dir in dirs {
        println!("{}:", dir);
        let keys = by_dir.get(dir).unwrap();
        for key in keys {
            println!("  {key}");
        }
    }

    0
}

/// Read a line from stdin with echo disabled (for password-style input).
fn read_secret_from_stdin() -> io::Result<String> {
    // Disable echo via stty (avoids libc dependency).
    let _ = std::process::Command::new("stty")
        .arg("-echo")
        .status();

    let result = read_line_plain();

    // Restore echo.
    let _ = std::process::Command::new("stty")
        .arg("echo")
        .status();
    // Print newline since echo was off during input.
    eprintln!();

    result
}

fn read_line_plain() -> io::Result<String> {
    let mut buf = String::new();
    io::stdin().read_line(&mut buf)?;
    Ok(buf.trim_end_matches('\n').trim_end_matches('\r').to_string())
}

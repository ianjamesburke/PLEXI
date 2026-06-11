use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

pub fn detect_shell() -> String {
    if let Ok(shell) = std::env::var("SHELL") {
        if Path::new(&shell).exists() {
            return shell;
        }
    }

    for shell in [
        "/bin/zsh",
        "/usr/bin/zsh",
        "/bin/bash",
        "/usr/bin/bash",
        "/bin/sh",
    ] {
        if Path::new(shell).exists() {
            return shell.to_string();
        }
    }

    "/bin/sh".to_string()
}

/// Resolve the user's login-shell PATH and install it as the process PATH.
///
/// macOS GUI bundles (launched from LaunchServices / Dock / Spotlight) inherit
/// a minimal PATH (`/usr/bin:/bin:/usr/sbin:/sbin`) with no Homebrew, no
/// `~/.local/bin`, no asdf/nvm/pyenv shims. Every subprocess Plexi spawns
/// (process apps, terminals, `gh` calls from apps) inherits that broken PATH
/// too, so `shutil.which("gh") == None` even when `gh` is installed.
///
/// Fix: at startup, ask the user's login shell what its PATH is and adopt it
/// as the process PATH. Cheap and robust; falls back to a static prepend of
/// common macOS bin dirs if the shell probe fails.
///
/// Idempotent — safe to call when already launched from a terminal (the
/// login-shell probe returns the same PATH we already have).
pub fn install_login_shell_path() {
    let resolved = probe_login_shell_path().or_else(fallback_path_with_homebrew);
    if let Some(new_path) = resolved {
        log::info!("Resolved login-shell PATH: {new_path}");
        // SAFETY: called once, early in `main()`, before any subprocess spawns
        // and before any thread reads PATH. All downstream reads see the new
        // value. On non-macOS platforms the fallback returns None and we
        // leave the inherited PATH untouched.
        unsafe {
            std::env::set_var("PATH", new_path);
        }
    }
}

/// Adopt user-defined env vars from the login shell that are missing from the
/// process environment.
///
/// macOS GUI bundles only inherit a minimal environment — API keys, tokens,
/// and other secrets set in `~/.zshrc` or `~/.zsh_secrets` are invisible to
/// Plexi and every app it spawns. This probes the login shell for its full
/// `env` output and sets any var not already present in the process env.
///
/// Skips system vars (HOME, USER, SHELL, PWD, etc.) and vars already set —
/// never overwrites existing values so the GUI context wins on conflicts.
/// Called after `install_login_shell_path` since PATH is already handled.
pub fn install_login_shell_env() {
    // System/terminal vars that are either already correct in the GUI context
    // or that build_env() sets explicitly later. Never adopt these from the shell.
    const SKIP: &[&str] = &[
        "PATH",
        "HOME",
        "USER",
        "LOGNAME",
        "SHELL",
        "TMPDIR",
        "TERM",
        "TERM_PROGRAM",
        "TERM_PROGRAM_VERSION",
        "COLORTERM",
        "TERMINFO",
        "SHLVL",
        "OLDPWD",
        "PWD",
        "_",
        "PS1",
        "PS2",
        "XPC_FLAGS",
        "XPC_SERVICE_NAME",
        "APPLE_SECURITY_ASSESSMENT",
        "COMMAND_MODE",
        "SECURITYSESSIONID",
        "SSH_AUTH_SOCK",
    ];

    let Some(vars) = probe_login_shell_env() else {
        return;
    };
    let mut adopted_keys: Vec<&str> = Vec::new();
    for (k, v) in &vars {
        if SKIP.contains(&k.as_str()) {
            continue;
        }
        if std::env::var(k).is_err() {
            // SAFETY: called once, early in main(), before any threads read env.
            unsafe {
                std::env::set_var(k, v);
            }
            adopted_keys.push(k.as_str());
        }
    }
    if !adopted_keys.is_empty() {
        log::info!(
            "Adopted {} env vars from login shell: [{}]",
            adopted_keys.len(),
            adopted_keys.join(", ")
        );
    }
}

fn probe_login_shell_env() -> Option<HashMap<String, String>> {
    let shell = detect_shell();
    // `-i -l`: interactive + login. Login alone loads `~/.zprofile` /
    // `~/.zlogin` but NOT `~/.zshrc`, so secrets sourced from `.zshrc` (e.g.
    // `~/.zsh_secrets`) are invisible. Interactive forces `.zshrc` to load.
    let output = Command::new(&shell)
        .args(["-i", "-l", "-c", "env"])
        .output()
        .ok()?;
    if !output.status.success() {
        log::warn!(
            "Login-shell env probe failed: {} exited {}",
            shell,
            output.status
        );
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let mut map = HashMap::new();
    for line in text.lines() {
        // Only parse lines that look like KEY=value; skip shell functions and
        // multiline continuations from the previous key.
        if let Some((k, v)) = line.split_once('=') {
            if !k.is_empty() && k.chars().all(|c| c.is_alphanumeric() || c == '_') {
                map.insert(k.to_string(), v.to_string());
            }
        }
    }
    Some(map)
}

fn probe_login_shell_path() -> Option<String> {
    let shell = detect_shell();
    let output = Command::new(&shell)
        .args(["-l", "-c", "printf %s \"$PATH\""])
        .output()
        .ok()?;
    if !output.status.success() {
        log::warn!(
            "Login-shell PATH probe failed: {} exited {}",
            shell,
            output.status
        );
        return None;
    }
    let path = String::from_utf8(output.stdout).ok()?.trim().to_string();
    if path.is_empty() {
        return None;
    }
    Some(path)
}

fn fallback_path_with_homebrew() -> Option<String> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let current = std::env::var("PATH").unwrap_or_default();
    let extras = [
        "/opt/homebrew/bin",
        "/opt/homebrew/sbin",
        "/usr/local/bin",
        "/usr/local/sbin",
    ];
    let missing: Vec<&str> = extras
        .into_iter()
        .filter(|p| !current.split(':').any(|seg| seg == *p))
        .collect();
    if missing.is_empty() {
        return None;
    }
    let prefix = missing.join(":");
    if current.is_empty() {
        Some(prefix)
    } else {
        Some(format!("{prefix}:{current}"))
    }
}

pub fn build_env(working_directory: Option<&Path>) -> HashMap<String, String> {
    let mut env = HashMap::new();

    env.insert("TERM".into(), "xterm-256color".into());
    log::info!("shell::build_env: TERM=xterm-256color");
    env.insert("COLORTERM".into(), "truecolor".into());
    env.insert("PLEXI_RUNNING".into(), "1".into());
    if let Some(channel) = crate::config::build_channel() {
        log::info!("shell::build_env: PLEXI_CHANNEL={channel}");
        env.insert("PLEXI_CHANNEL".into(), channel);
    }

    env.insert(
        "LANG".into(),
        std::env::var("LANG").unwrap_or_else(|_| "en_US.UTF-8".into()),
    );
    env.insert(
        "LC_ALL".into(),
        std::env::var("LC_ALL").unwrap_or_else(|_| "en_US.UTF-8".into()),
    );

    // PATH: the process PATH is already resolved to the login-shell PATH at
    // startup (see `install_login_shell_path`), so inheriting it here is
    // enough — no per-shell augmentation needed.

    #[cfg(target_os = "macos")]
    {
        let workspace_root = working_directory
            .and_then(crate::app::registry::resolve_workspace_root)
            .or_else(crate::config::active_workspace_root);
        if let Some(root) = workspace_root {
            let store = crate::workspace::secrets::MacKeychain::new();
            match crate::workspace::secrets::resolve_terminal_env(&root, &store) {
                Ok(resolved) => {
                    log::info!(
                        "shell::build_env: resolved {} allowlisted terminal env secrets for workspace {}",
                        resolved.len(),
                        root.display()
                    );
                    for (key, value) in resolved {
                        env.insert(key, value.to_string());
                    }
                }
                Err(e) => {
                    log::warn!(
                        "shell::build_env: terminal env secret resolution skipped for {}: {e}",
                        root.display()
                    );
                }
            }
        }
    }

    // ZDOTDIR injection for zsh shell integration
    let shell = detect_shell();
    if shell.ends_with("/zsh") || shell.ends_with("/zsh-5") {
        match ensure_shell_integration() {
            Ok(zdotdir) => {
                let orig = std::env::var("PLEXI_ORIG_ZDOTDIR")
                    .or_else(|_| std::env::var("ZDOTDIR"))
                    .unwrap_or_else(|_| std::env::var("HOME").unwrap_or_default());
                env.insert("PLEXI_ORIG_ZDOTDIR".into(), orig);
                env.insert("ZDOTDIR".into(), zdotdir.to_string_lossy().into());
            }
            Err(e) => {
                log::warn!("Failed to set up shell integration: {e}");
            }
        }
    }

    env
}

static CWD_CACHE: LazyLock<Mutex<HashMap<u32, (PathBuf, Instant)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
const CWD_CACHE_TTL: Duration = Duration::from_millis(300);

pub fn get_pid_cwd(pid: u32) -> Option<PathBuf> {
    // Check cache first — lsof is expensive and called every frame on macOS.
    if let Ok(cache) = CWD_CACHE.lock() {
        if let Some((path, ts)) = cache.get(&pid) {
            if ts.elapsed() < CWD_CACHE_TTL {
                return Some(path.clone());
            }
        }
    }

    let result = get_pid_cwd_uncached(pid);

    if let Some(ref path) = result {
        if let Ok(mut cache) = CWD_CACHE.lock() {
            cache.insert(pid, (path.clone(), Instant::now()));
        }
    }

    result
}

fn get_pid_cwd_uncached(pid: u32) -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let output = Command::new("/usr/sbin/lsof")
            .args(["-a", "-d", "cwd", "-Fn", "-p", &pid.to_string()])
            .output()
            .ok()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if let Some(path) = line.strip_prefix('n') {
                let p = PathBuf::from(path);
                if p.is_dir() {
                    return Some(p);
                }
            }
        }
        None
    }
    #[cfg(target_os = "linux")]
    {
        std::fs::read_link(format!("/proc/{}/cwd", pid)).ok()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = pid;
        None
    }
}

fn ensure_shell_integration() -> io::Result<PathBuf> {
    let zsh_dir = crate::config::config_dir()
        .join("shell-integration")
        .join("zsh");

    std::fs::create_dir_all(&zsh_dir)?;

    let zprofile = r#"# Plexi shell integration — automatically managed, do not edit
__plexi_orig="${PLEXI_ORIG_ZDOTDIR:-$HOME}"
[[ -f "$__plexi_orig/.zprofile" ]] && source "$__plexi_orig/.zprofile"
unset __plexi_orig
"#;

    let zshrc = r#"# Plexi shell integration — automatically managed, do not edit
__plexi_orig="${PLEXI_ORIG_ZDOTDIR:-$HOME}"
[[ -f "$__plexi_orig/.zshrc" ]] && source "$__plexi_orig/.zshrc"
unset __plexi_orig

# Emit OSC 7 after each prompt so Plexi can track cwd for split inheritance
__plexi_precmd() {
    printf '\e]7;file://%s%s\a' "$HOST" "$PWD"
}
precmd_functions+=(__plexi_precmd)
"#;

    std::fs::write(zsh_dir.join(".zprofile"), zprofile)?;
    std::fs::write(zsh_dir.join(".zshrc"), zshrc)?;

    Ok(zsh_dir)
}

/// Join args into a single shell-safe string for passing to `sh -c` / `zsh -c`.
///
/// Plain `args.join(" ")` loses quote structure — `["c", "ship it"]` becomes
/// `"c ship it"` which zsh re-tokenizes into three words. This function wraps
/// any arg containing whitespace or shell metacharacters in single quotes,
/// with inner `'` escaped as `'\''`.
pub fn shell_join(args: &[String]) -> String {
    args.iter()
        .map(|a| shell_quote(a))
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn shell_quote(s: &str) -> String {
    let needs_quoting = s.is_empty()
        || s.chars().any(|c| {
            matches!(
                c,
                ' ' | '\t'
                    | '\n'
                    | '"'
                    | '\''
                    | '\\'
                    | '!'
                    | '#'
                    | '$'
                    | '&'
                    | '('
                    | ')'
                    | '*'
                    | ';'
                    | '<'
                    | '>'
                    | '?'
                    | '['
                    | ']'
                    | '^'
                    | '{'
                    | '|'
                    | '}'
                    | '~'
                    | '`'
            )
        });
    if !needs_quoting {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', r"'\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_join_plain_args_no_quoting() {
        let args = vec!["claude".to_string(), "--version".to_string()];
        assert_eq!(shell_join(&args), "claude --version");
    }

    #[test]
    fn shell_join_multi_word_arg_single_quoted() {
        // Regression: args.join(" ") on ["c", "ship something useful"] → "c ship something useful"
        // zsh re-tokenizes this to 4 words. shell_join must produce "c 'ship something useful'".
        let args = vec!["c".to_string(), "ship something useful".to_string()];
        assert_eq!(shell_join(&args), "c 'ship something useful'");
    }

    #[test]
    fn shell_join_single_quote_in_arg_escaped() {
        let args = vec!["echo".to_string(), "it's alive".to_string()];
        assert_eq!(shell_join(&args), r"echo 'it'\''s alive'");
    }

    #[test]
    fn shell_join_empty_arg_quoted() {
        let args = vec!["echo".to_string(), String::new()];
        assert_eq!(shell_join(&args), "echo ''");
    }

    #[test]
    fn shell_join_special_chars_quoted() {
        let args = vec!["echo".to_string(), "$HOME".to_string()];
        assert_eq!(shell_join(&args), "echo '$HOME'");
    }
}

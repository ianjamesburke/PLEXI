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

pub fn build_env() -> HashMap<String, String> {
    let mut env = HashMap::new();

    if let Some(terminfo_dir) = detect_ghostty_terminfo_dir() {
        env.insert("TERM".into(), "xterm-ghostty".into());
        env.insert("TERMINFO".into(), terminfo_dir);
    } else {
        env.insert("TERM".into(), "xterm-256color".into());
    }
    env.insert("COLORTERM".into(), "truecolor".into());
    env.insert("PLEXI_RUNNING".into(), "1".into());

    env.insert(
        "LANG".into(),
        std::env::var("LANG").unwrap_or_else(|_| "en_US.UTF-8".into()),
    );
    env.insert(
        "LC_ALL".into(),
        std::env::var("LC_ALL").unwrap_or_else(|_| "en_US.UTF-8".into()),
    );

    // Prepend Homebrew paths on macOS
    if cfg!(target_os = "macos") {
        let path = std::env::var("PATH").unwrap_or_default();
        if !path.contains("/opt/homebrew/bin") {
            let extra = "/opt/homebrew/bin:/opt/homebrew/sbin:/usr/local/bin:/usr/local/sbin";
            env.insert("PATH".into(), format!("{extra}:{path}"));
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

fn detect_ghostty_terminfo_dir() -> Option<String> {
    let candidates = [
        "/Applications/Ghostty.app/Contents/Resources/terminfo",
        "/opt/homebrew/Cellar/ghostty",
        "/usr/local/Cellar/ghostty",
    ];

    for candidate in candidates {
        let path = Path::new(candidate);
        if path.join("78/xterm-ghostty").is_file() {
            return Some(path.to_string_lossy().into_owned());
        }

        if let Ok(entries) = std::fs::read_dir(path) {
            for entry in entries.flatten() {
                let terminfo_path = entry.path().join("share/terminfo/78/xterm-ghostty");
                if terminfo_path.is_file() {
                    if let Some(dir) = terminfo_path.parent().and_then(Path::parent) {
                        return Some(dir.to_string_lossy().into_owned());
                    }
                }
            }
        }
    }

    None
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_shell_returns_existing_path() {
        let shell = detect_shell();
        assert!(
            Path::new(&shell).exists(),
            "detect_shell returned non-existent path: {shell}"
        );
    }

    #[test]
    fn detect_shell_returns_absolute_path() {
        let shell = detect_shell();
        assert!(
            shell.starts_with('/'),
            "detect_shell should return absolute path, got: {shell}"
        );
    }

    #[test]
    fn build_env_sets_required_vars() {
        let env = build_env();
        assert!(env.contains_key("TERM"));
        assert!(env.contains_key("COLORTERM"));
        assert_eq!(env["COLORTERM"], "truecolor");
        assert!(env.contains_key("LANG"));
        assert!(env.contains_key("LC_ALL"));
    }
}

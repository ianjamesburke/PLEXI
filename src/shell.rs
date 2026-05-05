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
        "PATH", "HOME", "USER", "LOGNAME", "SHELL", "TMPDIR",
        "TERM", "TERM_PROGRAM", "TERM_PROGRAM_VERSION", "COLORTERM", "TERMINFO",
        "SHLVL", "OLDPWD", "PWD", "_", "PS1", "PS2",
        "XPC_FLAGS", "XPC_SERVICE_NAME",
        "APPLE_SECURITY_ASSESSMENT", "COMMAND_MODE",
        "SECURITYSESSIONID", "SSH_AUTH_SOCK",
    ];

    let Some(vars) = probe_login_shell_env() else { return };
    let mut adopted_keys: Vec<&str> = Vec::new();
    for (k, v) in &vars {
        if SKIP.contains(&k.as_str()) {
            continue;
        }
        if std::env::var(k).is_err() {
            // SAFETY: called once, early in main(), before any threads read env.
            unsafe { std::env::set_var(k, v); }
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
    let extras = ["/opt/homebrew/bin", "/opt/homebrew/sbin", "/usr/local/bin", "/usr/local/sbin"];
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

    // PATH: the process PATH is already resolved to the login-shell PATH at
    // startup (see `install_login_shell_path`), so inheriting it here is
    // enough — no per-shell augmentation needed.

    // Inject user-flagged secrets as env vars
    for entry in crate::secrets::list_inject_secrets() {
        #[cfg(target_os = "macos")]
        if let Some(value) = crate::secrets::retrieve_secret(&entry.key, &entry.app_id, &entry.directory) {
            log::info!("shell::build_env: injecting secret '{}' into pane env", entry.key);
            env.insert(entry.key.clone(), value.to_string());
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


use std::collections::HashMap;
use std::io;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

const TERMINAL_ENV_NAMES_VAR: &str = "PLEXI_TERMINAL_ENV_NAMES";
const TERMINAL_ENV_VALUE_PREFIX: &str = "PLEXI_TERMINAL_ENV_VALUE_";

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

/// Run a one-shot login-shell probe fully isolated from the launching session.
///
/// Plexi may be launched attached to a terminal (`plexi` run directly, before
/// the GUI window opens). A login/interactive shell spawned to read the user's
/// env or PATH otherwise inherits that controlling terminal, so anything the
/// profile chain touches — job control, `sudo`, `tput` — can steal the user's
/// keystrokes or write onto their prompt (the documented "`zsh -i` hijacks
/// SIGINT" hazard). A footgun as ordinary as a `sleep()` shell function that
/// shells out to `sudo` then bleeds `sudo: a terminal is required` onto the
/// launching session.
///
/// `setsid` gives the probe its own session with no controlling terminal, and
/// `Command::output()` reads stdout/stderr through pipes over a null stdin — so
/// the launching session can never be polluted. Captured stderr is routed to
/// the channel log, never silently discarded.
fn run_login_shell_probe(shell: &str, args: &[&str]) -> Option<std::process::Output> {
    let mut cmd = Command::new(shell);
    cmd.args(args).stdin(std::process::Stdio::null());
    // SAFETY: pre_exec runs only libc::setsid() between fork and exec — an
    // async-signal-safe syscall with no allocation, locking, or heap access.
    #[cfg(unix)]
    unsafe {
        cmd.pre_exec(|| {
            // A forked child is never a process-group leader, so setsid()
            // succeeds and yields a new session detached from any terminal.
            if libc::setsid() == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let output = cmd.output().ok()?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stderr = stderr.trim();
    if !stderr.is_empty() {
        log::warn!(
            "Login-shell probe `{shell} {}` wrote to stderr: {stderr}",
            args.join(" ")
        );
    }
    Some(output)
}

fn probe_login_shell_env() -> Option<HashMap<String, String>> {
    let shell = detect_shell();
    // `-i -l`: interactive + login. Login alone loads `~/.zprofile` /
    // `~/.zlogin` but NOT `~/.zshrc`, so secrets sourced from `.zshrc` (e.g.
    // `~/.zsh_secrets`) are invisible. Interactive forces `.zshrc` to load.
    let output = run_login_shell_probe(&shell, &["-i", "-l", "-c", "env"])?;
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
    let output = run_login_shell_probe(&shell, &["-l", "-c", "printf %s \"$PATH\""])?;
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
            let store = crate::workspace::secrets::system_store();
            match crate::workspace::secrets::resolve_terminal_env(&root, store) {
                Ok(resolved) => {
                    log::info!(
                        "shell::build_env: resolved {} allowlisted terminal env secrets for workspace {}",
                        resolved.len(),
                        root.display()
                    );
                    let mut names = Vec::new();
                    for (key, value) in resolved {
                        if !is_shell_env_name(&key) {
                            log::warn!(
                                "shell::build_env: skipped invalid terminal env secret name {key}"
                            );
                            continue;
                        }
                        names.push(key.clone());
                        env.insert(preserved_terminal_env_name(&key), value.to_string());
                        env.insert(key, value.to_string());
                    }
                    if !names.is_empty() {
                        env.insert(TERMINAL_ENV_NAMES_VAR.into(), names.join(":"));
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

fn preserved_terminal_env_name(name: &str) -> String {
    format!("{TERMINAL_ENV_VALUE_PREFIX}{name}")
}

fn is_shell_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first == '_' || first.is_ascii_alphabetic()) {
        return false;
    }
    chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
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

/// Invoked-command name for `pid` — the basename of the path the process was
/// exec'd as. This is the name the agent detector must match on: it is the
/// command word the shell's PATH lookup resolved, which survives symlinked
/// installs.
///
/// On macOS every kernel-owned name (`proc_name`, `p_comm`, `proc_pidpath`)
/// carries the RESOLVED target of a symlink: the Homebrew codex cask installs
/// `codex -> codex-aarch64-apple-darwin`, and exec through that symlink names
/// the process `codex-aarch64-apple-darwin` (verified live on codex 0.145.0).
/// Only `KERN_PROCARGS2`'s saved exec path preserves the string actually
/// passed to `execve` (`/opt/homebrew/bin/codex`), so its basename is the
/// primary source; `proc_name` is the fallback for processes whose args block
/// is unreadable (other-uid, zombies). Shebang scripts are named after their
/// interpreter in every source including the args block — script-distributed
/// agents (pi) stay hook-covered at boot instead.
pub fn get_pid_name(pid: u32) -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        invoked_exec_basename(pid).or_else(|| {
            let mut buf = [0u8; 64];
            let n = unsafe {
                libc::proc_name(
                    pid as libc::c_int,
                    buf.as_mut_ptr() as *mut libc::c_void,
                    buf.len() as u32,
                )
            };
            (n > 0).then(|| String::from_utf8_lossy(&buf[..n as usize]).into_owned())
        })
    }
    #[cfg(target_os = "linux")]
    {
        // Linux `comm` is set from the basename of the path as passed to
        // execve — symlinks are NOT resolved — so it already is the invoked
        // name (truncated to 15 bytes, which covers every known agent).
        std::fs::read_to_string(format!("/proc/{pid}/comm"))
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = pid;
        None
    }
}

/// Basename of the as-invoked exec path from `sysctl(KERN_PROCARGS2)`.
/// Readable for same-uid processes only; `None` on any failure.
#[cfg(target_os = "macos")]
fn invoked_exec_basename(pid: u32) -> Option<String> {
    let mut argmax: libc::c_int = 0;
    let mut size = std::mem::size_of::<libc::c_int>();
    let mut mib = [libc::CTL_KERN, libc::KERN_ARGMAX];
    let rc = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            2,
            &mut argmax as *mut _ as *mut libc::c_void,
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 || argmax <= 0 {
        log::warn!("invoked_exec_basename: KERN_ARGMAX sysctl failed (rc={rc})");
        return None;
    }
    // Layout: [c_int argc][exec path NUL][NUL padding][argv[0] NUL]...
    let mut buf = vec![0u8; argmax as usize];
    let mut len = buf.len();
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid as libc::c_int];
    let rc = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            3,
            buf.as_mut_ptr() as *mut libc::c_void,
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 || len <= std::mem::size_of::<libc::c_int>() {
        return None;
    }
    let path = &buf[std::mem::size_of::<libc::c_int>()..len];
    let end = path.iter().position(|&b| b == 0)?;
    let path = std::str::from_utf8(&path[..end]).ok()?;
    let base = path.rsplit('/').next()?;
    (!base.is_empty()).then(|| base.to_string())
}

/// Map a foreground process name to the canonical agent name the lifecycle
/// hooks report (`PLEXI_AGENT_NAME`). This is the identity half of the shared
/// agent detector: hooks are authoritative once they fire, but not every
/// agent CLI emits a hook at boot (Codex defers `session_start` to the first
/// prompt submission), so the host also recognizes the agent by its process
/// name. Interpreter-launched agents are resolved by [`get_pid_agent_probe`],
/// which applies this same table to their script argv.
pub fn known_agent_process(name: &str) -> Option<&'static str> {
    match name {
        "claude" => Some("claude-code"),
        "codex" => Some("codex"),
        "pi" => Some("pi"),
        _ => None,
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum AgentProcessProbe {
    Known(&'static str),
    UnsupportedInterpreter {
        interpreter: String,
        script: Option<String>,
    },
    Unknown,
}

pub fn get_pid_agent_probe(pid: u32) -> AgentProcessProbe {
    let Some(process_name) = get_pid_name(pid) else {
        return AgentProcessProbe::Unknown;
    };
    if let Some(agent) = known_agent_process(&process_name) {
        return AgentProcessProbe::Known(agent);
    }
    if !known_agent_interpreter(&process_name) {
        return AgentProcessProbe::Unknown;
    }
    let args = get_pid_argv(pid).unwrap_or_default();
    let script = args
        .iter()
        .find(|arg| {
            let base = std::path::Path::new(arg)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(arg);
            !arg.starts_with('-')
                && base != process_name
                && !known_agent_interpreter(base)
                && base != "env"
        })
        .cloned();
    if let Some(agent) = script.as_deref().and_then(known_agent_script) {
        return AgentProcessProbe::Known(agent);
    }
    AgentProcessProbe::UnsupportedInterpreter {
        interpreter: process_name,
        script,
    }
}

fn known_agent_interpreter(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    matches!(name.as_str(), "node" | "nodejs" | "python" | "python3")
        || name
            .strip_prefix("python")
            .is_some_and(|suffix| suffix.starts_with(|c: char| c.is_ascii_digit()))
}

fn known_agent_script(script: &str) -> Option<&'static str> {
    let path = std::path::Path::new(script);
    let base = path.file_stem()?.to_str()?;
    if let Some(agent) = known_agent_process(base) {
        return Some(agent);
    }
    let normalized = script.replace('\\', "/");
    if normalized.contains("/pi-coding-agent/") {
        Some("pi")
    } else if normalized.contains("/claude-code/") || normalized.contains("/@anthropic-ai/") {
        Some("claude-code")
    } else if normalized.contains("/@openai/codex/") {
        Some("codex")
    } else {
        None
    }
}

#[cfg(target_os = "macos")]
fn get_pid_argv(pid: u32) -> Option<Vec<String>> {
    let mut argmax: libc::c_int = 0;
    let mut size = std::mem::size_of::<libc::c_int>();
    let mut mib = [libc::CTL_KERN, libc::KERN_ARGMAX];
    if unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            2,
            &mut argmax as *mut _ as *mut libc::c_void,
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    } != 0
        || argmax <= 0
    {
        return None;
    }
    let mut buf = vec![0u8; argmax as usize];
    let mut len = buf.len();
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid as libc::c_int];
    if unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            3,
            buf.as_mut_ptr() as *mut libc::c_void,
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    } != 0
        || len <= std::mem::size_of::<libc::c_int>()
    {
        return None;
    }
    let argc =
        libc::c_int::from_ne_bytes(buf[..std::mem::size_of::<libc::c_int>()].try_into().ok()?);
    if argc <= 0 {
        return None;
    }
    let bytes = &buf[std::mem::size_of::<libc::c_int>()..len];
    let exec_end = bytes.iter().position(|byte| *byte == 0)?;
    let mut cursor = exec_end + 1;
    while cursor < bytes.len() && bytes[cursor] == 0 {
        cursor += 1;
    }
    let mut args = Vec::with_capacity(argc as usize);
    while args.len() < argc as usize && cursor < bytes.len() {
        let end = bytes[cursor..].iter().position(|byte| *byte == 0)? + cursor;
        args.push(String::from_utf8_lossy(&bytes[cursor..end]).into_owned());
        cursor = end + 1;
    }
    Some(args)
}

#[cfg(target_os = "linux")]
fn get_pid_argv(pid: u32) -> Option<Vec<String>> {
    let bytes = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    Some(
        bytes
            .split(|byte| *byte == 0)
            .filter(|arg| !arg.is_empty())
            .map(|arg| String::from_utf8_lossy(arg).into_owned())
            .collect(),
    )
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn get_pid_argv(pid: u32) -> Option<Vec<String>> {
    let _ = pid;
    None
}

/// True when `pid` has at least one live child process. Used as one of the
/// two terminal-activity signals (alongside the PTY foreground pgid): a
/// shell with children is running something — a script, a server, a REPL.
pub fn pid_has_children(pid: u32) -> bool {
    #[cfg(target_os = "macos")]
    {
        // One pid slot is enough — we only care whether any child exists.
        let mut buf = [0i32; 1];
        let n = unsafe {
            libc::proc_listchildpids(
                pid as libc::pid_t,
                buf.as_mut_ptr() as *mut libc::c_void,
                std::mem::size_of_val(&buf) as libc::c_int,
            )
        };
        n > 0
    }
    #[cfg(target_os = "linux")]
    {
        std::fs::read_to_string(format!("/proc/{pid}/task/{pid}/children"))
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = pid;
        false
    }
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

# Re-apply workspace terminal env after user startup files so workspace-scoped
# secrets win over global exports in .zshrc/.zprofile.
if [[ -n "${PLEXI_TERMINAL_ENV_NAMES:-}" ]]; then
    for __plexi_env_name in ${(s.:.)PLEXI_TERMINAL_ENV_NAMES}; do
        __plexi_value_var="PLEXI_TERMINAL_ENV_VALUE_${__plexi_env_name}"
        if [[ -n "${(P)__plexi_value_var+x}" ]]; then
            export "${__plexi_env_name}=${(P)__plexi_value_var}"
            unset "$__plexi_value_var"
        fi
    done
    unset PLEXI_TERMINAL_ENV_NAMES __plexi_env_name __plexi_value_var
fi

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

    #[test]
    fn login_shell_probe_captures_stderr_not_inherited() {
        // Regression (stint 0443): a probe shell that writes to stderr must have
        // that output captured in `Output`, never inherited onto the launching
        // session's fds. A dotfiles footgun like a `sleep()` function shelling
        // out to `sudo` used to bleed `sudo: a terminal is required` onto the
        // terminal Plexi was launched from.
        let out = run_login_shell_probe("/bin/sh", &["-c", "printf out; printf err 1>&2"]).unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), "out");
        assert_eq!(String::from_utf8_lossy(&out.stderr).trim(), "err");
    }

    #[test]
    fn login_shell_probe_stdin_is_null_no_hang() {
        // Regression (stint 0443): stdin is redirected to /dev/null, so a probe
        // shell that reads stdin hits EOF immediately instead of blocking on —
        // or stealing keystrokes from — the launching terminal.
        let out = run_login_shell_probe("/bin/sh", &["-c", "cat; printf done"]).unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), "done");
    }

    #[test]
    fn shell_env_name_validation_accepts_posix_names() {
        assert!(is_shell_env_name("OPENROUTER_API_KEY"));
        assert!(is_shell_env_name("_PLEXI_TEST"));
        assert!(is_shell_env_name("A1"));
    }

    #[test]
    fn shell_env_name_validation_rejects_unsafe_names() {
        assert!(!is_shell_env_name(""));
        assert!(!is_shell_env_name("1PASSWORD"));
        assert!(!is_shell_env_name("BAD-NAME"));
        assert!(!is_shell_env_name("BAD:NAME"));
        assert!(!is_shell_env_name("BAD NAME"));
    }

    #[test]
    fn known_agent_process_maps_canonical_names() {
        assert_eq!(known_agent_process("codex"), Some("codex"));
        assert_eq!(known_agent_process("claude"), Some("claude-code"));
        assert_eq!(known_agent_process("pi"), Some("pi"));
        assert_eq!(known_agent_process("zsh"), None);
        assert_eq!(known_agent_process("vim"), None);
        // Exact match on the INVOKED name only — `get_pid_name` owns
        // normalizing a symlinked install back to its invoked basename, so
        // a resolved payload name must never reach (or match) this table.
        assert_eq!(known_agent_process("codex-aarch64-apple-darwin"), None);
    }

    #[cfg(unix)]
    #[test]
    fn interpreter_script_argv_identifies_agent_through_shebang_symlink() {
        let dir = std::env::temp_dir().join(format!(
            "plexi-interpreter-agent-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let script = dir.join("cli.py");
        std::fs::write(
            &script,
            "#!/usr/bin/env python3\nimport time\ntime.sleep(30)\n",
        )
        .expect("write script");
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&script).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script, perms).unwrap();
        let alias = dir.join("pi");
        std::os::unix::fs::symlink(&script, &alias).expect("symlink");

        let mut child = std::process::Command::new(&alias)
            .spawn()
            .expect("spawn interpreter-shaped agent");
        std::thread::sleep(std::time::Duration::from_millis(100));
        let probe = get_pid_agent_probe(child.id());
        let process_name = get_pid_name(child.id());
        let argv = get_pid_argv(child.id());
        let _ = child.kill();
        let _ = child.wait();

        assert_eq!(
            probe,
            AgentProcessProbe::Known("pi"),
            "process_name={process_name:?} argv={argv:?}"
        );
    }

    #[test]
    fn get_pid_name_reports_the_exec_basename() {
        let mut child = std::process::Command::new("/bin/sleep")
            .arg("5")
            .spawn()
            .expect("spawn sleep");
        let name = get_pid_name(child.id());
        let _ = child.kill();
        let _ = child.wait();
        assert_eq!(name.as_deref(), Some("sleep"));
    }

    /// The Homebrew codex cask is a symlink (`codex ->
    /// codex-aarch64-apple-darwin`), and macOS names a process exec'd through
    /// a symlink after the RESOLVED target — so identity must come from the
    /// as-invoked path, not `proc_name`. This pins the exact failure that
    /// made a booted Codex invisible to the agent detector (PR #2510 round 2).
    #[cfg(unix)]
    #[test]
    fn get_pid_name_reports_the_invoked_name_through_a_symlink() {
        let dir = std::env::temp_dir().join(format!("plexi-symlink-name-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let link = dir.join("codex");
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink("/bin/sleep", &link).expect("symlink");
        let mut child = std::process::Command::new(&link)
            .arg("30")
            .spawn()
            .expect("spawn through symlink");
        let name = get_pid_name(child.id());
        let _ = child.kill();
        let _ = child.wait();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(name.as_deref(), Some("codex"));
    }

    #[test]
    fn get_pid_name_none_for_dead_pid() {
        let mut child = std::process::Command::new("/bin/sleep")
            .arg("0")
            .spawn()
            .expect("spawn sleep");
        let _ = child.wait();
        assert_eq!(get_pid_name(child.id()), None);
    }
}

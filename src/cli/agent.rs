use std::path::{Path, PathBuf};

const CLAUDE_CODE_HOOK_EVENTS: &[&str] = &[
    "PreToolUse",
    "SessionStart",
    "UserPromptSubmit",
    "PermissionRequest",
    "PostToolUse",
    "PostToolBatch",
    "Stop",
    "StopFailure",
    "SessionEnd",
];

const CODEX_HOOK_EVENTS: &[&str] = &[
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PermissionRequest",
    "PostToolUse",
    "Stop",
];

const PLEXI_AGENT_STATE_SCRIPT: &str = "claude-code-agent-state.sh";
const PLEXI_PI_EXTENSION: &str = "plexi-agent-state.ts";
const PLEXI_CLAUDE_SETTINGS: &str = "plexi-claude-settings.json";

/// Resolve the workspace root from the current directory.
pub(crate) fn resolve_workspace_cwd() -> Result<std::path::PathBuf, i32> {
    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: could not determine current directory: {e}");
            return Err(1);
        }
    };
    match crate::app::registry::resolve_workspace_root(&cwd) {
        Some(root) => Ok(root),
        None => {
            eprintln!("{}", workspace_not_found_error(&cwd));
            Err(1)
        }
    }
}

fn workspace_not_found_error(cwd: &Path) -> String {
    format!(
        "error: no {}/ workspace found at or above {}.\n  Run `plexi workspace init` first.",
        crate::config::workspace_channel_dir(),
        cwd.display()
    )
}

/// Multi-CLI harness identifiers supported natively by Plexi.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentHarness {
    Claude,
    Codex,
    Pi,
    Agy,
    Script,
}

impl AgentHarness {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "claude" | "claude-code" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            "pi" => Some(Self::Pi),
            "agy" | "antigravity" => Some(Self::Agy),
            "script" | "sh" | "bash" => Some(Self::Script),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Pi => "pi",
            Self::Agy => "agy",
            Self::Script => "script",
        }
    }

    pub fn adapter(self) -> &'static HarnessAdapter {
        HARNESS_ADAPTERS
            .iter()
            .find(|a| a.harness == self)
            .expect("all AgentHarness variants exist in HARNESS_ADAPTERS")
    }

    pub fn resume_command_with_flags(self, session_id: &str, flags: &[String]) -> String {
        self.adapter().resume_command_with_flags(session_id, flags)
    }

    pub fn restore_command_with_flags(self, session_id: Option<&str>, flags: &[String]) -> String {
        self.adapter().restore_command_with_flags(session_id, flags)
    }
}

#[cfg(test)]
thread_local! {
    static TEST_PI_SESSIONS_DIR_OVERRIDE: std::cell::RefCell<Vec<(u64, PathBuf)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

#[cfg(test)]
static NEXT_TEST_PI_SESSIONS_GUARD_TOKEN: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(1);

#[cfg(test)]
pub struct TestPiSessionsDirGuard {
    token: u64,
}

#[cfg(test)]
impl Drop for TestPiSessionsDirGuard {
    fn drop(&mut self) {
        let token = self.token;
        TEST_PI_SESSIONS_DIR_OVERRIDE.with(|c| c.borrow_mut().retain(|(t, _)| *t != token));
    }
}

#[cfg(test)]
pub fn set_test_pi_sessions_dir(path: PathBuf) -> TestPiSessionsDirGuard {
    let token =
        NEXT_TEST_PI_SESSIONS_GUARD_TOKEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    TEST_PI_SESSIONS_DIR_OVERRIDE.with(|c| c.borrow_mut().push((token, path)));
    TestPiSessionsDirGuard { token }
}

fn pi_sessions_dir() -> Option<PathBuf> {
    #[cfg(test)]
    if let Some(override_dir) =
        TEST_PI_SESSIONS_DIR_OVERRIDE.with(|c| c.borrow().last().map(|(_, p)| p.clone()))
    {
        return Some(override_dir);
    }
    dirs::home_dir().map(|h| h.join(".pi").join("agent").join("sessions"))
}

/// Return true when `session_id` resolves to a persisted Pi session transcript on disk,
/// either as a direct file path or matching `<sid>.jsonl` / `*_<sid>.jsonl` under
/// `sessions_dir` (`~/.pi/agent/sessions/` and its project subdirectories).
pub fn pi_session_exists_in(session_id: &str, sessions_dir: Option<&Path>) -> bool {
    let Some(sid) = non_empty_string(session_id) else {
        return false;
    };
    if Path::new(sid).is_file() {
        return true;
    }
    let Some(root) = sessions_dir else {
        return false;
    };
    if !root.is_dir() {
        return false;
    }
    let exact_name = format!("{sid}.jsonl");
    let suffix_name = format!("_{sid}.jsonl");
    let matches_file = |path: &Path| -> bool {
        if !path.is_file() {
            return false;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            return false;
        };
        name == exact_name || name.ends_with(&suffix_name)
    };
    let Ok(entries) = std::fs::read_dir(root) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Ok(sub_entries) = std::fs::read_dir(&path) {
                for sub in sub_entries.flatten() {
                    if matches_file(&sub.path()) {
                        return true;
                    }
                }
            }
        } else if matches_file(&path) {
            return true;
        }
    }
    false
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptionArgKind {
    Flag,
    Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HarnessOptionSpec {
    pub name: &'static str,
    pub kind: OptionArgKind,
    pub description: &'static str,
}

pub static CLAUDE_SUPPORTED_OPTIONS: &[HarnessOptionSpec] = &[
    HarnessOptionSpec { name: "--verbose", kind: OptionArgKind::Flag, description: "Enable verbose output" },
    HarnessOptionSpec { name: "-v", kind: OptionArgKind::Flag, description: "Enable verbose output" },
    HarnessOptionSpec { name: "--effort", kind: OptionArgKind::Value, description: "Reasoning effort level" },
    HarnessOptionSpec { name: "--thinking", kind: OptionArgKind::Flag, description: "Enable extended thinking" },
    HarnessOptionSpec { name: "--max-thinking-tokens", kind: OptionArgKind::Value, description: "Maximum thinking tokens" },
    HarnessOptionSpec { name: "--budget", kind: OptionArgKind::Value, description: "Cost budget limit" },
    HarnessOptionSpec { name: "--permission-mode", kind: OptionArgKind::Value, description: "Permission prompt mode" },
    HarnessOptionSpec { name: "--theme", kind: OptionArgKind::Value, description: "UI theme" },
    HarnessOptionSpec { name: "--timeout", kind: OptionArgKind::Value, description: "Execution timeout in seconds" },
    HarnessOptionSpec { name: "--fallback-model", kind: OptionArgKind::Value, description: "Fallback model identifier" },
    HarnessOptionSpec { name: "--mcp-config", kind: OptionArgKind::Value, description: "Path to MCP configuration file" },
    HarnessOptionSpec { name: "--header", kind: OptionArgKind::Value, description: "Custom HTTP header" },
    HarnessOptionSpec { name: "--model", kind: OptionArgKind::Value, description: "Model identifier override" },
];

pub static CLAUDE_RESERVED_OPTIONS: &[&str] = &[
    "-p", "--print", "exec", "--prompt", "--resume", "--settings", "--append-system-prompt", "--system-prompt",
];

pub static CLAUDE_SECURITY_OVERRIDES: &[&str] = &[
    "--dangerously-skip-permissions", "--allow-dangerously-skip-permissions", "--skip-permissions",
    "--no-sandbox", "--disable-sandbox", "--dangerously-bypass", "--bypass-all", "--full-auto",
];

pub static PI_SUPPORTED_OPTIONS: &[HarnessOptionSpec] = &[
    HarnessOptionSpec { name: "--verbose", kind: OptionArgKind::Flag, description: "Enable verbose output" },
    HarnessOptionSpec { name: "-v", kind: OptionArgKind::Flag, description: "Enable verbose output" },
    HarnessOptionSpec { name: "--debug", kind: OptionArgKind::Flag, description: "Enable debug logging" },
    HarnessOptionSpec { name: "--thinking", kind: OptionArgKind::Flag, description: "Enable model thinking" },
    HarnessOptionSpec { name: "--temperature", kind: OptionArgKind::Value, description: "Sampling temperature" },
    HarnessOptionSpec { name: "--max-tokens", kind: OptionArgKind::Value, description: "Maximum completion tokens" },
    HarnessOptionSpec { name: "--top-p", kind: OptionArgKind::Value, description: "Nucleus sampling top-p" },
    HarnessOptionSpec { name: "--tools", kind: OptionArgKind::Value, description: "Allowed tool list" },
    HarnessOptionSpec { name: "--theme", kind: OptionArgKind::Value, description: "UI theme" },
    HarnessOptionSpec { name: "--profile", kind: OptionArgKind::Value, description: "Profile directory or name" },
    HarnessOptionSpec { name: "-e", kind: OptionArgKind::Value, description: "Extension path" },
    HarnessOptionSpec { name: "--extension", kind: OptionArgKind::Value, description: "Extension path" },
    HarnessOptionSpec { name: "--model", kind: OptionArgKind::Value, description: "Model identifier override" },
];

pub static PI_RESERVED_OPTIONS: &[&str] = &[
    "-p", "--print", "exec", "--prompt", "--session", "--append-system-prompt", "--no-extensions", "--no-mcp",
];

pub static PI_SECURITY_OVERRIDES: &[&str] = &[
    "--dangerously-skip-permissions", "--no-sandbox", "--disable-sandbox", "--dangerously-bypass", "--bypass-all", "--full-auto",
];

pub static CODEX_SUPPORTED_OPTIONS: &[HarnessOptionSpec] = &[
    HarnessOptionSpec { name: "--verbose", kind: OptionArgKind::Flag, description: "Enable verbose output" },
    HarnessOptionSpec { name: "-v", kind: OptionArgKind::Flag, description: "Enable verbose output" },
    HarnessOptionSpec { name: "--debug", kind: OptionArgKind::Flag, description: "Enable debug mode" },
    HarnessOptionSpec { name: "--search", kind: OptionArgKind::Flag, description: "Enable web search" },
    HarnessOptionSpec { name: "--web-search", kind: OptionArgKind::Flag, description: "Enable web search" },
    HarnessOptionSpec { name: "--temperature", kind: OptionArgKind::Value, description: "Sampling temperature" },
    HarnessOptionSpec { name: "--max-tokens", kind: OptionArgKind::Value, description: "Maximum completion tokens" },
    HarnessOptionSpec { name: "--top-p", kind: OptionArgKind::Value, description: "Nucleus sampling top-p" },
    HarnessOptionSpec { name: "--color", kind: OptionArgKind::Value, description: "Color formatting mode" },
    HarnessOptionSpec { name: "-c", kind: OptionArgKind::Value, description: "Configuration key=value" },
    HarnessOptionSpec { name: "--config", kind: OptionArgKind::Value, description: "Configuration key=value" },
    HarnessOptionSpec { name: "--model", kind: OptionArgKind::Value, description: "Model identifier override" },
];

pub static CODEX_RESERVED_OPTIONS: &[&str] = &[
    "-p", "--prompt", "exec", "resume",
];

pub static CODEX_SECURITY_OVERRIDES: &[&str] = &[
    "--sandbox", "--no-sandbox", "--disable-sandbox", "--dangerously-bypass", "--bypass-all", "--full-auto",
];

pub static AGY_SUPPORTED_OPTIONS: &[HarnessOptionSpec] = &[
    HarnessOptionSpec { name: "--verbose", kind: OptionArgKind::Flag, description: "Enable verbose output" },
    HarnessOptionSpec { name: "-v", kind: OptionArgKind::Flag, description: "Enable verbose output" },
    HarnessOptionSpec { name: "--debug", kind: OptionArgKind::Flag, description: "Enable debug mode" },
    HarnessOptionSpec { name: "--thinking", kind: OptionArgKind::Flag, description: "Enable model thinking" },
    HarnessOptionSpec { name: "--temperature", kind: OptionArgKind::Value, description: "Sampling temperature" },
    HarnessOptionSpec { name: "--max-tokens", kind: OptionArgKind::Value, description: "Maximum completion tokens" },
    HarnessOptionSpec { name: "--mode", kind: OptionArgKind::Value, description: "Operational mode" },
    HarnessOptionSpec { name: "--theme", kind: OptionArgKind::Value, description: "UI theme" },
    HarnessOptionSpec { name: "--model", kind: OptionArgKind::Value, description: "Model identifier override" },
];

pub static AGY_RESERVED_OPTIONS: &[&str] = &[
    "-p", "--prompt", "exec", "--resume", "--prompt-interactive",
];

pub static AGY_SECURITY_OVERRIDES: &[&str] = &[
    "--dangerously-skip-permissions", "--no-sandbox", "--disable-sandbox", "--dangerously-bypass", "--bypass-all", "--full-auto", "--bypass-checks",
];

pub static SCRIPT_SUPPORTED_OPTIONS: &[HarnessOptionSpec] = &[
    HarnessOptionSpec { name: "-e", kind: OptionArgKind::Flag, description: "Exit immediately on nonzero status" },
    HarnessOptionSpec { name: "-u", kind: OptionArgKind::Flag, description: "Treat unset variables as error" },
    HarnessOptionSpec { name: "-x", kind: OptionArgKind::Flag, description: "Print commands as executed" },
    HarnessOptionSpec { name: "-v", kind: OptionArgKind::Flag, description: "Print shell input lines as read" },
    HarnessOptionSpec { name: "-n", kind: OptionArgKind::Flag, description: "Read commands without executing" },
    HarnessOptionSpec { name: "-o", kind: OptionArgKind::Value, description: "Set shell option (pipefail, errexit, nounset)" },
];

pub static SCRIPT_RESERVED_OPTIONS: &[&str] = &[
    "-c", "-i", "-s",
];

pub static SCRIPT_SECURITY_OVERRIDES: &[&str] = &[];

/// First-class adapter mapping a CLI agent harness to its spawn, session-ID capture,
/// and resume command conventions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HarnessAdapter {
    pub harness: AgentHarness,
    pub binary: &'static str,
    pub task_flag: &'static str,
    pub resume_subcommand_or_flag: &'static str,
    pub supported_options: &'static [HarnessOptionSpec],
    pub reserved_options: &'static [&'static str],
    pub security_overrides: &'static [&'static str],
}

pub const HARNESS_ADAPTERS: [HarnessAdapter; 5] = [
    HarnessAdapter {
        harness: AgentHarness::Claude,
        binary: "claude",
        task_flag: "",
        resume_subcommand_or_flag: "--resume",
        supported_options: CLAUDE_SUPPORTED_OPTIONS,
        reserved_options: CLAUDE_RESERVED_OPTIONS,
        security_overrides: CLAUDE_SECURITY_OVERRIDES,
    },
    HarnessAdapter {
        harness: AgentHarness::Codex,
        binary: "codex",
        task_flag: "",
        resume_subcommand_or_flag: "resume",
        supported_options: CODEX_SUPPORTED_OPTIONS,
        reserved_options: CODEX_RESERVED_OPTIONS,
        security_overrides: CODEX_SECURITY_OVERRIDES,
    },
    HarnessAdapter {
        harness: AgentHarness::Pi,
        binary: "pi",
        task_flag: "",
        resume_subcommand_or_flag: "--session",
        supported_options: PI_SUPPORTED_OPTIONS,
        reserved_options: PI_RESERVED_OPTIONS,
        security_overrides: PI_SECURITY_OVERRIDES,
    },
    HarnessAdapter {
        harness: AgentHarness::Agy,
        binary: "agy",
        task_flag: "--prompt-interactive",
        resume_subcommand_or_flag: "--resume",
        supported_options: AGY_SUPPORTED_OPTIONS,
        reserved_options: AGY_RESERVED_OPTIONS,
        security_overrides: AGY_SECURITY_OVERRIDES,
    },
    HarnessAdapter {
        harness: AgentHarness::Script,
        binary: "/bin/sh",
        task_flag: "-c",
        resume_subcommand_or_flag: "-c",
        supported_options: SCRIPT_SUPPORTED_OPTIONS,
        reserved_options: SCRIPT_RESERVED_OPTIONS,
        security_overrides: SCRIPT_SECURITY_OVERRIDES,
    },
];

pub fn harness_adapter(name: &str) -> Option<&'static HarnessAdapter> {
    let kind = AgentHarness::parse(name)?;
    HARNESS_ADAPTERS.iter().find(|a| a.harness == kind)
}

fn shell_quote_arg(s: &str) -> String {
    if !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | ':'))
    {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', "'\\''"))
}

pub fn redact_harness_flags(flags: &[String]) -> Vec<String> {
    let mut out = Vec::with_capacity(flags.len());
    let mut hide_next = false;
    for flag in flags {
        if hide_next {
            hide_next = false;
            out.push("[REDACTED]".to_string());
            continue;
        }
        let lower = flag.to_ascii_lowercase();
        if lower.starts_with("--token")
            || lower.starts_with("--api-key")
            || lower.starts_with("--password")
            || lower.starts_with("--secret")
            || lower == "--header"
            || lower == "-h"
        {
            if let Some((k, _)) = flag.split_once('=') {
                out.push(format!("{k}=[REDACTED]"));
            } else {
                hide_next = true;
                out.push(flag.clone());
            }
            continue;
        }
        if let Some((k, v)) = flag.split_once('=') {
            let k_lower = k.to_ascii_lowercase();
            if k_lower.contains("token")
                || k_lower.contains("secret")
                || k_lower.contains("key")
                || k_lower.contains("password")
                || k_lower.contains("auth")
            {
                out.push(format!("{k}=[REDACTED]"));
                continue;
            }
            let v_lower = v.to_ascii_lowercase();
            let nested_key_is_sensitive = v
                .split_once('=')
                .map(|(nested_key, _)| {
                    let nested_key = nested_key.to_ascii_lowercase();
                    nested_key.contains("token")
                        || nested_key.contains("secret")
                        || nested_key.contains("key")
                        || nested_key.contains("password")
                        || nested_key.contains("auth")
                })
                .unwrap_or(false);
            if v_lower.contains("bearer")
                || nested_key_is_sensitive
                || v.starts_with("ghp_")
                || v.starts_with("sk-")
                || v.starts_with("sec_")
            {
                out.push(format!("{k}=[REDACTED]"));
                continue;
            }
        }
        let v_lower = flag.to_ascii_lowercase();
        if v_lower.contains("bearer")
            || flag.starts_with("ghp_")
            || flag.starts_with("sk-")
            || flag.starts_with("sec_")
        {
            out.push("[REDACTED]".to_string());
            continue;
        }
        out.push(flag.clone());
    }
    out
}

fn validate_injection_characters(arg: &str) -> Result<(), String> {
    if arg.contains('\0') {
        return Err("flag contains forbidden null byte".to_string());
    }
    if arg.contains('\n') || arg.contains('\r') {
        return Err("flag contains forbidden newline character".to_string());
    }
    for op in [";", "&&", "||", "|", "`", "$(", ">", "<"] {
        if arg.contains(op) {
            return Err(format!("flag '{arg}' contains forbidden shell operator '{op}'"));
        }
    }
    if arg.ends_with('&') || arg.contains(" & ") {
        return Err(format!("flag '{arg}' contains forbidden shell operator '&'"));
    }
    Ok(())
}

pub fn validate_custom_harness_flags(
    adapter: &HarnessAdapter,
    flags: &[String],
    configured_model: Option<&str>,
) -> Result<Vec<String>, String> {
    if flags.is_empty() {
        return Ok(Vec::new());
    }

    let mut i = 0;
    while i < flags.len() {
        let raw = &flags[i];
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(format!("empty flag at index {i}"));
        }

        validate_injection_characters(raw)?;

        let (flag_key, inline_val) = match raw.split_once('=') {
            Some((k, v)) => (k, Some(v)),
            None => (raw.as_str(), None),
        };

        // Reserved security overrides. Plexi owns sandbox and approval authority,
        // so value-taking controls are rejected as a class rather than allowing a
        // future permissive value to slip through this settings surface.
        let is_sec_override = adapter.security_overrides.contains(&flag_key)
            || matches!(
                flag_key,
                "--dangerously-skip-permissions"
                    | "--allow-dangerously-skip-permissions"
                    | "--skip-permissions"
                    | "--dangerously-bypass"
                    | "--bypass-all"
                    | "--full-auto"
                    | "--no-sandbox"
                    | "--disable-sandbox"
                    | "--sandbox"
                    | "--ask-for-approval"
            );
        if is_sec_override {
            return Err(format!(
                "flag '{raw}' is a reserved security override and cannot be configured"
            ));
        }

        if adapter.harness == AgentHarness::Claude && flag_key == "--permission-mode" {
            let permission_mode = inline_val.or_else(|| flags.get(i + 1).map(String::as_str));
            if permission_mode.is_some_and(|mode| {
                let mode = mode.to_ascii_lowercase();
                mode.contains("bypass") || mode == "dontask"
            }) {
                return Err(format!(
                    "permission mode in flag '{raw}' is a reserved security override and cannot be configured"
                ));
            }
        }

        // Reserved lifecycle overrides
        let is_reserved = adapter.reserved_options.contains(&flag_key)
            || matches!(flag_key, "-p" | "--print" | "exec" | "--prompt");
        if is_reserved {
            return Err(format!(
                "flag '{raw}' is reserved for Plexi lifecycle authority"
            ));
        }

        // Codex specific config validation
        if adapter.harness == AgentHarness::Codex && (flag_key == "-c" || flag_key == "--config") {
            let config_val = if let Some(val) = inline_val {
                val
            } else if i + 1 < flags.len() {
                &flags[i + 1]
            } else {
                return Err(format!("missing required value for flag '{raw}'"));
            };
            let (cfg_key, _) = config_val.split_once('=').unwrap_or((config_val, ""));
            let cfg_key = cfg_key.trim();
            if cfg_key == "developer_instructions" {
                return Err(format!(
                    "configuration key '{cfg_key}' in flag '{raw}' is reserved for Plexi persona authority"
                ));
            }
            if cfg_key.starts_with("sandbox")
                || cfg_key.starts_with("approval_policy")
                || cfg_key.starts_with("ask_for_approval")
            {
                return Err(format!(
                    "configuration key '{cfg_key}' in flag '{raw}' is reserved for Plexi sandbox authority"
                ));
            }
        }

        // Conflict with configured model
        if flag_key == "--model" {
            if let Some(cm) = configured_model {
                return Err(format!(
                    "flag '{raw}' conflicts with configured model '{cm}' in settings.toml"
                ));
            }
        }

        if !flag_key.starts_with('-') {
            return Err(format!(
                "unexpected positional argument '{raw}' in flags; flags must begin with '-'"
            ));
        }

        let spec = adapter
            .supported_options
            .iter()
            .find(|opt| opt.name == flag_key);

        let Some(opt_spec) = spec else {
            return Err(format!(
                "adapter '{}' does not support flag '{flag_key}' (see `plexi agent flags {}`)",
                adapter.harness.as_str(),
                adapter.harness.as_str()
            ));
        };

        match opt_spec.kind {
            OptionArgKind::Flag => {
                if inline_val.is_some() {
                    return Err(format!("flag '{flag_key}' does not take a value"));
                }
                i += 1;
            }
            OptionArgKind::Value => {
                if let Some(val) = inline_val {
                    if val.trim().is_empty() {
                        return Err(format!("missing required value for flag '{raw}'"));
                    }
                    validate_injection_characters(val)?;
                    i += 1;
                } else if i + 1 < flags.len() {
                    let next_val = &flags[i + 1];
                    if next_val.trim().is_empty() {
                        return Err(format!("missing required value for flag '{raw}'"));
                    }
                    validate_injection_characters(next_val)?;
                    i += 2;
                } else {
                    return Err(format!("missing required value for flag '{raw}'"));
                }
            }
        }
    }

    Ok(flags.to_vec())
}

impl HarnessAdapter {
    pub fn resume_command_with_flags(&self, session_id: &str, flags: &[String]) -> String {
        let sid = shell_quote_arg(session_id.trim());
        let safe_flags: Vec<String> = flags.iter().map(|f| shell_quote_arg(f)).collect();
        let flags_str = if safe_flags.is_empty() {
            String::new()
        } else {
            format!("{} ", safe_flags.join(" "))
        };
        if self.harness == AgentHarness::Script {
            format!("{} {}-c {}", self.binary, flags_str, sid)
        } else {
            format!(
                "{} {}{} {}",
                self.binary, flags_str, self.resume_subcommand_or_flag, sid
            )
        }
    }

    /// Build the CLI command to restore an agent pane on workspace startup or standby wake.
    /// Falls back to a bare harness binary launch when `session_id` is absent or when a
    /// `pi` session ID has no persisted session file on disk.
    pub fn restore_command_with_flags(
        &self,
        session_id: Option<&str>,
        flags: &[String],
    ) -> String {
        let dir = pi_sessions_dir();
        self.restore_command_with_flags_in(session_id, flags, dir.as_deref())
    }

    #[allow(dead_code)]
    pub fn restore_command_in(
        &self,
        session_id: Option<&str>,
        pi_sessions_root: Option<&Path>,
    ) -> String {
        self.restore_command_with_flags_in(session_id, &[], pi_sessions_root)
    }

    pub fn restore_command_with_flags_in(
        &self,
        session_id: Option<&str>,
        flags: &[String],
        pi_sessions_root: Option<&Path>,
    ) -> String {
        let safe_flags: Vec<String> = flags.iter().map(|f| shell_quote_arg(f)).collect();
        let flags_prefix = if safe_flags.is_empty() {
            String::new()
        } else {
            format!(" {}", safe_flags.join(" "))
        };
        let Some(sid) = session_id.and_then(non_empty_string) else {
            return format!("{}{flags_prefix}", self.binary);
        };
        if self.harness == AgentHarness::Pi && !pi_session_exists_in(sid, pi_sessions_root) {
            log::warn!(
                "agent_restore: Pi session '{}' has no persisted session file on disk; falling back to bare '{}' launch",
                sid,
                self.binary
            );
            return format!("{}{flags_prefix}", self.binary);
        }
        self.resume_command_with_flags(sid, flags)
    }

    /// Build the interactive CLI command to spawn an agent conversation for `task` (or explicit
    /// one-shot `/bin/sh -c` when `self.harness == AgentHarness::Script`).
    ///
    /// `pi` always passes `--no-extensions --no-mcp` plus any explicit `-e <path>` entries.
    /// `pi` and `claude` receive persona/contract text via `--append-system-prompt`.
    /// `codex` receives persona/contract text via `-c developer_instructions=...` and runs in
    /// interactive mode (`--sandbox workspace-write -c sandbox_workspace_write.network_access=true`).
    /// `agy` runs in interactive mode via `--prompt-interactive`.
    pub fn spawn_command(
        &self,
        task: &str,
        model: Option<&str>,
        append_system_prompt: Option<&str>,
    ) -> String {
        self.spawn_command_with_flags_and_extensions(task, model, append_system_prompt, &[], &[])
    }

    #[allow(dead_code)]
    pub fn spawn_command_with_extensions(
        &self,
        task: &str,
        model: Option<&str>,
        append_system_prompt: Option<&str>,
        extensions: &[String],
    ) -> String {
        self.spawn_command_with_flags_and_extensions(
            task,
            model,
            append_system_prompt,
            &[],
            extensions,
        )
    }

    pub fn spawn_command_with_flags_and_extensions(
        &self,
        task: &str,
        model: Option<&str>,
        append_system_prompt: Option<&str>,
        flags: &[String],
        extensions: &[String],
    ) -> String {
        self.start_command_with_flags_and_extensions(
            non_empty_string(task),
            model,
            append_system_prompt,
            flags,
            extensions,
        )
    }

    /// Summarize the composed CLI flags for logging without including persona or task prose.
    #[cfg(test)]
    pub fn spawn_flags_summary(
        &self,
        model: Option<&str>,
        has_append_system_prompt: bool,
        extensions: &[String],
    ) -> String {
        self.start_flags_summary_with_custom(
            model,
            has_append_system_prompt,
            false,
            extensions,
            &[],
        )
    }

    /// Build the interactive CLI command to start a multi-turn conversation in a pane.
    ///
    /// Never passes non-interactive print/exec flags (`-p`, `--print`, `exec`, `--prompt`)
    /// unless `self.harness == AgentHarness::Script`.
    #[allow(dead_code)]
    pub fn start_command(
        &self,
        message: Option<&str>,
        model: Option<&str>,
        append_system_prompt: Option<&str>,
    ) -> String {
        self.start_command_with_flags_and_extensions(message, model, append_system_prompt, &[], &[])
    }

    #[allow(dead_code)]
    pub fn start_command_with_extensions(
        &self,
        message: Option<&str>,
        model: Option<&str>,
        append_system_prompt: Option<&str>,
        extensions: &[String],
    ) -> String {
        self.start_command_with_flags_and_extensions(
            message,
            model,
            append_system_prompt,
            &[],
            extensions,
        )
    }

    pub fn start_command_with_flags_and_extensions(
        &self,
        message: Option<&str>,
        model: Option<&str>,
        append_system_prompt: Option<&str>,
        flags: &[String],
        extensions: &[String],
    ) -> String {
        if self.harness == AgentHarness::Script {
            let cmd = message.and_then(non_empty_string).unwrap_or("true");
            let mut parts = vec![self.binary.to_string()];
            for f in flags {
                parts.push(shell_quote_arg(f));
            }
            parts.push("-c".to_string());
            parts.push(shell_quote_arg(cmd));
            return parts.join(" ");
        }
        let mut parts = vec![self.binary.to_string()];
        if self.harness == AgentHarness::Pi {
            parts.push("--no-extensions".to_string());
            parts.push("--no-mcp".to_string());
            parts.push("--models".to_string());
            parts.push("'*'".to_string());
        }
        for f in flags {
            parts.push(shell_quote_arg(f));
        }
        if self.harness == AgentHarness::Pi {
            for ext in extensions {
                if let Some(e) = non_empty_string(ext) {
                    parts.push("-e".to_string());
                    parts.push(shell_quote_arg(e));
                }
            }
        }
        if let Some(m) = model.and_then(non_empty_string) {
            parts.push("--model".to_string());
            parts.push(shell_quote_arg(m));
        }
        if self.harness == AgentHarness::Codex {
            parts.extend(
                [
                    "--sandbox",
                    "workspace-write",
                    "-c",
                    "sandbox_workspace_write.network_access=true",
                ]
                .map(str::to_string),
            );
        }
        if let Some(persona) = append_system_prompt.and_then(non_empty_string) {
            match self.harness {
                AgentHarness::Pi | AgentHarness::Claude => {
                    parts.push("--append-system-prompt".to_string());
                    parts.push(shell_quote_arg(persona));
                }
                AgentHarness::Codex => {
                    parts.push("-c".to_string());
                    parts.push(shell_quote_arg(&format!(
                        "developer_instructions={persona}"
                    )));
                }
                AgentHarness::Agy | AgentHarness::Script => {}
            }
        }
        if let Some(msg) = message.and_then(non_empty_string) {
            if self.harness == AgentHarness::Agy {
                parts.push("--prompt-interactive".to_string());
            }
            parts.push(shell_quote_arg(msg));
        }
        parts.join(" ")
    }

    /// Summarize the composed interactive CLI flags for logging without persona or message prose.
    #[allow(dead_code)]
    pub fn start_flags_summary(
        &self,
        model: Option<&str>,
        has_append_system_prompt: bool,
        has_initial_message: bool,
        extensions: &[String],
    ) -> String {
        self.start_flags_summary_with_custom(
            model,
            has_append_system_prompt,
            has_initial_message,
            extensions,
            &[],
        )
    }

    pub fn start_flags_summary_with_custom(
        &self,
        model: Option<&str>,
        has_append_system_prompt: bool,
        has_initial_message: bool,
        extensions: &[String],
        flags: &[String],
    ) -> String {
        if self.harness == AgentHarness::Script {
            let redacted_flags = redact_harness_flags(flags);
            if redacted_flags.is_empty() {
                return "-c <SCRIPT>".to_string();
            } else {
                return format!("{} -c <SCRIPT>", redacted_flags.join(" "));
            }
        }
        let mut parts = Vec::new();
        if self.harness == AgentHarness::Pi {
            parts.push("--no-extensions".to_string());
            parts.push("--no-mcp".to_string());
            parts.push("--models".to_string());
            parts.push("'*'".to_string());
        }
        let redacted_flags = redact_harness_flags(flags);
        for f in redacted_flags {
            parts.push(shell_quote_arg(&f));
        }
        if self.harness == AgentHarness::Pi {
            for ext in extensions {
                if let Some(e) = non_empty_string(ext) {
                    parts.push("-e".to_string());
                    parts.push(shell_quote_arg(e));
                }
            }
        }
        if let Some(m) = model.and_then(non_empty_string) {
            parts.push("--model".to_string());
            parts.push(shell_quote_arg(m));
        }
        if self.harness == AgentHarness::Codex {
            parts.extend(
                [
                    "--sandbox",
                    "workspace-write",
                    "-c",
                    "sandbox_workspace_write.network_access=true",
                ]
                .map(str::to_string),
            );
        }
        if has_append_system_prompt {
            match self.harness {
                AgentHarness::Pi | AgentHarness::Claude => {
                    parts.push("--append-system-prompt".to_string());
                    parts.push("<AGENT.md>".to_string());
                }
                AgentHarness::Codex => {
                    parts.push("-c".to_string());
                    parts.push("developer_instructions=<AGENT.md>".to_string());
                }
                AgentHarness::Agy | AgentHarness::Script => {}
            }
        }
        if has_initial_message {
            if self.harness == AgentHarness::Agy {
                parts.push("--prompt-interactive".to_string());
            }
            parts.push("<MESSAGE>".to_string());
        }
        parts.join(" ")
    }

    /// Extract a session ID from a hook JSON payload or structured telemetry value.
    pub fn capture_session_id(&self, payload: &serde_json::Value) -> Option<String> {
        let candidates = match self.harness {
            AgentHarness::Claude => &["session_id", "sessionId"][..],
            AgentHarness::Codex => &["session_id", "thread_id", "conversation_id"][..],
            AgentHarness::Pi => &["session_id", "sessionId", "sessionFile"][..],
            AgentHarness::Agy => &["session_id", "conversation_id", "conversationId"][..],
            AgentHarness::Script => &["session_id"][..],
        };
        for key in candidates {
            if let Some(val) = payload
                .get(*key)
                .and_then(|v| v.as_str())
                .and_then(non_empty_string)
            {
                return Some(val.to_string());
            }
        }
        None
    }
}

/// Resolve the active harness adapter for an agent's `[harness]` config, preferring `cli`
/// when its binary is on `PATH` (or when no `fallback` is set) and falling back to `fallback`
/// when the primary binary is absent.
pub fn resolve_harness_adapter(
    harness_cfg: &crate::agent::AgentHarnessConfig,
) -> Result<&'static HarnessAdapter, String> {
    let primary = harness_adapter(&harness_cfg.cli).ok_or_else(|| {
        format!(
            "unknown harness cli '{}' (expected pi, claude, codex, agy, or script)",
            harness_cfg.cli
        )
    })?;
    if primary.harness == AgentHarness::Script || super::binary_in_path(primary.binary) {
        return Ok(primary);
    }
    if let Some(ref fb_name) = harness_cfg.fallback {
        let fallback = harness_adapter(fb_name).ok_or_else(|| {
            format!(
                "unknown fallback '{}' (expected pi, claude, codex, agy, or script)",
                fb_name
            )
        })?;
        if fallback.harness == AgentHarness::Script || super::binary_in_path(fallback.binary) {
            return Ok(fallback);
        }
    }
    Ok(primary)
}

/// Build the `# Plexi worker contract` section included in every spawned agent session.
pub fn format_worker_contract(definition: &crate::agent::AgentDefinition) -> String {
    let mut contract = format!(
        "# Plexi worker contract\n\
         You are agent `{}` running in an interactive Plexi agent session.\n\
         Always report your final or structured task completion by running: plexi agent done --summary \"<one line>\" --result '<json>'.\n\
         If the task does not fit the result schema, run plexi agent done --summary \"<why>\" --result '{{}}' or plexi agent ask \"<question>\".",
        definition.id
    );
    contract.push_str("\nAfter asking, wait for the parent's reply with: plexi pane message recv --timeout 300. Retry on timeout; continue the task after reading the reply.");
    if let Some(schema) = definition
        .harness
        .as_ref()
        .and_then(|h| h.result_schema.as_ref())
    {
        if let Ok(schema_json) = serde_json::to_string(schema) {
            contract.push_str(&format!("\nResult schema: {schema_json}"));
        }
    }
    contract
}

/// Unified pure function building the exact interactive shell command `plexi agent spawn`,
/// `plexi agent start`, `plexi agent call`/`handoff`, `plexi agent team run`, and GUI/palette
/// spawns run for `(definition, adapter, task)`.
///
/// - Refuses agents without a `[harness]` table.
/// - Never passes non-interactive print/exec flags (`-p`, `--print`, `exec`, `--prompt`)
///   unless `adapter.harness == AgentHarness::Script`.
/// - Applies `AGENT.md` persona and `# Plexi worker contract` (including `result_schema`).
pub fn build_agent_spawn_command(
    definition: &crate::agent::AgentDefinition,
    adapter: &HarnessAdapter,
    task: &str,
) -> Result<String, String> {
    build_agent_start_command(definition, adapter, non_empty_string(task))
}

/// Pure function building the exact interactive shell command `plexi agent start` and
/// `plexi agent spawn` run for `(definition, adapter, message)`.
///
/// - Refuses agents without a `[harness]` table.
/// - Never passes non-interactive print/exec flags (`-p`, `--print`, `exec`, `--prompt`)
///   unless `adapter.harness == AgentHarness::Script`.
/// - Applies `AGENT.md` persona and `# Plexi worker contract` even when `message` is `None`.
pub fn build_agent_start_command(
    definition: &crate::agent::AgentDefinition,
    adapter: &HarnessAdapter,
    message: Option<&str>,
) -> Result<String, String> {
    let harness_cfg = definition.harness.as_ref().ok_or_else(|| {
        format!(
            "agent '{}' has no [harness] table in settings.toml (it is an Assistant persona and cannot be started as a CLI harness session)",
            definition.id
        )
    })?;
    if adapter.harness == AgentHarness::Script {
        return Ok(adapter.start_command_with_flags_and_extensions(
            message,
            None,
            None,
            &harness_cfg.flags,
            &[],
        ));
    }
    let persona = definition.prompt.trim();
    let contract = format_worker_contract(definition);
    let system_instructions = if persona.is_empty() {
        contract.clone()
    } else {
        format!("{persona}\n\n{contract}")
    };
    let initial_message = message.and_then(non_empty_string);
    let (append_prompt, effective_message) = match adapter.harness {
        AgentHarness::Pi | AgentHarness::Claude => (
            Some(system_instructions),
            initial_message.map(str::to_string),
        ),
        AgentHarness::Codex => {
            let dev_instructions = if persona.is_empty() {
                contract
            } else {
                format!("# Persona\n{persona}\n\n{contract}")
            };
            (
                Some(dev_instructions),
                initial_message.map(str::to_string),
            )
        }
        AgentHarness::Agy => {
            let full_prompt = match (persona.is_empty(), initial_message) {
                (false, Some(msg)) => format!("# Persona\n{persona}\n\n{msg}\n\n{contract}"),
                (false, None) => format!("# Persona\n{persona}\n\n{contract}"),
                (true, Some(msg)) => format!("{msg}\n\n{contract}"),
                (true, None) => contract,
            };
            (None, Some(full_prompt))
        }
        AgentHarness::Script => (None, initial_message.map(str::to_string)),
    };
    let flags_summary = adapter.start_flags_summary_with_custom(
        harness_cfg.model.as_deref(),
        append_prompt.as_deref().and_then(non_empty_string).is_some(),
        effective_message.is_some(),
        &harness_cfg.extensions,
        &harness_cfg.flags,
    );
    log::info!(
        "agent_spawn: role={} harness={} flags=\"{}\"",
        definition.id,
        adapter.harness.as_str(),
        flags_summary
    );
    if !harness_cfg.flags.is_empty() {
        log::info!(
            "agent_flags: role={} harness={} safe_flags={:?}",
            definition.id,
            adapter.harness.as_str(),
            redact_harness_flags(&harness_cfg.flags)
        );
    }
    Ok(adapter.start_command_with_flags_and_extensions(
        effective_message.as_deref(),
        harness_cfg.model.as_deref(),
        append_prompt.as_deref(),
        &harness_cfg.flags,
        &harness_cfg.extensions,
    ))
}

/// Resolve the already-validated custom flags for a persisted agent session.
///
/// A saved session keeps its configured role and harness, while the definition
/// remains the source of truth for its argv array. If the definition disappeared
/// or now resolves to a different adapter, resume safely falls back to no custom
/// flags instead of applying flags for another harness.
pub fn resolved_session_harness_flags_in(
    role: Option<&str>,
    harness: AgentHarness,
    profile_dir: &Path,
    cwd: Option<&Path>,
) -> Vec<String> {
    let Some(role) = role.and_then(non_empty_string) else {
        return Vec::new();
    };
    let workspace_root = cwd.and_then(crate::app::registry::resolve_agent_lookup_workspace_root);
    let Ok((definition, _)) = resolve_agent_definition_in(role, profile_dir, workspace_root.as_deref())
    else {
        return Vec::new();
    };
    let Some(config) = definition.harness else {
        return Vec::new();
    };
    let configured_for_harness = harness_adapter(&config.cli)
        .is_some_and(|adapter| adapter.harness == harness)
        || config
            .fallback
            .as_deref()
            .and_then(harness_adapter)
            .is_some_and(|adapter| adapter.harness == harness);
    if configured_for_harness {
        config.flags
    } else {
        Vec::new()
    }
}

fn tier_display_label(source: crate::agent::AgentSource) -> &'static str {
    match source {
        crate::agent::AgentSource::Workspace => "workspace",
        crate::agent::AgentSource::User => "global",
        crate::agent::AgentSource::BuiltIn => "built-in",
    }
}

/// Validate that `<name>` is a safe lowercase agent slug (`a-z`, `0-9`, `-`, starting with `a-z`).
pub fn validate_agent_slug(name: &str) -> Result<(), String> {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed != name {
        return Err(format!(
            "invalid agent name '{name}': must be a non-empty lowercase slug (a-z, 0-9, '-')"
        ));
    }
    if trimmed == "default" {
        return Err(
            "invalid agent name 'default': 'default' is the built-in agent — edit its seeded definition under <workspace>/.plexi/agents/default/ instead of creating it"
                .to_string(),
        );
    }
    let mut chars = trimmed.chars();
    let Some(first) = chars.next() else {
        return Err("agent name cannot be empty".to_string());
    };
    if !first.is_ascii_lowercase() {
        return Err(format!(
            "invalid agent name '{name}': must start with a lowercase ASCII letter (a-z)"
        ));
    }
    if trimmed.len() > 64
        || !trimmed
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        || trimmed.ends_with('-')
    {
        return Err(format!(
            "invalid agent name '{name}': must contain only lowercase ASCII letters, digits, and hyphens (a-z, 0-9, '-')"
        ));
    }
    Ok(())
}

/// Resolve an agent by `<name>` through `AgentRegistry` (workspace tier shadowing global tier).
/// If an agent directory for `<name>` exists on disk with a broken `AGENT.md` or `settings.toml`,
/// returns that parse error rather than silently ignoring it.
pub fn resolve_agent_definition_in(
    name: &str,
    profile_dir: &Path,
    workspace_root: Option<&Path>,
) -> Result<(crate::agent::AgentDefinition, bool), String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("agent name cannot be empty".to_string());
    }
    // First use of the built-in `default` seeds the workspace copy the user owns
    // (stint 1035). An existing `default` directory is never touched, so a user's
    // own definition always wins.
    if trimmed == "default" {
        if let Some(ws) = workspace_root {
            if let Err(error) = crate::agent::ensure_workspace_default_agent(ws) {
                log::error!(
                    "agent default: could not seed {}: {error}",
                    crate::agent::workspace_agents_dir(ws).join("default").display()
                );
            }
        }
    }
    let registry =
        crate::agent::AgentRegistry::load_with_optional_workspace(profile_dir, workspace_root);

    // If a workspace agent directory for `trimmed` exists on disk, ensure any parse error in it
    // is surfaced rather than silently falling through to a global definition or "not found".
    if let Some(ws) = workspace_root {
        let ws_dir = crate::agent::workspace_agents_dir(ws).join(trimmed);
        if ws_dir.is_dir()
            && !registry
                .active(trimmed)
                .is_some_and(|d| d.source == crate::agent::AgentSource::Workspace)
        {
            let def = crate::agent::AgentDefinition::load_dir(
                &ws_dir,
                crate::agent::AgentSource::Workspace,
            )?;
            let shadows_global = registry
                .active(trimmed)
                .is_some_and(|d| d.source == crate::agent::AgentSource::User);
            return Ok((def, shadows_global));
        }
    }

    let global_dir = profile_dir.join("agents").join(trimmed);
    if global_dir.is_dir()
        && !registry
            .active(trimmed)
            .is_some_and(|d| d.source != crate::agent::AgentSource::BuiltIn)
    {
        let def =
            crate::agent::AgentDefinition::load_dir(&global_dir, crate::agent::AgentSource::User)?;
        return Ok((def, false));
    }

    if let Some(def) = registry.active(trimmed) {
        let shadows_global = registry
            .shadowed(trimmed)
            .iter()
            .any(|s| s.source == crate::agent::AgentSource::User);
        return Ok((def.clone(), shadows_global));
    }

    let mut searched = Vec::new();
    if let Some(ws) = workspace_root {
        searched.push(
            crate::agent::workspace_agents_dir(ws)
                .join(trimmed)
                .display()
                .to_string(),
        );
    }
    searched.push(global_dir.display().to_string());
    let available: Vec<&str> = registry
        .agents()
        .filter(|d| d.source != crate::agent::AgentSource::BuiltIn || d.harness.is_some())
        .map(|d| d.id.as_str())
        .collect();
    let available_str = if available.is_empty() {
        "none".to_string()
    } else {
        available.join(", ")
    };
    Err(format!(
        "agent '{trimmed}' not found (searched: {}; available agents: {available_str})",
        searched.join(", ")
    ))
}

pub struct AgentCreateOptions<'a> {
    pub name: &'a str,
    pub cli: &'a str,
    pub model: Option<&'a str>,
    pub fallback: Option<&'a str>,
    pub description: Option<&'a str>,
    pub persona: Option<&'a str>,
    pub persona_file: Option<&'a str>,
    pub result_schema: Option<&'a str>,
    pub ephemeral: bool,
    pub display_name: Option<&'a str>,
    pub global: bool,
    pub cwd: Option<&'a Path>,
    pub flags: &'a [String],
}

pub fn agent_create_in(
    opts: &AgentCreateOptions<'_>,
    profile_dir: &Path,
) -> Result<PathBuf, String> {
    validate_agent_slug(opts.name)?;
    let cli_trimmed = opts.cli.trim();
    if !matches!(cli_trimmed, "pi" | "claude" | "codex" | "agy" | "script") {
        return Err(format!(
            "invalid --cli '{}': expected one of pi, claude, codex, agy, script",
            opts.cli
        ));
    }
    if let Some(fb) = opts.fallback.and_then(non_empty_string) {
        if !matches!(fb, "pi" | "claude" | "codex" | "agy") {
            return Err(format!(
                "invalid --fallback '{fb}': expected one of pi, claude, codex, agy (script is only allowed as --cli)"
            ));
        }
    }
    if !opts.flags.is_empty() {
        let primary = harness_adapter(cli_trimmed).ok_or_else(|| {
            format!("invalid --cli '{}': unknown harness", opts.cli)
        })?;
        validate_custom_harness_flags(primary, opts.flags, opts.model)?;
        if let Some(fb_str) = opts.fallback.and_then(non_empty_string) {
            if let Some(fb_adapter) = harness_adapter(fb_str) {
                validate_custom_harness_flags(fb_adapter, opts.flags, opts.model)?;
            }
        }
    }

    let effective_cwd = match opts.cwd {
        Some(d) => d.to_path_buf(),
        None => std::env::current_dir()
            .map_err(|e| format!("could not determine current directory: {e}"))?,
    };

    let persona_text = match (opts.persona, opts.persona_file) {
        (Some(_), Some(_)) => {
            return Err(
                "--persona and --persona-file are mutually exclusive; pass only one".to_string(),
            )
        }
        (None, None) => {
            return Err("one of --persona <text> or --persona-file <path> is required".to_string())
        }
        (Some(inline), None) => {
            if inline.trim().is_empty() {
                return Err("--persona must be non-empty".to_string());
            }
            inline.to_string()
        }
        (None, Some(file_str)) => {
            let file_path = {
                let p = Path::new(file_str.trim());
                if p.is_absolute() {
                    p.to_path_buf()
                } else {
                    effective_cwd.join(p)
                }
            };
            let content = std::fs::read_to_string(&file_path).map_err(|e| {
                format!("failed to read --persona-file {}: {e}", file_path.display())
            })?;
            if content.trim().is_empty() {
                return Err(format!("--persona-file {} is empty", file_path.display()));
            }
            content
        }
    };

    let normalized_schema_json = match opts.result_schema.and_then(non_empty_string) {
        None => None,
        Some(spec) => {
            let value = crate::host::pane_result::parse_schema_spec(spec, Some(&effective_cwd))
                .map_err(|e| format!("invalid --result-schema: {e}"))?;
            let compact =
                serde_json::to_string(&value).map_err(|e| format!("serialize schema: {e}"))?;
            Some(compact)
        }
    };

    let (target_dir, source) = if opts.global {
        (
            profile_dir.join("agents").join(opts.name),
            crate::agent::AgentSource::User,
        )
    } else {
        let Some(ws_root) =
            crate::app::registry::resolve_agent_create_workspace_root(&effective_cwd)
        else {
            return Err(format!(
                "no project workspace (.plexi/ or .git) found at or above {}; run inside a project or pass --global",
                effective_cwd.display()
            ));
        };
        crate::agent::migrate_legacy_workspace_agents(&ws_root);
        (
            crate::agent::workspace_agents_dir(&ws_root).join(opts.name),
            crate::agent::AgentSource::Workspace,
        )
    };

    if target_dir.exists() {
        return Err(format!(
            "agent '{}' already exists at {}; edit {}/AGENT.md and {}/settings.toml directly",
            opts.name,
            target_dir.display(),
            target_dir.display(),
            target_dir.display()
        ));
    }

    let display_name = opts
        .display_name
        .and_then(non_empty_string)
        .unwrap_or(opts.name);
    let description = opts.description.and_then(non_empty_string);

    let mut settings_lines = vec![
        "[agent]".to_string(),
        format!("id = {}", toml::Value::String(opts.name.to_string())),
        format!(
            "display_name = {}",
            toml::Value::String(display_name.to_string())
        ),
    ];
    if let Some(desc) = description {
        settings_lines.push(format!(
            "description = {}",
            toml::Value::String(desc.to_string())
        ));
    }
    settings_lines.push(String::new());
    settings_lines.push("[harness]".to_string());
    settings_lines.push(format!(
        "cli = {}",
        toml::Value::String(cli_trimmed.to_string())
    ));
    if let Some(m) = opts.model.and_then(non_empty_string) {
        settings_lines.push(format!("model = {}", toml::Value::String(m.to_string())));
    }
    if let Some(fb) = opts.fallback.and_then(non_empty_string) {
        settings_lines.push(format!(
            "fallback = {}",
            toml::Value::String(fb.to_string())
        ));
    }
    if opts.ephemeral {
        settings_lines.push("ephemeral = true".to_string());
    }
    if !opts.flags.is_empty() {
        let flag_items: Vec<String> = opts
            .flags
            .iter()
            .map(|f| format!("\"{}\"", f.replace('\\', "\\\\").replace('"', "\\\"")))
            .collect();
        settings_lines.push(format!("flags = [{}]", flag_items.join(", ")));
    }
    if let Some(schema_str) = normalized_schema_json {
        settings_lines.push(format!(
            "result_schema = {}",
            toml::Value::String(schema_str)
        ));
    }
    let settings_content = format!("{}\n", settings_lines.join("\n"));
    let prompt_content = if persona_text.ends_with('\n') {
        persona_text
    } else {
        format!("{persona_text}\n")
    };

    std::fs::create_dir_all(&target_dir)
        .map_err(|e| format!("failed to create {}: {e}", target_dir.display()))?;
    if let Err(e) = std::fs::write(target_dir.join("AGENT.md"), &prompt_content) {
        let _ = std::fs::remove_dir(&target_dir);
        return Err(format!(
            "failed to write {}: {e}",
            target_dir.join("AGENT.md").display()
        ));
    }
    if let Err(e) = std::fs::write(target_dir.join("settings.toml"), &settings_content) {
        let _ = std::fs::remove_file(target_dir.join("AGENT.md"));
        let _ = std::fs::remove_dir(&target_dir);
        return Err(format!(
            "failed to write {}: {e}",
            target_dir.join("settings.toml").display()
        ));
    }

    if let Err(e) = crate::agent::AgentDefinition::load_dir(&target_dir, source) {
        let _ = std::fs::remove_file(target_dir.join("AGENT.md"));
        let _ = std::fs::remove_file(target_dir.join("settings.toml"));
        let _ = std::fs::remove_dir(&target_dir);
        return Err(e);
    }

    Ok(target_dir)
}

#[allow(clippy::too_many_arguments)]
pub fn agent_create_cli(
    name: &str,
    cli: &str,
    model: Option<&str>,
    fallback: Option<&str>,
    description: Option<&str>,
    persona: Option<&str>,
    persona_file: Option<&str>,
    result_schema: Option<&str>,
    ephemeral: bool,
    display_name: Option<&str>,
    global: bool,
    cwd: Option<&str>,
    flags: &[String],
) -> i32 {
    log::info!("agent_create:cli: name={name} cli={cli} global={global} ephemeral={ephemeral}");
    let cwd_path = cwd.map(PathBuf::from);
    let opts = AgentCreateOptions {
        name,
        cli,
        model,
        fallback,
        description,
        persona,
        persona_file,
        result_schema,
        ephemeral,
        display_name,
        global,
        cwd: cwd_path.as_deref(),
        flags,
    };
    match agent_create_in(&opts, &crate::config::config_dir()) {
        Ok(created_dir) => {
            log::info!("agent_create:cli: created {}", created_dir.display());
            println!("{}", created_dir.display());
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

pub fn agent_list_in(
    profile_dir: &Path,
    workspace_root: Option<&Path>,
    json: bool,
) -> Result<String, String> {
    let registry =
        crate::agent::AgentRegistry::load_with_optional_workspace(profile_dir, workspace_root);
    // Directories neither tier will present as an agent, named with the file
    // that is missing or legacy, so a real agent is never silently invisible.
    let mut unrecognised = crate::agent::scan_agent_dirs(&profile_dir.join("agents"));
    if let Some(root) = workspace_root {
        unrecognised.extend(crate::agent::scan_agent_dirs(&crate::agent::workspace_agents_dir(
            root,
        )));
    }
    // The built-in `default` is listed too: it is a launchable Pi agent, and
    // `spawn`/`start default` resolve exactly this row until the workspace has
    // its own seeded copy (stint 1035).
    let mut rows = Vec::new();
    for def in registry.agents() {
        let tier = tier_display_label(def.source);
        let shadows_global = registry
            .shadowed(&def.id)
            .iter()
            .any(|s| s.source == crate::agent::AgentSource::User);
        let (harness_label, cli_val, model_val) = match &def.harness {
            Some(h) => {
                let label = match &h.model {
                    Some(m) => format!("{} ({m})", h.cli),
                    None => h.cli.clone(),
                };
                (
                    label,
                    serde_json::Value::String(h.cli.clone()),
                    h.model
                        .as_ref()
                        .map(|m| serde_json::Value::String(m.clone()))
                        .unwrap_or(serde_json::Value::Null),
                )
            }
            None => (
                "assistant".to_string(),
                serde_json::Value::Null,
                serde_json::Value::Null,
            ),
        };
        rows.push((
            def,
            tier,
            shadows_global,
            harness_label,
            serde_json::json!({
                "name": def.id,
                "display_name": def.display_name,
                "tier": tier,
                "harness": if def.harness.is_some() { cli_val.clone() } else { serde_json::Value::String("assistant".to_string()) },
                "cli": cli_val,
                "model": model_val,
                "description": def.description,
                "shadows_global": shadows_global,
                "path": def.path,
            }),
        ));
    }

    if json {
        let json_rows: Vec<serde_json::Value> = rows.into_iter().map(|(_, _, _, _, j)| j).collect();
        return serde_json::to_string_pretty(&json_rows).map_err(|e| e.to_string());
    }

    let mut out = if rows.is_empty() {
        "No agents found in workspace or global registry.".to_string()
    } else {
        let mut lines = Vec::with_capacity(rows.len());
        for (def, tier, shadows_global, harness_label, _) in rows {
            let mut line = format!("{:<18} {:<10} {:<32}", def.id, tier, harness_label);
            if !def.description.is_empty() {
                line.push_str(&format!(" {}", def.description));
            }
            if shadows_global {
                line.push_str(" [shadows global]");
            }
            lines.push(line.trim_end().to_string());
        }
        lines.join("\n")
    };
    if !unrecognised.is_empty() {
        out.push_str("\n\nunrecognised agent directories (not loaded):");
        for item in &unrecognised {
            out.push_str(&format!(
                "\n  {}  {}",
                item.path.display(),
                item.issue.describe()
            ));
        }
    }
    Ok(out)
}

pub fn agent_list_cli(cwd: Option<&str>, json: bool) -> i32 {
    log::info!("agent_list:cli: cwd={cwd:?} json={json}");
    let effective_cwd = cwd
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok());
    let workspace_root = effective_cwd
        .as_deref()
        .and_then(crate::app::registry::resolve_agent_lookup_workspace_root);
    match agent_list_in(
        &crate::config::config_dir(),
        workspace_root.as_deref(),
        json,
    ) {
        Ok(out) => {
            println!("{out}");
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

pub fn agent_show_in(
    name: &str,
    profile_dir: &Path,
    workspace_root: Option<&Path>,
    json: bool,
) -> Result<String, String> {
    let (def, shadows_global) = resolve_agent_definition_in(name, profile_dir, workspace_root)?;
    let tier = tier_display_label(def.source);
    let safe_flags = def
        .harness
        .as_ref()
        .map(|h| redact_harness_flags(&h.flags))
        .unwrap_or_default();
    let spawn_command = match &def.harness {
        Some(h) => {
            let adapter = resolve_harness_adapter(h)?;
            let mut display_def = def.clone();
            if let Some(display_harness) = display_def.harness.as_mut() {
                display_harness.flags = redact_harness_flags(&display_harness.flags);
            }
            Some(build_agent_spawn_command(&display_def, adapter, "<TASK>")?)
        }
        None => None,
    };

    if json {
        let harness_json = def.harness.as_ref().map(|h| {
            let redacted = redact_harness_flags(&h.flags);
            serde_json::json!({
                "cli": h.cli,
                "model": h.model,
                "fallback": h.fallback,
                "ephemeral": h.ephemeral,
                "result_schema": h.result_schema,
                "flags": redacted.clone(),
                "resolved_flags": redacted,
            })
        });
        let value = serde_json::json!({
            "name": def.id,
            "display_name": def.display_name,
            "tier": tier,
            "shadows_global": shadows_global,
            "path": def.path,
            "description": def.description,
            "persona": def.prompt,
            "can_call": def.can_call,
            "can_handoff": def.can_handoff,
            "harness": harness_json,
            "spawn_command": spawn_command,
        });
        return serde_json::to_string_pretty(&value).map_err(|e| e.to_string());
    }

    let path_str = def
        .path
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "(built-in)".to_string());
    let tier_str = if shadows_global {
        format!("{tier} (shadows global)")
    } else {
        tier.to_string()
    };
    let mut lines = vec![
        format!("Name:          {}", def.id),
        format!("Display name:  {}", def.display_name),
        format!("Tier:          {tier_str}"),
        format!("Path:          {path_str}"),
    ];
    if !def.description.is_empty() {
        lines.push(format!("Description:   {}", def.description));
    }
    match &def.harness {
        Some(h) => {
            lines.push(format!("CLI:           {}", h.cli));
            if let Some(m) = &h.model {
                lines.push(format!("Model:         {m}"));
            }
            if let Some(fb) = &h.fallback {
                lines.push(format!("Fallback:      {fb}"));
            }
            lines.push(format!("Ephemeral:     {}", h.ephemeral));
            if !safe_flags.is_empty() {
                lines.push(format!("Flags:         {}", safe_flags.join(" ")));
            }
            if let Some(schema) = &h.result_schema {
                if let Ok(s) = serde_json::to_string(schema) {
                    lines.push(format!("Result schema: {s}"));
                }
            }
        }
        None => {
            lines.push("Harness:       assistant (no [harness] table)".to_string());
        }
    }
    if let Some(cmd) = spawn_command {
        lines.push(format!("Spawn command: {cmd}"));
    }
    Ok(lines.join("\n"))
}

pub fn agent_show_cli(name: &str, cwd: Option<&str>, json: bool) -> i32 {
    log::info!("agent_show:cli: name={name} cwd={cwd:?} json={json}");
    let effective_cwd = cwd
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok());
    let workspace_root = effective_cwd.as_deref().and_then(|cwd| {
        resolve_agent_workspace_root_for(
            name,
            cwd,
            &crate::app::registry::workspace_walk_stops(),
        )
    });
    match agent_show_in(
        name,
        &crate::config::config_dir(),
        workspace_root.as_deref(),
        json,
    ) {
        Ok(out) => {
            println!("{out}");
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

pub fn agent_flags_cli(target: Option<&str>, cwd: Option<&str>, json: bool) -> i32 {
    log::info!("agent_flags:cli target={:?} cwd={:?} json={}", target, cwd, json);
    if let Some(t) = target {
        if let Some(adapter) = harness_adapter(t) {
            return display_adapter_flags(adapter, json);
        }
    }
    if target.is_none() {
        return display_all_adapters_flags(json);
    }
    let agent_name = target.unwrap();
    let effective_cwd = cwd
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok());
    let workspace_root = effective_cwd
        .as_deref()
        .and_then(crate::app::registry::resolve_agent_lookup_workspace_root);
    let (def, _) = match resolve_agent_definition_in(
        agent_name,
        &crate::config::config_dir(),
        workspace_root.as_deref(),
    ) {
        Ok(res) => res,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    display_agent_flags(&def, json)
}

fn display_adapter_flags(adapter: &HarnessAdapter, json: bool) -> i32 {
    if json {
        let options: Vec<serde_json::Value> = adapter
            .supported_options
            .iter()
            .map(|opt| {
                serde_json::json!({
                    "name": opt.name,
                    "kind": match opt.kind {
                        OptionArgKind::Flag => "flag",
                        OptionArgKind::Value => "value",
                    },
                    "description": opt.description,
                })
            })
            .collect();
        let payload = serde_json::json!({
            "harness": adapter.harness.as_str(),
            "binary": adapter.binary,
            "supported_options": options,
            "reserved_options": adapter.reserved_options,
            "security_overrides": adapter.security_overrides,
        });
        println!("{}", serde_json::to_string_pretty(&payload).unwrap());
        return 0;
    }

    println!("Harness: {}", adapter.harness.as_str());
    println!("Binary:  {}", adapter.binary);
    println!("\nSupported options:");
    for opt in adapter.supported_options {
        let kind_str = match opt.kind {
            OptionArgKind::Flag => "",
            OptionArgKind::Value => " <VALUE>",
        };
        let flag_display = format!("{}{}", opt.name, kind_str);
        println!("  {:<30} {}", flag_display, opt.description);
    }
    if !adapter.reserved_options.is_empty() {
        println!("\nReserved lifecycle options (host-managed):");
        println!("  {}", adapter.reserved_options.join(", "));
    }
    if !adapter.security_overrides.is_empty() {
        println!("\nProhibited security overrides:");
        println!("  {}", adapter.security_overrides.join(", "));
    }
    0
}

fn display_all_adapters_flags(json: bool) -> i32 {
    if json {
        let list: Vec<serde_json::Value> = HARNESS_ADAPTERS
            .iter()
            .map(|a| {
                let options: Vec<serde_json::Value> = a
                    .supported_options
                    .iter()
                    .map(|opt| {
                        serde_json::json!({
                            "name": opt.name,
                            "kind": match opt.kind {
                                OptionArgKind::Flag => "flag",
                                OptionArgKind::Value => "value",
                            },
                            "description": opt.description,
                        })
                    })
                    .collect();
                serde_json::json!({
                    "harness": a.harness.as_str(),
                    "binary": a.binary,
                    "supported_options": options,
                    "reserved_options": a.reserved_options,
                    "security_overrides": a.security_overrides,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&list).unwrap());
        return 0;
    }

    println!("Plexi Harness Adapters and Custom Flags Contract\n");
    for adapter in &HARNESS_ADAPTERS {
        let opt_names: Vec<&str> = adapter.supported_options.iter().map(|o| o.name).collect();
        println!(
            "{:<10} (binary: {:<8}) -> options: {}",
            adapter.harness.as_str(),
            adapter.binary,
            opt_names.join(", ")
        );
    }
    println!("\nRun `plexi agent flags <harness>` for adapter details and safety rules.");
    println!("Run `plexi agent flags <agent>` to inspect an agent's configured flags.");
    0
}

fn display_agent_flags(def: &crate::agent::AgentDefinition, json: bool) -> i32 {
    let harness_cfg = match &def.harness {
        Some(h) => h,
        None => {
            eprintln!("agent '{}' has no [harness] table in settings.toml", def.id);
            return 1;
        }
    };
    let redacted = redact_harness_flags(&harness_cfg.flags);
    if json {
        let payload = serde_json::json!({
            "agent": def.id,
            "harness": harness_cfg.cli,
            "flags": redacted.clone(),
            "resolved_safe_flags": redacted,
            "valid": true,
        });
        println!("{}", serde_json::to_string_pretty(&payload).unwrap());
        return 0;
    }
    println!("Agent:       {}", def.id);
    println!("Harness:     {}", harness_cfg.cli);
    if redacted.is_empty() {
        println!("Safe flags:  (none)");
    } else {
        println!("Safe flags:  {}", redacted.join(" "));
    }
    println!("Status:      valid (safe)");
    0
}

/// Workspace root to resolve `name` in.
///
/// The definition-lookup walk only accepts a directory that already holds
/// `.plexi/agents/`, so a fresh checkout with a `.git` and nothing else would
/// never get its own `default`. The seed walk (`agent create`'s) accepts `.git`
/// and `.plexi` as well, which is exactly that case (stint 1035).
fn resolve_agent_workspace_root_for(
    name: &str,
    start_cwd: &Path,
    stops: &[PathBuf],
) -> Option<PathBuf> {
    let lookup =
        crate::app::registry::resolve_agent_lookup_workspace_root_stopping_at(start_cwd, stops);
    if lookup.is_some() || name.trim() != "default" {
        return lookup;
    }
    crate::app::registry::resolve_agent_create_workspace_root_stopping_at(start_cwd, stops)
}

/// Unified resolver for `plexi agent start <name>` and `plexi agent spawn <name>` from
/// `start_cwd` (walking up to the project root stopping at `stops`, then falling back to
/// `profile_dir`, then built-in) that assembles the `agent_spawn` IPC payload.
#[allow(clippy::too_many_arguments)]
pub fn resolve_agent_spawn_in_stopping_at(
    agent_name: &str,
    profile_dir: &Path,
    start_cwd: &Path,
    task_or_message: Option<&str>,
    subcontext: Option<&str>,
    ephemeral_override: Option<bool>,
    parent_pane_id: Option<u64>,
    stops: &[PathBuf],
) -> Result<serde_json::Value, String> {
    let workspace_root = resolve_agent_workspace_root_for(agent_name, start_cwd, stops);
    let (def, _) =
        resolve_agent_definition_in(agent_name, profile_dir, workspace_root.as_deref())?;
    let Some(ref harness_cfg) = def.harness else {
        return Err(format!(
            "agent '{}' has no [harness] table in settings.toml (it is an Assistant persona and cannot be started as a CLI harness session)",
            def.id
        ));
    };
    let adapter = resolve_harness_adapter(harness_cfg)?;
    let cmd_string = with_auto_state_hooks(
        adapter,
        build_agent_start_command(&def, adapter, task_or_message)?,
    );
    let project_cwd = workspace_root
        .or_else(|| {
            crate::app::registry::resolve_agent_create_workspace_root_stopping_at(start_cwd, stops)
        })
        .unwrap_or_else(|| start_cwd.to_path_buf());
    let task_text = task_or_message.and_then(non_empty_string).unwrap_or_default();
    let effective_ephemeral = match ephemeral_override {
        Some(explicit) => explicit,
        None => harness_cfg.ephemeral,
    };

    let mut payload = serde_json::json!({
        "type": "agent_spawn",
        "role": def.id,
        "harness": adapter.harness.as_str(),
        "command": cmd_string,
        "task": task_text,
        "ephemeral": effective_ephemeral,
        "cwd": project_cwd.to_string_lossy().into_owned(),
    });
    if let Some(sub) = subcontext.and_then(non_empty_string) {
        payload["subcontext"] = serde_json::Value::String(sub.to_string());
    }
    if let Some(pid) = parent_pane_id {
        payload["parent_pane_id"] = serde_json::json!(pid);
    }
    if let Some(ref schema) = harness_cfg.result_schema {
        payload["result_schema"] = schema.clone();
    }
    Ok(payload)
}

/// Resolve an interactive `plexi agent start <name>` request from `start_cwd` (walking up to the
/// project root stopping at `stops`, then falling back to `profile_dir`, then built-in) and
/// assemble the `agent_spawn` IPC payload.
pub fn resolve_agent_start_in_stopping_at(
    agent_name: &str,
    profile_dir: &Path,
    start_cwd: &Path,
    message: Option<&str>,
    parent_pane_id: Option<u64>,
    stops: &[PathBuf],
) -> Result<serde_json::Value, String> {
    resolve_agent_spawn_in_stopping_at(
        agent_name,
        profile_dir,
        start_cwd,
        message,
        None,
        Some(false),
        parent_pane_id,
        stops,
    )
}

fn resolve_effective_cwd(cwd: Option<&str>) -> Result<PathBuf, i32> {
    match cwd.and_then(non_empty_string) {
        Some(raw) => {
            let p = PathBuf::from(raw);
            if p.is_absolute() {
                Ok(p)
            } else {
                match std::env::current_dir() {
                    Ok(base) => Ok(base.join(p)),
                    Err(e) => {
                        eprintln!("error: could not determine current directory: {e}");
                        Err(1)
                    }
                }
            }
        }
        None => match std::env::current_dir() {
            Ok(d) => Ok(d),
            Err(e) => {
                eprintln!("error: could not determine current directory: {e}");
                Err(1)
            }
        },
    }
}

fn dispatch_agent_spawn_payload(
    payload: serde_json::Value,
    response_prefix: &str,
    label: &str,
    json: bool,
) -> i32 {
    let content = match super::request(payload, response_prefix, label) {
        Ok(c) => c,
        Err(code) => return code,
    };
    if let Err(code) = super::check_reply_error(&content) {
        return code;
    }
    if json {
        println!("{content}");
        return 0;
    }
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
        if let Some(pid) = v.get("pane_id").and_then(|v| v.as_u64()) {
            println!("{pid}");
            return 0;
        }
    }
    println!("{content}");
    0
}

/// `plexi agent start <name> [MESSAGE] [--message <MESSAGE>] [--cwd <DIR>]`
pub fn agent_start_cli(
    agent_name: &str,
    cwd: Option<&str>,
    message: Option<&str>,
    json: bool,
) -> i32 {
    log::info!(
        "agent_start:cli: name={agent_name} cwd={cwd:?} message_present={}",
        message.and_then(non_empty_string).is_some()
    );
    let effective_cwd = match resolve_effective_cwd(cwd) {
        Ok(d) => d,
        Err(code) => return code,
    };
    let parent_pane_id = std::env::var("PLEXI_PANE_ID")
        .ok()
        .and_then(|v| v.parse().ok());
    let stops = crate::app::registry::workspace_walk_stops();
    let payload = match resolve_agent_start_in_stopping_at(
        agent_name,
        &crate::config::config_dir(),
        &effective_cwd,
        message,
        parent_pane_id,
        &stops,
    ) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    dispatch_agent_spawn_payload(payload, "agent-start-response", "agent start", json)
}

/// `plexi agent spawn <name> --task "<brief>" [--subcontext <name>] [--ephemeral]`
#[allow(clippy::too_many_arguments)]
pub fn agent_spawn_cli(
    agent_name: &str,
    task: &str,
    subcontext: Option<&str>,
    ephemeral: bool,
    from_pane_id: Option<u64>,
    cwd: Option<&str>,
    json: bool,
) -> i32 {
    log::info!(
        "agent_spawn:cli: name={agent_name} subcontext={subcontext:?} ephemeral={ephemeral}"
    );
    let effective_cwd = match resolve_effective_cwd(cwd) {
        Ok(d) => d,
        Err(code) => return code,
    };
    let parent_pane_id = from_pane_id.or_else(|| {
        std::env::var("PLEXI_PANE_ID")
            .ok()
            .and_then(|v| v.parse().ok())
    });
    let stops = crate::app::registry::workspace_walk_stops();
    let ephemeral_override = if ephemeral { Some(true) } else { None };
    let mut payload = match resolve_agent_spawn_in_stopping_at(
        agent_name,
        &crate::config::config_dir(),
        &effective_cwd,
        Some(task),
        subcontext,
        ephemeral_override,
        parent_pane_id,
        &stops,
    ) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    if let Some(raw_cwd) = cwd.and_then(non_empty_string) {
        let _ = raw_cwd;
        payload["cwd"] = serde_json::Value::String(effective_cwd.to_string_lossy().into_owned());
    }
    dispatch_agent_spawn_payload(payload, "agent-spawn-response", "agent spawn", json)
}

/// `plexi agent wait <pane-id...> [--timeout <secs>] [--json]`
pub fn agent_wait_cli(pane_ids: &[u64], timeout_secs: f64, json: bool) -> i32 {
    if pane_ids.is_empty() {
        eprintln!("error: agent wait requires at least one pane id");
        return 1;
    }
    let payload = serde_json::json!({
        "type": "agent_wait",
        "pane_ids": pane_ids,
        "timeout_secs": timeout_secs,
    });
    let wait_dur = std::time::Duration::from_secs_f64(timeout_secs.max(1.0) + 5.0);
    let content = match super::request_with(payload, "agent-wait-response", "agent wait", wait_dur)
    {
        Ok(c) => c,
        Err(code) => return code,
    };
    if let Err(code) = super::check_reply_error(&content) {
        return code;
    }
    if json {
        println!("{content}");
        return 0;
    }
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
        if let Some(results) = val.get("results").and_then(|v| v.as_array()) {
            for r in results {
                let pid = r.get("pane_id").and_then(|v| v.as_u64()).unwrap_or(0);
                let agent = r.get("agent").and_then(|v| v.as_str()).unwrap_or("agent");
                let status = r.get("status").and_then(|v| v.as_str()).unwrap_or("completed");
                let summary = r.get("summary").and_then(|v| v.as_str()).unwrap_or("");
                if summary.is_empty() {
                    println!("#{pid} {agent} [{status}]");
                } else {
                    println!("#{pid} {agent} [{status}]: {summary}");
                }
            }
            return 0;
        }
    }
    println!("{content}");
    0
}

/// `plexi agent done --result <json> [--summary "<text>"] [--pane-id <id>]`
pub fn agent_done_cli(result: &str, summary: Option<&str>, pane_id_flag: Option<u64>) -> i32 {
    let Some(pane_id) = pane_id_flag.or_else(|| {
        std::env::var("PLEXI_PANE_ID")
            .ok()
            .and_then(|v| v.parse().ok())
    }) else {
        eprintln!("error: PLEXI_PANE_ID is not set — pass --pane-id or run inside a Plexi pane");
        return 1;
    };
    log::info!(
        "agent_done:cli: pane_id={pane_id} bytes={} summary_present={}",
        result.len(),
        summary.is_some()
    );
    let mut payload = serde_json::json!({
        "type": "agent_done",
        "pane_id": pane_id,
        "result": result,
    });
    if let Some(s) = summary.and_then(non_empty_string) {
        payload["summary"] = serde_json::Value::String(s.to_string());
    }
    let content = match super::request(payload, "agent-done-response", "agent done") {
        Ok(c) => c,
        Err(code) => return code,
    };
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
        if val.get("ok").and_then(|v| v.as_bool()) == Some(false) || val.get("error").is_some() {
            let err_code = val
                .get("error_code")
                .and_then(|v| v.as_str())
                .unwrap_or("error");
            let msg = val
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("agent done failed");
            eprintln!("error: {err_code}: {msg}");
            return if err_code == "invalid_result" { 2 } else { 1 };
        }
    }
    crate::program_status::emit::emit_agent_report(
        if crate::program_status::emit::done_is_failure(result) {
            crate::program_status::State::Error
        } else {
            crate::program_status::State::Done
        },
        None,
        summary.and_then(non_empty_string),
    );
    println!("{content}");
    0
}

/// `plexi agent ask "<question>" [--pane-id <id>]` — Tier 1 worker-to-master escalation.
pub fn agent_ask_cli(question: &str, pane_id_flag: Option<u64>) -> i32 {
    let Some(pane_id) = pane_id_flag.or_else(|| {
        std::env::var("PLEXI_PANE_ID")
            .ok()
            .and_then(|v| v.parse().ok())
    }) else {
        eprintln!("error: PLEXI_PANE_ID is not set — pass --pane-id or run inside a Plexi pane");
        return 1;
    };
    let Some(q) = non_empty_string(question) else {
        eprintln!("error: question cannot be empty");
        return 1;
    };
    log::info!("agent_ask:cli: pane_id={pane_id} question_len={}", q.len());
    let payload = serde_json::json!({
        "type": "agent_ask",
        "pane_id": pane_id,
        "question": q,
    });
    let content = match super::request(payload, "agent-ask-response", "agent ask") {
        Ok(c) => c,
        Err(code) => return code,
    };
    if let Err(code) = super::check_reply_error(&content) {
        return code;
    }
    crate::program_status::emit::emit_agent_report(
        crate::program_status::State::Blocked,
        Some("question"),
        Some(q),
    );
    println!("{content}");
    0
}

/// Report print-mode worker exit even when the interactive pane shell stays alive.
/// Repository checks, sandbox settings, and approval flags remain harness-owned.
pub(crate) fn supervised_worker_command(command: &str, reporter: &str, harness: &str) -> String {
    let body = format!("( {command}\n); plexi_worker_exit=$?; if [ $plexi_worker_exit -eq 0 ]; then plexi_worker_state=exited_unreported; else plexi_worker_state=failed; fi; {reporter} agent report --state $plexi_worker_state --agent {} --event HarnessExit --detail \"harness exited with code $plexi_worker_exit without agent done\"", shell_quote_arg(harness));
    format!("/bin/sh -c {}", shell_quote_arg(&body))
}

/// `plexi agent report --state <state>` — report agent state to host via socket.
pub fn agent_report_cli(
    state: &str,
    agent: &str,
    detail: Option<&str>,
    session_id: Option<&str>,
    event: Option<&str>,
    blocked_reason: Option<crate::protocol::AgentBlockedReason>,
) -> i32 {
    log::info!(
        "agent_report:cli: state={state} agent={agent} detail_present={}",
        detail.and_then(non_empty_string).is_some()
    );
    let normalized = match state.to_lowercase().as_str() {
        "working" => "working",
        "blocked" => "blocked",
        "needs_you" | "needs-you" => "needs_you",
        "interrupted" => "interrupted",
        "failed" => "failed",
        "exited_unreported" | "exited-unreported" => "exited_unreported",
        "completed" => "completed",
        "idle" => "idle",
        other => {
            eprintln!(
                "error: unknown state '{other}'; expected working, blocked, needs_you, interrupted, failed, exited_unreported, completed, or idle"
            );
            return 1;
        }
    };
    let pane_id: u64 = match std::env::var("PLEXI_PANE_ID")
        .ok()
        .and_then(|v| v.parse().ok())
    {
        Some(id) => id,
        None => {
            eprintln!("error: PLEXI_PANE_ID is not set — run this inside a Plexi terminal pane");
            return 1;
        }
    };
    let mut payload = serde_json::json!({
        "type": "set_agent_state",
        "pane_id": pane_id,
        "state": normalized,
        "agent": agent,
    });
    if let Some(active_tool) = detail.and_then(non_empty_string) {
        payload["detail"] = serde_json::Value::String(active_tool.to_string());
    }
    let resolved_session_id = session_id
        .and_then(non_empty_string)
        .map(ToOwned::to_owned)
        .or_else(|| {
            let adapter = harness_adapter(agent)?;
            let raw_json = std::env::var("PLEXI_AGENT_HOOK_JSON")
                .ok()
                .or_else(|| detail.map(ToOwned::to_owned))?;
            let val: serde_json::Value = serde_json::from_str(&raw_json).ok()?;
            adapter.capture_session_id(&val)
        });
    if let Some(sid) = resolved_session_id {
        payload["session_id"] = serde_json::Value::String(sid);
    }
    // The shared hook script uses an invocation-scoped env var so older
    // channel CLIs can still accept its unchanged state-report arguments.
    let hook_event = std::env::var("PLEXI_AGENT_EVENT").ok();
    if let Some(event) = event.or(hook_event.as_deref()).and_then(non_empty_string) {
        payload["event"] = serde_json::Value::String(event.to_string());
    }
    if let Some(reason) = blocked_reason {
        payload["blocked_reason"] = serde_json::json!(reason);
    }
    let code = super::send_to_socket(payload);
    if code == 0 {
        use crate::program_status::State;
        let (state, kind) = match normalized {
            "working" => (Some(State::Working), None),
            "blocked" | "needs_you" => (
                Some(State::Blocked),
                Some(match blocked_reason {
                    Some(crate::protocol::AgentBlockedReason::PermissionPrompt) => "permission",
                    _ => "question",
                }),
            ),
            "idle" => (Some(State::Idle), None),
            "completed" => (Some(State::Done), None),
            "failed" | "interrupted" | "exited_unreported" => (Some(State::Error), None),
            _ => (None, None),
        };
        if let Some(state) = state {
            crate::program_status::emit::emit_agent_report(
                state,
                kind,
                detail.and_then(non_empty_string),
            );
        }
    }
    code
}

/// `plexi agent status` — query agent states for all panes.
pub fn agent_status_cli(blocked: bool, working: bool, idle: bool) -> i32 {
    log::info!("agent_status:cli: blocked={blocked} working={working} idle={idle}");
    let content = match super::request(
        serde_json::json!({ "type": "get_agent_states" }),
        "plexi-agent-states",
        "agent states",
    ) {
        Ok(content) => content,
        Err(code) => return code,
    };
    let states: Vec<serde_json::Value> = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: could not parse agent states: {e}");
            return 1;
        }
    };
    let filter_active = blocked || working || idle;
    let filtered: Vec<&serde_json::Value> = states
        .iter()
        .filter(|s| {
            if !filter_active {
                return true;
            }
            let st = s["state"].as_str().unwrap_or("");
            (blocked && st == "blocked") || (working && st == "working") || (idle && st == "idle")
        })
        .collect();
    if filtered.is_empty() {
        println!("No agent states tracked.");
        return 0;
    }
    let show_detail = filtered
        .iter()
        .any(|s| s.get("detail").and_then(|v| v.as_str()).is_some());
    if show_detail {
        println!(
            "{:<12} {:<16} {:<12} {:<20} DETAIL",
            "PANE_ID", "AGENT", "STATE", "SESSION_ID"
        );
    } else {
        println!(
            "{:<12} {:<16} {:<12} SESSION_ID",
            "PANE_ID", "AGENT", "STATE"
        );
    }
    for s in &filtered {
        let pane_id = s["pane_id"].as_u64().unwrap_or(0);
        let agent = s["agent"].as_str().unwrap_or("unknown");
        let state = s["state"].as_str().unwrap_or("unknown");
        let session_id = s.get("session_id").and_then(|v| v.as_str()).unwrap_or("-");
        if show_detail {
            let detail = s.get("detail").and_then(|v| v.as_str()).unwrap_or("-");
            println!(
                "{:<12} {:<16} {:<12} {:<20} {}",
                pane_id, agent, state, session_id, detail
            );
        } else {
            println!("{:<12} {:<16} {:<12} {}", pane_id, agent, state, session_id);
        }
    }
    0
}

/// `plexi agent history [--limit <n>] [--json]` — list closed agent runs and sessions.
pub fn agent_history_cli(limit: usize, json: bool) -> i32 {
    log::info!("agent_history:cli: limit={limit} json={json}");
    let mut records = crate::workspace::load_agent_history();
    records.reverse();
    if limit > 0 && records.len() > limit {
        records.truncate(limit);
    }
    if json {
        match serde_json::to_string_pretty(&records) {
            Ok(out) => println!("{out}"),
            Err(e) => {
                eprintln!("error: failed to serialize agent history: {e}");
                return 1;
            }
        }
        return 0;
    }
    if records.is_empty() {
        println!("No closed agent runs recorded.");
        return 0;
    }
    println!(
        "{:<24} {:<10} {:<20} {:<12} {:<14} {:<22} SUMMARY",
        "RUN_ID", "HARNESS", "SESSION_ID", "STATE", "ROLE", "CLOSED_AT"
    );
    for r in &records {
        let session_id = r.session_id.as_deref().unwrap_or("-");
        let role = r
            .config_name
            .as_deref()
            .or(r.pane_name.as_deref())
            .unwrap_or("-");
        let summary = r.summary.as_deref().unwrap_or("-");
        println!(
            "{:<24} {:<10} {:<20} {:<12} {:<14} {:<22} {}",
            r.run_id,
            r.harness,
            session_id,
            r.last_state.as_str(),
            role,
            r.closed_at,
            summary
        );
    }
    0
}

/// `plexi agent reopen <run-id|session-id> [--subcontext <name>] [--json]` — reopen a closed agent run.
pub fn agent_reopen_cli(target: &str, subcontext: Option<&str>, json: bool) -> i32 {
    let Some(query) = non_empty_string(target) else {
        eprintln!("error: run-id or session-id cannot be empty");
        return 1;
    };
    log::info!("agent_reopen:cli: query={query} subcontext={subcontext:?} json={json}");
    let mut payload = serde_json::json!({
        "type": "agent_reopen",
        "query": query,
    });
    if let Some(sub) = subcontext.and_then(non_empty_string) {
        payload["subcontext"] = serde_json::Value::String(sub.to_string());
    }
    let content = match super::request(payload, "agent-reopen-response", "agent reopen") {
        Ok(c) => c,
        Err(code) => return code,
    };
    if let Err(code) = super::check_reply_error(&content) {
        return code;
    }
    if json {
        println!("{content}");
        return 0;
    }
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
        if let Some(pid) = v.get("pane_id").and_then(|v| v.as_u64()) {
            let cmd = v.get("command").and_then(|v| v.as_str()).unwrap_or("");
            println!("Reopened in pane {pid}: {cmd}");
            return 0;
        }
    }
    println!("{content}");
    0
}

fn non_empty_string(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

/// `plexi agent hook install` — write hook script and patch selected agent configs.
pub fn agent_hook_install_cli(claude_code: bool, codex: bool, pi: bool) -> i32 {
    if !claude_code && !codex && !pi {
        eprintln!("error: specify at least one of --claude-code, --codex, or --pi");
        return 1;
    }
    log::info!("agent_hook_install:cli: claude-code={claude_code} codex={codex} pi={pi}");

    // Hooks are global but execute inside panes from every release channel.
    // The bare shim honors the pane's PLEXI_SOCKET, while a channel-suffixed
    // binary would report to whichever channel last installed hooks. The
    // script itself lives under the channel-neutral shared dir for the same
    // reason: every channel writes byte-identical content there, so the path
    // patched into the agent's global config survives uninstalling whichever
    // channel happened to run install last.
    let script_path = match write_agent_state_script("plexi") {
        Ok(path) => path,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    let script_str = script_path.to_string_lossy().to_string();
    println!("Hook script: {script_str}");

    let mut status = 0;
    if claude_code {
        status |= install_claude_code_hooks(&script_str);
    }
    if codex {
        status |= install_codex_hooks(&script_str);
    }
    if pi {
        status |= install_pi_extension();
    }
    status
}

/// `plexi agent hook uninstall` — remove PLEXI hook entries from selected agent configs.
pub fn agent_hook_uninstall_cli(claude_code: bool, codex: bool, pi: bool) -> i32 {
    if !claude_code && !codex && !pi {
        eprintln!("error: specify at least one of --claude-code, --codex, or --pi");
        return 1;
    }
    log::info!("agent_hook_uninstall:cli: claude-code={claude_code} codex={codex} pi={pi}");

    let mut status = 0;
    if claude_code {
        status |= uninstall_claude_code_hooks();
    }
    if codex {
        status |= uninstall_codex_hooks();
    }
    if pi {
        status |= uninstall_pi_extension();
    }
    status
}

fn write_agent_state_script(binary: &str) -> Result<PathBuf, String> {
    publish_agent_state_script(binary, true)
}

/// Restore the shared executable before starting panes; agent registrations
/// remain opt-in through `agent hook install`.
pub(crate) fn ensure_agent_state_script() -> Result<(), String> {
    let path = crate::config::shared_dir()
        .join("hooks")
        .join(PLEXI_AGENT_STATE_SCRIPT);
    match std::fs::symlink_metadata(&path) {
        Ok(_) => return Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            return Err(format!(
                "could not inspect hook script {}: {e}",
                path.display()
            ));
        }
    }
    publish_agent_state_script("plexi", false)?;
    Ok(())
}

fn publish_agent_state_script(binary: &str, replace: bool) -> Result<PathBuf, String> {
    use std::io::Write;
    let hooks_dir = crate::config::shared_dir().join("hooks");
    std::fs::create_dir_all(&hooks_dir)
        .map_err(|e| format!("could not create hooks dir {}: {e}", hooks_dir.display()))?;
    let script_path = hooks_dir.join(PLEXI_AGENT_STATE_SCRIPT);
    let mut staged = tempfile::NamedTempFile::new_in(&hooks_dir)
        .map_err(|e| format!("could not stage hook script {}: {e}", script_path.display()))?;
    staged
        .write_all(agent_state_script(binary).as_bytes())
        .map_err(|e| format!("could not write hook script {}: {e}", script_path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        staged
            .as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("could not chmod hook script {}: {e}", script_path.display()))?;
    }
    let result = if replace {
        staged.persist(&script_path)
    } else {
        staged.persist_noclobber(&script_path)
    };
    match result {
        Ok(_) => {}
        Err(e) if !replace && e.error.kind() == std::io::ErrorKind::AlreadyExists => {
            return Ok(script_path);
        }
        Err(e) => {
            return Err(format!(
                "could not publish hook script {}: {e}",
                script_path.display()
            ));
        }
    }
    log::info!(
        "agent_hook_install: wrote hook script to {}",
        script_path.display()
    );
    Ok(script_path)
}

/// The registered hook command is a thin wrapper: all parsing happens in
/// `plexi agent hook-event` (native Rust, no `jq`, no state logic in shell).
fn agent_state_script(binary: &str) -> String {
    format!(
        r#"#!/bin/sh
if [ -z "${{PLEXI_SOCKET:-}}" ] || [ -z "${{PLEXI_PANE_ID:-}}" ]; then
    [ -t 0 ] || cat >/dev/null
    exit 0
fi
exec "{binary}" agent hook-event --agent "${{PLEXI_AGENT_NAME:-claude-code}}"
"#
    )
}

/// One state report derived from a harness hook payload.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct HookReport {
    pub state: &'static str,
    pub detail: Option<String>,
    pub session_id: Option<String>,
    pub event: String,
}

fn hook_str<'a>(input: &'a serde_json::Value, paths: &[&[&str]]) -> Option<&'a str> {
    paths.iter().find_map(|path| {
        let mut cur = input;
        for key in *path {
            cur = cur.get(key)?;
        }
        cur.as_str().and_then(non_empty_string)
    })
}

fn truncate_chars(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}

fn hook_tool_detail(input: &serde_json::Value) -> Option<String> {
    let tool = hook_str(input, &[&["tool_name"], &["toolName"]])?;
    let path_value = || {
        hook_str(
            input,
            &[
                &["tool_input", "file_path"],
                &["tool_input", "path"],
                &["input", "file_path"],
                &["input", "path"],
                &["args", "file_path"],
                &["args", "path"],
            ],
        )
        .and_then(|p| Path::new(p).file_name())
        .map(|n| n.to_string_lossy().into_owned())
    };
    let labelled = |label: &str, value: Option<String>| match value {
        Some(v) => format!("{label}: {v}"),
        None => label.to_string(),
    };
    Some(match tool {
        "Bash" | "bash" => labelled(
            "Bash",
            hook_str(
                input,
                &[
                    &["tool_input", "command"],
                    &["tool_input", "cmd"],
                    &["input", "command"],
                    &["args", "command"],
                ],
            )
            .map(|v| truncate_chars(v, 60)),
        ),
        "apply_patch" => labelled(
            "Patch",
            hook_str(input, &[&["tool_input", "command"], &["input", "command"]])
                .map(|v| truncate_chars(v, 60)),
        ),
        "Edit" | "edit" => labelled("Edit", path_value()),
        "Write" | "write" => labelled("Write", path_value()),
        "Read" | "read" => labelled("Read", path_value()),
        "WebSearch" => labelled(
            "WebSearch",
            hook_str(input, &[&["tool_input", "query"], &["input", "query"]])
                .map(|v| truncate_chars(v, 50)),
        ),
        "WebFetch" => labelled(
            "WebFetch",
            hook_str(input, &[&["tool_input", "url"], &["input", "url"]])
                .map(|v| truncate_chars(v, 60)),
        ),
        "Agent" => labelled(
            "Agent",
            hook_str(
                input,
                &[
                    &["tool_input", "description"],
                    &["tool_input", "prompt"],
                    &["input", "description"],
                    &["input", "prompt"],
                ],
            )
            .map(|v| truncate_chars(v, 60)),
        ),
        other => other.to_string(),
    })
}

/// Map a Claude Code / Codex / Pi hook payload to a state report. Events that
/// carry no state (subagent lifecycle, unknown names) return `None`.
pub(crate) fn map_hook_payload(input: &serde_json::Value) -> Option<HookReport> {
    let event = hook_str(input, &[&["hook_event_name"]])?;
    let (state, with_tool) = match event {
        "PreToolUse" | "tool_call" | "PostToolUse" | "PostToolBatch" | "tool_result" => {
            ("working", true)
        }
        "PermissionRequest" => ("blocked", true),
        "UserPromptSubmit" | "agent_start" | "before_agent_start" => ("working", false),
        "SessionStart" | "session_start" | "Stop" | "agent_end" | "StopFailure" | "SessionEnd"
        | "session_shutdown" => ("idle", false),
        "UsageLimit" | "BootFailure" => ("blocked", false),
        _ => return None,
    };
    Some(HookReport {
        state,
        detail: if with_tool { hook_tool_detail(input) } else { None },
        session_id: hook_str(input, &[&["session_id"], &["sessionId"]]).map(str::to_string),
        event: event.to_string(),
    })
}

/// `plexi agent hook-event [--agent <name>]` — read one harness hook payload
/// from stdin and report the derived state. Never fails the calling hook.
pub fn agent_hook_event_cli(agent: Option<&str>) -> i32 {
    use std::io::Read;
    let mut raw = String::new();
    let _ = std::io::stdin().read_to_string(&mut raw);
    if std::env::var_os("PLEXI_SOCKET").is_none() || std::env::var_os("PLEXI_PANE_ID").is_none() {
        return 0;
    }
    let Ok(input) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return 0;
    };
    let Some(report) = map_hook_payload(&input) else {
        return 0;
    };
    let agent = agent
        .and_then(non_empty_string)
        .map(str::to_string)
        .or_else(|| std::env::var("PLEXI_AGENT_NAME").ok())
        .unwrap_or_else(|| "claude-code".to_string());
    let _ = agent_report_cli(
        report.state,
        &agent,
        report.detail.as_deref(),
        report.session_id.as_deref(),
        Some(&report.event),
        None,
    );
    0
}

/// Hook command registered with harnesses: no shell, no `jq`.
const NATIVE_HOOK_COMMAND: &str = "plexi agent hook-event --agent";

/// Settings JSON Claude Code merges via `--settings`, registering the state hooks.
fn claude_state_settings_json() -> String {
    let command = format!("{NATIVE_HOOK_COMMAND} claude-code");
    let mut hooks = serde_json::Map::new();
    for event in CLAUDE_CODE_HOOK_EVENTS {
        hooks.insert(
            (*event).to_string(),
            serde_json::json!([{"hooks": [{"type": "command", "command": command}]}]),
        );
    }
    serde_json::to_string_pretty(&serde_json::json!({ "hooks": hooks })).unwrap_or_default()
}

fn write_if_changed(path: &Path, content: &str) -> Result<(), String> {
    if std::fs::read_to_string(path).is_ok_and(|existing| existing == content) {
        return Ok(());
    }
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)
        .map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    let mut staged = tempfile::NamedTempFile::new_in(dir)
        .map_err(|e| format!("could not stage {}: {e}", path.display()))?;
    std::io::Write::write_all(&mut staged, content.as_bytes())
        .map_err(|e| format!("could not write {}: {e}", path.display()))?;
    staged
        .persist(path)
        .map_err(|e| format!("could not publish {}: {e}", path.display()))?;
    Ok(())
}

/// Insert extra launch flags directly after the harness binary.
fn insert_after_binary(command: &str, binary: &str, flags: &[String]) -> String {
    match command.strip_prefix(binary) {
        Some(rest) => format!("{binary} {}{rest}", flags.join(" ")),
        None => command.to_string(),
    }
}

/// Make state reporting work on first launch with zero `plexi agent hook
/// install`: provision channel-neutral hook files under the shared dir and add
/// the matching launch flags (`claude --settings`, `pi -e`). Codex has no
/// per-launch hook flag, so it keeps the opt-in `hook install --codex`.
/// Provisioning failures are logged and the command is returned unchanged.
pub fn with_auto_state_hooks(adapter: &HarnessAdapter, command: String) -> String {
    let hooks_dir = crate::config::shared_dir().join("hooks");
    let result = match adapter.harness {
        AgentHarness::Claude => {
            let path = hooks_dir.join(PLEXI_CLAUDE_SETTINGS);
            write_if_changed(&path, &claude_state_settings_json()).map(|_| {
                vec![
                    "--settings".to_string(),
                    shell_quote_arg(&path.to_string_lossy()),
                ]
            })
        }
        AgentHarness::Pi => {
            let path = hooks_dir.join(PLEXI_PI_EXTENSION);
            write_if_changed(&path, &pi_extension_script()).map(|_| {
                vec!["-e".to_string(), shell_quote_arg(&path.to_string_lossy())]
            })
        }
        _ => return command,
    };
    match result {
        Ok(flags) => insert_after_binary(&command, adapter.binary, &flags),
        Err(e) => {
            log::warn!("agent_hook: auto-provision for {} failed: {e}", adapter.binary);
            command
        }
    }
}

fn install_claude_code_hooks(script_str: &str) -> i32 {
    let settings_path = claude_settings_path();
    let code = install_json_agent_hooks(
        &settings_path,
        CLAUDE_CODE_HOOK_EVENTS,
        script_str,
        None,
        "Claude Code",
    );
    if code == 0 {
        println!(
            "Registered Claude Code hooks in {} for {} lifecycle events.",
            settings_path.display(),
            CLAUDE_CODE_HOOK_EVENTS.len()
        );
    }
    code
}

fn install_codex_hooks(script_str: &str) -> i32 {
    let hooks_path = codex_hooks_path();
    let command = format!("PLEXI_AGENT_NAME=codex {script_str}");
    let code = install_json_agent_hooks(
        &hooks_path,
        CODEX_HOOK_EVENTS,
        &command,
        Some("Plexi agent state"),
        "Codex",
    );
    if code == 0 {
        println!(
            "Registered Codex hooks in {} for {} lifecycle events.",
            hooks_path.display(),
            CODEX_HOOK_EVENTS.len()
        );
        println!("Run `/hooks` in Codex once to trust the new hook definitions.");
    }
    code
}

fn install_json_agent_hooks(
    path: &Path,
    events: &[&str],
    command: &str,
    status_message: Option<&str>,
    label: &str,
) -> i32 {
    let mut settings = match read_json_settings(path) {
        Ok(settings) => settings,
        Err(e) => {
            eprintln!("error: refusing to modify {label} config: {e}");
            return 1;
        }
    };
    let changed = match register_agent_hooks(&mut settings, events, command, status_message) {
        Ok(changed) => changed,
        Err(e) => {
            eprintln!("error: refusing to modify {label} config: {e}");
            return 1;
        }
    };
    if changed {
        write_json_settings(path, &settings)
    } else {
        0
    }
}

/// Point every lifecycle event at `command`, dropping any Plexi entry already
/// there. Re-installing from a second channel therefore rewrites stale
/// channel-suffixed script paths instead of stacking a duplicate beside them.
fn register_agent_hooks(
    settings: &mut serde_json::Value,
    events: &[&str],
    command: &str,
    status_message: Option<&str>,
) -> Result<bool, String> {
    if !settings.is_object() {
        return Err("top-level JSON value is not an object".to_string());
    }
    if settings.get("hooks").is_none() {
        settings["hooks"] = serde_json::json!({});
    } else if !settings["hooks"].is_object() {
        return Err("the existing 'hooks' value is not an object".to_string());
    }
    let mut changed = false;
    for event in events {
        changed |= register_json_hook(settings, event, command, status_message)?;
    }
    Ok(changed)
}

fn register_json_hook(
    settings: &mut serde_json::Value,
    event: &str,
    command: &str,
    status_message: Option<&str>,
) -> Result<bool, String> {
    let event_value = &settings["hooks"][event];
    if !event_value.is_null() && !event_value.is_array() {
        return Err(format!("the existing hooks.{event} value is not an array"));
    }
    let event_hooks = event_value.as_array().cloned().unwrap_or_default();

    let mut hook = serde_json::json!({"type": "command", "command": command});
    if let Some(message) = status_message {
        hook["statusMessage"] = serde_json::Value::String(message.to_string());
    }
    let new_entry = serde_json::json!({
        "matcher": "",
        "hooks": [hook]
    });
    let mut next_entries: Vec<serde_json::Value> = event_hooks
        .into_iter()
        .filter(|entry| !contains_plexi_agent_hook(entry))
        .collect();
    next_entries.push(new_entry);
    let changed = event_value.as_array() != Some(&next_entries);
    if changed {
        settings["hooks"][event] = serde_json::Value::Array(next_entries);
    }
    Ok(changed)
}

fn install_pi_extension() -> i32 {
    let extension_path = pi_extension_path();
    install_pi_extension_at(&extension_path)
}

pub(crate) fn install_pi_extension_at(extension_path: &Path) -> i32 {
    let content = pi_extension_script();
    if extension_path.exists() {
        let existing = match std::fs::read_to_string(extension_path) {
            Ok(existing) => existing,
            Err(e) => {
                eprintln!("error: could not read {}: {e}", extension_path.display());
                return 1;
            }
        };
        if existing == content {
            return 0;
        }
        if !is_plexi_pi_extension(&existing) {
            eprintln!(
                "error: refusing to overwrite unknown Pi extension {}; move it aside first.",
                extension_path.display()
            );
            return 1;
        }
    }
    if let Some(parent) = extension_path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("error: could not create {}: {e}", parent.display());
            return 1;
        }
    }
    if let Err(e) = backup_existing_file(extension_path) {
        eprintln!("error: {e}");
        return 1;
    }
    if let Err(e) = std::fs::write(extension_path, content) {
        eprintln!("error: could not write {}: {e}", extension_path.display());
        return 1;
    }
    log::info!(
        "agent_hook_install: wrote Pi extension to {}",
        extension_path.display()
    );
    println!("Registered Pi extension at {}.", extension_path.display());
    0
}

/// Migrate a legacy Plexi-generated `~/.pi/agent/extensions/plexi-agent-state.ts`
/// to `.plexi-hook.bak` and remove the active file so `pi-plexi` owns lifecycle
/// reporting without duplicate state events. Refuses to touch unknown user-authored
/// files.
pub fn migrate_legacy_pi_extension_at(extension_path: &Path) -> Result<bool, String> {
    if !extension_path.exists() {
        return Ok(false);
    }
    let existing = std::fs::read_to_string(extension_path)
        .map_err(|e| format!("could not read {}: {e}", extension_path.display()))?;
    if !is_plexi_pi_extension(&existing) {
        return Err(format!(
            "refusing to migrate unknown Pi extension {}; move it aside manually",
            extension_path.display()
        ));
    }
    backup_existing_file(extension_path)?;
    std::fs::remove_file(extension_path)
        .map_err(|e| format!("could not remove {}: {e}", extension_path.display()))?;
    log::info!(
        "agent_hook: migrated legacy Pi extension {}",
        extension_path.display()
    );
    Ok(true)
}

fn pi_extension_script() -> String {
    r#"// Plexi-managed agent state extension.
import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

const plexiBin = process.env.PLEXI_CLI_BIN || "plexi";
const PLEXI_PI_PACKAGE_ACTIVE = "__PLEXI_PI_PACKAGE_ACTIVE__";
let turnStarted = false;

function isPiPlexiPackageActive(): boolean {
  return Boolean((globalThis as Record<string, unknown>)[PLEXI_PI_PACKAGE_ACTIVE]) ||
    process.env.PLEXI_PI_PACKAGE_ACTIVE === "1";
}

function isSessionPersisted(ctx: any): boolean {
  if (turnStarted) return true;
  try {
    const file = ctx?.sessionManager?.getSessionFile?.() ?? "";
    return typeof file === "string" && file.trim().length > 0 && existsSync(file.trim());
  } catch {
    return false;
  }
}

function sessionId(ctx: any, requirePersisted = false): string {
  if (requirePersisted && !isSessionPersisted(ctx)) return "";
  try {
    const id = ctx?.sessionManager?.getSessionId?.();
    if (typeof id === "string" && id.trim()) return id.trim();
    const file = ctx?.sessionManager?.getSessionFile?.();
    if (typeof file === "string" && file.trim()) return file.trim();
  } catch {
  }
  return "";
}

function report(eventName: string, payload: Record<string, unknown>, ctx: any) {
  if (isPiPlexiPackageActive()) return;
  if (!process.env.PLEXI_SOCKET || !process.env.PLEXI_PANE_ID) return;
  if (eventName === "session_start") {
    turnStarted = false;
  } else if (
    eventName === "before_agent_start" ||
    eventName === "agent_start" ||
    eventName === "tool_call" ||
    eventName === "tool_result" ||
    eventName === "agent_end"
  ) {
    turnStarted = true;
  }
  const requirePersisted = eventName === "session_start" || eventName === "session_shutdown";
  try {
    const child = spawn(plexiBin, ["agent", "hook-event", "--agent", "pi"], {
      env: { ...process.env, PLEXI_AGENT_NAME: "pi" },
      stdio: ["pipe", "ignore", "ignore"],
    });
    child.on("error", () => {});
    child.stdin.on("error", () => {});
    child.stdin.end(JSON.stringify({
      hook_event_name: eventName,
      session_id: sessionId(ctx, requirePersisted),
      ...payload,
    }));
  } catch {
  }
}

export default function (pi: ExtensionAPI) {
  pi.on("session_start", (event, ctx) => report("session_start", { reason: event.reason }, ctx));
  pi.on("before_agent_start", (_event, ctx) => report("before_agent_start", {}, ctx));
  pi.on("agent_start", (_event, ctx) => report("agent_start", {}, ctx));
  pi.on("tool_call", (event, ctx) => report("tool_call", {
    tool_name: event.toolName,
    tool_input: event.args,
    tool_use_id: event.toolCallId,
  }, ctx));
  pi.on("tool_result", (event, ctx) => report("tool_result", {
    tool_name: event.toolName,
    tool_input: event.input,
    tool_use_id: event.toolCallId,
    is_error: event.isError,
  }, ctx));
  pi.on("agent_end", (_event, ctx) => report("agent_end", {}, ctx));
  pi.on("session_shutdown", (_event, ctx) => report("session_shutdown", {}, ctx));
}
"#
    .to_string()
}

fn uninstall_claude_code_hooks() -> i32 {
    let settings_path = claude_settings_path();
    uninstall_json_hooks(&settings_path, CLAUDE_CODE_HOOK_EVENTS, "Claude Code")
}

fn uninstall_codex_hooks() -> i32 {
    let hooks_path = codex_hooks_path();
    uninstall_json_hooks(&hooks_path, CODEX_HOOK_EVENTS, "Codex")
}

fn uninstall_json_hooks(path: &Path, events: &[&str], label: &str) -> i32 {
    if !path.exists() {
        println!("Nothing to uninstall - {} does not exist.", path.display());
        return 0;
    }

    let mut settings = match read_json_settings(path) {
        Ok(settings) => settings,
        Err(e) => {
            eprintln!("error: refusing to modify {label} config: {e}");
            return 1;
        }
    };
    let mut removed = 0usize;

    if let Some(hooks_map) = settings.get_mut("hooks").and_then(|h| h.as_object_mut()) {
        for event in events {
            let remove_empty_event =
                if let Some(event_arr) = hooks_map.get_mut(*event).and_then(|v| v.as_array_mut()) {
                    let before = event_arr.len();
                    event_arr.retain(|entry| !contains_plexi_agent_hook(entry));
                    removed += before - event_arr.len();
                    event_arr.is_empty()
                } else {
                    false
                };
            if remove_empty_event {
                hooks_map.remove(*event);
            }
        }
    }

    if removed == 0 {
        println!("No PLEXI {label} hook entries found in {}.", path.display());
        return 0;
    }

    let code = write_json_settings(path, &settings);
    if code == 0 {
        println!(
            "Removed {removed} PLEXI {label} hook entr{} from {}.",
            if removed == 1 { "y" } else { "ies" },
            path.display()
        );
    }
    code
}

fn uninstall_pi_extension() -> i32 {
    let extension_path = pi_extension_path();
    uninstall_pi_extension_at(&extension_path)
}

pub(crate) fn uninstall_pi_extension_at(extension_path: &Path) -> i32 {
    if !extension_path.exists() {
        println!(
            "Nothing to uninstall - {} does not exist.",
            extension_path.display()
        );
        return 0;
    }
    match migrate_legacy_pi_extension_at(extension_path) {
        Ok(_) => {
            println!("Removed Pi extension {}.", extension_path.display());
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

pub fn is_plexi_pi_extension(content: &str) -> bool {
    content.contains("Plexi-managed agent state extension.")
        || (content.contains("const hookCommand =") && content.contains("PLEXI_AGENT_NAME: \"pi\""))
        || (content.contains("const plexiBin =") && content.contains("PLEXI_AGENT_NAME: \"pi\""))
}

fn contains_plexi_agent_hook(entry: &serde_json::Value) -> bool {
    entry
        .get("hooks")
        .and_then(|h| h.as_array())
        .map(|arr| {
            arr.iter().any(|h| {
                h.get("command")
                    .and_then(|c| c.as_str())
                    .map(|c| {
                        c.contains(PLEXI_AGENT_STATE_SCRIPT) || c.contains("plexi agent report")
                    })
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

fn claude_settings_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".claude")
        .join("settings.json")
}

fn codex_hooks_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".codex")
        .join("hooks.json")
}

fn pi_extension_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".pi")
        .join("agent")
        .join("extensions")
        .join(PLEXI_PI_EXTENSION)
}

fn read_json_settings(path: &Path) -> Result<serde_json::Value, String> {
    if !path.exists() {
        return Ok(serde_json::json!({}));
    }
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("could not read {}: {e}", path.display()))?;
    serde_json::from_str(&content).map_err(|e| format!("could not parse {}: {e}", path.display()))
}

fn write_json_settings(path: &Path, settings: &serde_json::Value) -> i32 {
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("error: could not create {}: {e}", parent.display());
            return 1;
        }
    }
    let json = match serde_json::to_string_pretty(settings) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: could not serialize settings: {e}");
            return 1;
        }
    };
    if let Err(e) = backup_existing_file(path) {
        eprintln!("error: {e}");
        return 1;
    }
    let tmp_path = path.with_extension("json.tmp");
    if let Err(e) = std::fs::write(&tmp_path, &json) {
        eprintln!("error: could not write temp settings: {e}");
        return 1;
    }
    if let Err(e) = std::fs::rename(&tmp_path, path) {
        eprintln!("error: could not rename temp settings: {e}");
        let _ = std::fs::remove_file(&tmp_path);
        return 1;
    }
    log::info!("agent_hook: wrote {}", path.display());
    0
}

/// Preserve the first pre-Plexi configuration before any mutation. Later
/// installs and uninstalls deliberately keep that original recovery point.
fn backup_existing_file(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    let backup = path.with_extension(format!(
        "{}.plexi-hook.bak",
        path.extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("bak")
    ));
    if backup.exists() {
        return Ok(());
    }
    std::fs::copy(path, &backup).map_err(|e| {
        format!(
            "could not back up {} to {}: {e}",
            path.display(),
            backup.display()
        )
    })?;
    log::info!(
        "agent_hook: backed up {} to {}",
        path.display(),
        backup.display()
    );
    Ok(())
}

#[cfg(test)]
mod agent_tests {
    use std::fs;

    #[test]
    fn workspace_not_found_error_uses_the_active_channel_directory() {
        let _channel = crate::config::set_test_channel("alpha");
        let error = super::workspace_not_found_error(std::path::Path::new("/tmp/no-workspace"));
        assert!(error.contains(".plexi-alpha/ workspace"), "{error}");
        assert!(!error.contains("no .plexi/ workspace"), "{error}");
    }

    #[test]
    fn register_json_hook_is_idempotent_and_agent_specific() {
        let mut settings = serde_json::json!({});

        super::register_json_hook(
            &mut settings,
            "PreToolUse",
            "PLEXI_AGENT_NAME=codex /tmp/claude-code-agent-state.sh",
            Some("Plexi agent state"),
        )
        .unwrap();
        super::register_json_hook(
            &mut settings,
            "PreToolUse",
            "PLEXI_AGENT_NAME=codex /tmp/claude-code-agent-state.sh",
            Some("Plexi agent state"),
        )
        .unwrap();
        super::register_json_hook(
            &mut settings,
            "PreToolUse",
            "PLEXI_AGENT_NAME=codex /tmp/current/claude-code-agent-state.sh",
            Some("Plexi agent state"),
        )
        .unwrap();

        let hooks = settings["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(hooks.len(), 1);
        assert_eq!(
            hooks[0]["hooks"][0]["command"],
            "PLEXI_AGENT_NAME=codex /tmp/current/claude-code-agent-state.sh"
        );
        assert_eq!(hooks[0]["hooks"][0]["statusMessage"], "Plexi agent state");
    }

    #[test]
    fn native_hook_payloads_map_states_without_jq_or_bash() {
        let report = |v: serde_json::Value| super::map_hook_payload(&v);
        let r = report(serde_json::json!({"hook_event_name": "SessionStart", "session_id": "s1"}))
            .unwrap();
        assert_eq!((r.state, r.session_id.as_deref(), r.event.as_str()), ("idle", Some("s1"), "SessionStart"));
        assert_eq!(
            report(serde_json::json!({"hook_event_name": "UserPromptSubmit"})).unwrap().state,
            "working"
        );
        let r = report(serde_json::json!({
            "hook_event_name": "PreToolUse", "tool_name": "Bash",
            "tool_input": {"command": "cargo test --lib"}
        }))
        .unwrap();
        assert_eq!((r.state, r.detail.as_deref()), ("working", Some("Bash: cargo test --lib")));
        let r = report(serde_json::json!({
            "hook_event_name": "PermissionRequest", "tool_name": "Edit",
            "tool_input": {"file_path": "/repo/src/main.rs"}
        }))
        .unwrap();
        assert_eq!((r.state, r.detail.as_deref()), ("blocked", Some("Edit: main.rs")));
        // Stop, StopFailure and SessionEnd keep distinct raw provenance.
        let events: Vec<_> = ["Stop", "StopFailure", "SessionEnd"]
            .iter()
            .map(|e| report(serde_json::json!({"hook_event_name": e, "sessionId": "s2"})).unwrap())
            .collect();
        assert!(events.iter().all(|r| r.state == "idle" && r.session_id.as_deref() == Some("s2")));
        assert_eq!(
            events.iter().map(|r| r.event.as_str()).collect::<Vec<_>>(),
            ["Stop", "StopFailure", "SessionEnd"]
        );
        // Pi event names and tool-name-only events.
        assert_eq!(report(serde_json::json!({"hook_event_name": "agent_end"})).unwrap().state, "idle");
        let r = report(serde_json::json!({"hook_event_name": "tool_call", "toolName": "mcp_x"})).unwrap();
        assert_eq!(r.detail.as_deref(), Some("mcp_x"));
        // Stateless and unknown events are ignored.
        assert!(report(serde_json::json!({"hook_event_name": "SubagentStart"})).is_none());
        assert!(report(serde_json::json!({"hook_event_name": "Whatever"})).is_none());
        assert!(report(serde_json::json!({"no_event": true})).is_none());
        // Long details are truncated on char boundaries.
        let long = "é".repeat(100);
        let r = report(serde_json::json!({
            "hook_event_name": "PreToolUse", "tool_name": "Bash", "tool_input": {"command": long}
        }))
        .unwrap();
        assert_eq!(r.detail.unwrap().chars().count(), "Bash: ".len() + 60);
    }

    #[test]
    fn registered_hook_script_is_a_thin_exec_wrapper() {
        let shell = super::agent_state_script("plexi");
        assert!(!shell.contains("jq"), "no jq dependency");
        assert!(shell.contains("exec \"plexi\" agent hook-event --agent"));
    }

    #[test]
    fn pi_extension_calls_plexi_directly() {
        let pi = super::pi_extension_script();
        assert!(pi.contains("PLEXI_AGENT_NAME: \"pi\""));
        assert!(pi.contains("pi.on(\"tool_call\""));
        assert!(pi.contains("pi.on(\"agent_end\""));
        assert!(pi.contains("spawn(plexiBin, [\"agent\", \"hook-event\", \"--agent\", \"pi\"]"));
        assert!(!pi.contains("bash") && !pi.contains("jq"));
        assert!(super::is_plexi_pi_extension(&pi));
    }

    #[test]
    fn spawn_auto_provisions_hooks_for_claude_and_pi_without_hook_install() {
        let shared = tempfile::tempdir().unwrap();
        let _guard = crate::config::set_test_shared_dir(shared.path().to_path_buf());
        let hooks = shared.path().join("hooks");
        let adapter = |h| super::HARNESS_ADAPTERS.iter().find(|a| a.harness == h).unwrap();

        let claude = adapter(super::AgentHarness::Claude);
        let cmd = super::with_auto_state_hooks(claude, "claude --model x 'task'".to_string());
        let settings_path = hooks.join("plexi-claude-settings.json");
        assert!(
            cmd.starts_with(&format!("claude --settings '{}' --model x", settings_path.display()))
                || cmd.starts_with(&format!("claude --settings {} --model x", settings_path.display())),
            "{cmd}"
        );
        let settings: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&settings_path).unwrap()).unwrap();
        for event in super::CLAUDE_CODE_HOOK_EVENTS {
            assert_eq!(
                settings["hooks"][*event][0]["hooks"][0]["command"],
                "plexi agent hook-event --agent claude-code"
            );
        }

        let pi = adapter(super::AgentHarness::Pi);
        let cmd = super::with_auto_state_hooks(pi, "pi --no-extensions --no-mcp 'task'".to_string());
        let ext = hooks.join("plexi-agent-state.ts");
        assert!(cmd.contains("--no-extensions") && cmd.contains("-e "), "{cmd}");
        assert!(cmd.contains(&ext.to_string_lossy().to_string()), "{cmd}");
        assert!(cmd.starts_with("pi -e "), "{cmd}");
        assert!(super::is_plexi_pi_extension(&fs::read_to_string(&ext).unwrap()));

        // Other harnesses are untouched, and provisioning is idempotent.
        let codex = adapter(super::AgentHarness::Codex);
        assert_eq!(super::with_auto_state_hooks(codex, "codex 'x'".to_string()), "codex 'x'");
        let again = super::with_auto_state_hooks(pi, "pi --no-extensions 'task'".to_string());
        assert!(again.starts_with("pi -e "));
    }

    #[test]
    fn hook_script_lands_in_the_channel_neutral_shared_dir() {
        let shared = tempfile::tempdir().unwrap();
        let profile = tempfile::tempdir().unwrap();
        let _shared_guard = crate::config::set_test_shared_dir(shared.path().to_path_buf());
        let _profile_guard = crate::config::set_test_profile_dir(profile.path().to_path_buf());

        let script_path = super::write_agent_state_script("plexi").unwrap();

        assert_eq!(
            script_path,
            shared
                .path()
                .join("hooks")
                .join(super::PLEXI_AGENT_STATE_SCRIPT),
            "hook script must live under the shared dir, not the channel profile dir"
        );
        assert!(script_path.is_file());
        assert!(!profile.path().join("hooks").exists());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&script_path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o755);
        }
    }

    #[cfg(unix)]
    #[test]
    fn agent_state_script_exits_with_tty_and_drains_pipe() {
        use std::io::Write;
        use std::os::fd::FromRawFd;
        use std::os::unix::process::CommandExt;
        use std::process::{Command, Stdio};
        use std::time::{Duration, Instant};

        // Keep the master open without sending EOF: cat would wait forever.
        let (_master, slave) = unsafe {
            let (mut master, mut slave) = (0, 0);
            assert_eq!(
                libc::openpty(
                    &mut master,
                    &mut slave,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                ),
                0,
                "openpty: {}",
                std::io::Error::last_os_error()
            );
            (
                std::fs::File::from_raw_fd(master),
                std::fs::File::from_raw_fd(slave),
            )
        };
        for (label, stdin) in [("TTY", Stdio::from(slave)), ("pipe", Stdio::piped())] {
            let mut child = Command::new("/bin/sh")
                .args(["-c", &super::agent_state_script("plexi")])
                .env_remove("PLEXI_SOCKET")
                .env_remove("PLEXI_PANE_ID")
                .stdin(stdin)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .process_group(0)
                .spawn()
                .unwrap();
            // More than a pipe buffer: exiting without draining must break this write.
            let writer = child.stdin.take().map(|mut pipe| {
                std::thread::spawn(move || pipe.write_all(&vec![b'x'; 1024 * 1024]))
            });
            let timeout = crate::testing::load_aware_timeout(Duration::from_secs(2));
            let started = Instant::now();
            let status = loop {
                if let Some(status) = child.try_wait().unwrap() {
                    break Some(status);
                }
                if started.elapsed() >= timeout {
                    // Kill the isolated shell AND its cat before reaping or joining.
                    assert_eq!(
                        unsafe { libc::kill(-(child.id() as i32), libc::SIGKILL) },
                        0
                    );
                    child.wait().unwrap();
                    break None;
                }
                std::thread::sleep(Duration::from_millis(10));
            };
            if let Some(writer) = writer {
                writer
                    .join()
                    .unwrap()
                    .expect("hook must drain the entire piped payload");
            }
            assert!(
                status.is_some_and(|status| status.success()),
                "{label} hook did not exit successfully within {timeout:?}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn startup_repairs_missing_shared_hook_without_replacing_existing_scripts() {
        use std::os::unix::fs::PermissionsExt;
        let shared = tempfile::tempdir().unwrap();
        let _guard = crate::config::set_test_shared_dir(shared.path().to_path_buf());
        let path = shared
            .path()
            .join("hooks")
            .join(super::PLEXI_AGENT_STATE_SCRIPT);
        super::ensure_agent_state_script().unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            super::agent_state_script("plexi")
        );
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o755
        );
        assert!(std::process::Command::new(&path)
            .stdin(std::process::Stdio::null())
            .env_remove("PLEXI_SOCKET")
            .env_remove("PLEXI_PANE_ID")
            .status()
            .unwrap()
            .success());
        std::fs::write(&path, "custom existing hook\n").unwrap();
        super::ensure_agent_state_script().unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "custom existing hook\n"
        );
        std::fs::remove_file(&path).unwrap();
        super::ensure_agent_state_script().unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            super::agent_state_script("plexi")
        );
    }

    #[test]
    fn shared_dir_is_channel_neutral() {
        let _channel = crate::config::set_test_channel("pr-999");
        assert!(crate::config::shared_dir().ends_with(".plexi"));
    }

    #[test]
    fn reinstall_rewrites_stale_channel_suffixed_script_paths() {
        let stale = "/home/u/.plexi-alpha/hooks/claude-code-agent-state.sh";
        let neutral = "/home/u/.plexi/hooks/claude-code-agent-state.sh";

        let mut settings = serde_json::json!({});
        super::register_agent_hooks(&mut settings, super::CLAUDE_CODE_HOOK_EVENTS, stale, None)
            .unwrap();
        super::register_agent_hooks(&mut settings, super::CLAUDE_CODE_HOOK_EVENTS, neutral, None)
            .unwrap();

        for event in super::CLAUDE_CODE_HOOK_EVENTS {
            let entries = settings["hooks"][*event].as_array().unwrap();
            assert_eq!(
                entries.len(),
                1,
                "{event} kept a stale entry alongside the new one"
            );
            assert_eq!(entries[0]["hooks"][0]["command"], neutral);
        }
    }

    #[test]
    fn reinstall_preserves_unrelated_hook_entries() {
        let mut settings = serde_json::json!({
            "hooks": {
                "PreToolUse": [{
                    "matcher": "",
                    "hooks": [{"type": "command", "command": "/usr/local/bin/my-own-hook"}]
                }]
            }
        });
        let neutral = "/home/u/.plexi/hooks/claude-code-agent-state.sh";

        super::register_agent_hooks(&mut settings, super::CLAUDE_CODE_HOOK_EVENTS, neutral, None)
            .unwrap();

        let entries = settings["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(
            entries[0]["hooks"][0]["command"],
            "/usr/local/bin/my-own-hook"
        );
        assert_eq!(entries[1]["hooks"][0]["command"], neutral);
    }

    #[test]
    fn json_hook_round_trip_preserves_user_config_and_removes_only_plexi_entries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let original = serde_json::json!({
            "model": "user-choice",
            "hooks": {"PreToolUse": [{
                "matcher": "*.rs",
                "hooks": [{"type": "command", "command": "/usr/local/bin/user-hook"}]
            }]}
        });
        fs::write(&path, serde_json::to_string_pretty(&original).unwrap()).unwrap();

        for _ in 0..2 {
            assert_eq!(
                super::install_json_agent_hooks(
                    &path,
                    super::CLAUDE_CODE_HOOK_EVENTS,
                    "/tmp/claude-code-agent-state.sh",
                    None,
                    "Claude Code"
                ),
                0
            );
        }
        let installed: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(installed["model"], "user-choice");
        assert_eq!(
            installed["hooks"]["PreToolUse"].as_array().unwrap().len(),
            2
        );
        assert_eq!(
            fs::read_to_string(path.with_extension("json.plexi-hook.bak")).unwrap(),
            serde_json::to_string_pretty(&original).unwrap()
        );

        assert_eq!(
            super::uninstall_json_hooks(&path, super::CLAUDE_CODE_HOOK_EVENTS, "Claude Code"),
            0
        );
        let removed: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(removed["model"], "user-choice");
        assert_eq!(removed["hooks"]["PreToolUse"].as_array().unwrap().len(), 1);
        assert_eq!(
            removed["hooks"]["PreToolUse"][0]["hooks"][0]["command"],
            "/usr/local/bin/user-hook"
        );
    }

    #[test]
    fn invalid_or_unknown_hook_config_is_refused_without_modification() {
        let dir = tempfile::tempdir().unwrap();
        let invalid = dir.path().join("invalid.json");
        fs::write(&invalid, "{ definitely not json").unwrap();
        assert_eq!(
            super::install_json_agent_hooks(
                &invalid,
                super::CODEX_HOOK_EVENTS,
                "plexi",
                None,
                "Codex"
            ),
            1
        );
        assert_eq!(
            fs::read_to_string(&invalid).unwrap(),
            "{ definitely not json"
        );

        let unexpected = dir.path().join("unexpected.json");
        let source = r#"{"hooks":{"SessionStart":"do not replace"}}"#;
        fs::write(&unexpected, source).unwrap();
        assert_eq!(
            super::install_json_agent_hooks(
                &unexpected,
                super::CODEX_HOOK_EVENTS,
                "plexi",
                None,
                "Codex"
            ),
            1
        );
        assert_eq!(fs::read_to_string(&unexpected).unwrap(), source);
    }

    #[test]
    fn codex_hook_round_trip_removes_only_plexi_entries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hooks.json");
        let original = serde_json::json!({
            "hooks": {"SessionStart": [{
                "matcher": "project-*",
                "hooks": [{"type": "command", "command": "/usr/local/bin/user-codex-hook"}]
            }]}
        });
        fs::write(&path, serde_json::to_string_pretty(&original).unwrap()).unwrap();

        assert_eq!(
            super::install_json_agent_hooks(
                &path,
                super::CODEX_HOOK_EVENTS,
                "PLEXI_AGENT_NAME=codex /tmp/claude-code-agent-state.sh",
                Some("Plexi agent state"),
                "Codex"
            ),
            0
        );
        assert_eq!(
            super::uninstall_json_hooks(&path, super::CODEX_HOOK_EVENTS, "Codex"),
            0
        );
        let removed: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(removed, original);
    }

    #[test]
    fn pi_extension_migration_backs_up_legacy_hook_and_preserves_unknown_files() {
        let dir = tempfile::tempdir().unwrap();
        let ext_path = dir.path().join("plexi-agent-state.ts");
        let bak_path = dir.path().join("plexi-agent-state.ts.plexi-hook.bak");

        // 1. Unknown user extension is preserved and never overwritten or removed.
        let custom_content = "// User-authored custom extension\nexport default function() {}\n";
        fs::write(&ext_path, custom_content).unwrap();
        assert_eq!(
            super::install_pi_extension_at(&ext_path),
            1
        );
        assert!(super::migrate_legacy_pi_extension_at(&ext_path).is_err());
        assert_eq!(fs::read_to_string(&ext_path).unwrap(), custom_content);
        assert!(!bak_path.exists());

        // 2. Legacy Plexi-generated extension is recognized, backed up to .plexi-hook.bak, and removed on migration.
        fs::remove_file(&ext_path).unwrap();
        assert_eq!(
            super::install_pi_extension_at(&ext_path),
            0
        );
        let generated = fs::read_to_string(&ext_path).unwrap();
        assert!(super::is_plexi_pi_extension(&generated));
        assert!(generated.contains("__PLEXI_PI_PACKAGE_ACTIVE__"));

        let migrated = super::migrate_legacy_pi_extension_at(&ext_path).unwrap();
        assert!(migrated);
        assert!(!ext_path.exists());
        assert!(bak_path.is_file());
        assert_eq!(fs::read_to_string(&bak_path).unwrap(), generated);
    }

    #[test]
    fn pi_worker_uses_explicit_model_scope_without_changing_selected_model() {
        let adapter = super::harness_adapter("pi").unwrap();
        let command = adapter.spawn_command("task", Some("openai/gpt-4o"), None);
        assert!(command.contains("--models '*'"), "{command}");
        assert!(command.contains("--model openai/gpt-4o"), "{command}");
        assert!(!command.contains(" -p"), "{command}");
    }

    #[test]
    fn supervised_worker_reports_nonzero_and_clean_exit_without_completion() {
        for (command, code) in [("exit 1", 1), ("true", 0), ("printf 'output\\n'; exit 7", 7)] {
            let wrapped = super::supervised_worker_command(command, "printf '%s\\n'", "script");
            let output = std::process::Command::new("/bin/sh").args(["-c", &wrapped]).output().unwrap();
            assert!(output.status.success());
            let text = String::from_utf8(output.stdout).unwrap();
            assert!(text.contains("HarnessExit"), "{text}");
            assert!(text.contains(&format!("harness exited with code {code} without agent done")), "{text}");
            let args: Vec<_> = text.lines().collect();
            assert!(
                args.windows(2).any(|pair| {
                    pair == ["--state", if code == 0 { "exited_unreported" } else { "failed" }]
                }),
                "{text}"
            );
        }
    }
    #[test]
    fn codex_worker_command_allows_socket_without_profile_write_access() {
        let cwd = tempfile::tempdir().unwrap();
        assert!(!cwd.path().join(".git").exists());
        let adapter = super::harness_adapter("codex").unwrap();
        let command = adapter.spawn_command("submit a ballot", None, None);
        // A shell function records the exact argv delivered in the non-git cwd.
        let probe = format!("codex() {{ printf '%s\\n' \"$@\"; }}; {command}");
        let output = std::process::Command::new("/bin/sh").args(["-c", &probe]).current_dir(cwd.path()).output().unwrap();
        assert!(output.status.success());
        assert_eq!(String::from_utf8(output.stdout).unwrap(), "--sandbox\nworkspace-write\n-c\nsandbox_workspace_write.network_access=true\nsubmit a ballot\n");
        assert_eq!(adapter.spawn_flags_summary(None, false, &[]), "--sandbox workspace-write -c sandbox_workspace_write.network_access=true");
        assert!(!command.contains("--dangerously"));
        assert!(!command.contains("writable_roots"));
        assert!(!command.contains("--add-dir"));
        assert!(!command.contains(" exec"));
    }
    #[test]
    fn multi_cli_harness_adapters_build_spawn_resume_and_capture_session_ids() {
        let claude = super::harness_adapter("claude").expect("claude adapter");
        assert_eq!(
            claude.resume_command_with_flags("sess-claude-1", &[]),
            "claude --resume sess-claude-1"
        );
        assert_eq!(
            claude.spawn_command("fix bug", Some("claude-sonnet-4-5"), None),
            "claude --model claude-sonnet-4-5 'fix bug'"
        );
        assert_eq!(
            claude.capture_session_id(&serde_json::json!({"session_id": "s-1"})),
            Some("s-1".to_string())
        );

        let codex = super::harness_adapter("codex").expect("codex adapter");
        assert_eq!(
            codex.resume_command_with_flags("thread-99", &[]),
            "codex resume thread-99"
        );
        assert_eq!(
            codex.spawn_command("run tests", None, None),
            "codex --sandbox workspace-write -c sandbox_workspace_write.network_access=true 'run tests'"
        );
        assert_eq!(
            codex.capture_session_id(&serde_json::json!({"thread_id": "t-99"})),
            Some("t-99".to_string())
        );

        let pi = super::harness_adapter("pi").expect("pi adapter");
        assert_eq!(
            pi.resume_command_with_flags("pi-sess-7", &[]),
            "pi --session pi-sess-7"
        );
        assert_eq!(
            pi.spawn_command("audit module", None, None),
            "pi --no-extensions --no-mcp --models '*' 'audit module'"
        );
        assert_eq!(
            pi.capture_session_id(&serde_json::json!({"sessionId": "pi-7"})),
            Some("pi-7".to_string())
        );

        let agy = super::harness_adapter("agy").expect("agy adapter");
        assert_eq!(
            agy.resume_command_with_flags("conv-123", &[]),
            "agy --resume conv-123"
        );
        assert_eq!(
            agy.spawn_command("implement stint", None, None),
            "agy --prompt-interactive 'implement stint'"
        );
        assert_eq!(
            agy.capture_session_id(&serde_json::json!({"conversation_id": "conv-123"})),
            Some("conv-123".to_string())
        );

        let script = super::harness_adapter("script").expect("script adapter");
        assert_eq!(
            script.spawn_command("echo hello", None, None),
            "/bin/sh -c 'echo hello'"
        );
    }

    #[test]
    fn build_agent_spawn_command_composes_persona_model_and_worker_contract_for_all_four_clis() {
        let schema_str =
            r#"{"type":"object","required":["answer"],"properties":{"answer":{"type":"string"}}}"#;
        let make_def = |cli: &str, model: Option<&str>| {
            let model_line = model
                .map(|m| format!("model = \"{m}\"\n"))
                .unwrap_or_default();
            let settings = format!(
                "[agent]\n\
                 id = \"researcher\"\n\
                 display_name = \"Researcher\"\n\
                 \n\
                 [harness]\n\
                 cli = \"{cli}\"\n\
                 {model_line}\
                 result_schema = '{schema_str}'\n"
            );
            crate::agent::AgentDefinition::parse("You are a fast codebase researcher.", &settings)
                .expect("valid definition")
        };

        // 1. pi: interactive (no -p), uses --no-extensions --no-mcp, --append-system-prompt with persona + contract + schema, and initial message.
        let pi_def = make_def("pi", Some("openrouter/~z-ai/glm-flash-latest"));
        let pi_adapter = super::harness_adapter("pi").unwrap();
        let pi_cmd =
            super::build_agent_spawn_command(&pi_def, pi_adapter, "Find where AgentRegistry loads")
                .unwrap();
        assert!(
            pi_cmd.starts_with(
                "pi --no-extensions --no-mcp --models '*' --model 'openrouter/~z-ai/glm-flash-latest' --append-system-prompt 'You are a fast codebase researcher.\n\n# Plexi worker contract"
            ),
            "unexpected pi command: {pi_cmd}"
        );
        assert!(!pi_cmd.contains(" -p"));
        assert!(pi_cmd.contains("--no-extensions"));
        assert!(pi_cmd.contains("--no-mcp"));
        assert!(pi_cmd.ends_with("'Find where AgentRegistry loads'"));
        assert!(pi_cmd.contains("# Plexi worker contract"));
        assert!(pi_cmd.contains("agent `researcher`"));
        assert!(pi_cmd.contains("plexi agent done --summary \"<one line>\" --result "));
        assert!(pi_cmd.contains("plexi agent ask \"<question>\""));
        assert!(pi_cmd.contains("\"required\":[\"answer\"]"));
        assert!(!pi_cmd.contains("# Persona"));
        let raw_contract = super::format_worker_contract(&pi_def);
        assert!(
            raw_contract.contains("plexi agent done --summary \"<one line>\" --result '<json>'")
        );

        // 2. claude: interactive (no -p), uses --append-system-prompt with persona + contract.
        let claude_def = make_def("claude", Some("claude-sonnet-4-5"));
        let claude_adapter = super::harness_adapter("claude").unwrap();
        let claude_cmd =
            super::build_agent_spawn_command(&claude_def, claude_adapter, "Audit module").unwrap();
        assert!(
            claude_cmd.starts_with(
                "claude --model claude-sonnet-4-5 --append-system-prompt 'You are a fast codebase researcher.\n\n# Plexi worker contract"
            ),
            "unexpected claude command: {claude_cmd}"
        );
        assert!(!claude_cmd.contains(" -p"));
        assert!(claude_cmd.contains("# Plexi worker contract"));
        assert!(!claude_cmd.contains("--append-system-prompt-file"));
        assert!(!claude_cmd.contains("# Persona"));
        assert!(claude_cmd.ends_with("'Audit module'"));

        // 3. codex: interactive (no exec); passes developer_instructions with # Persona + contract.
        let codex_def = make_def("codex", Some("gpt-5-codex"));
        let codex_adapter = super::harness_adapter("codex").unwrap();
        let codex_cmd =
            super::build_agent_spawn_command(&codex_def, codex_adapter, "Run suite").unwrap();
        assert!(
            codex_cmd.starts_with("codex --model gpt-5-codex --sandbox workspace-write -c sandbox_workspace_write.network_access=true -c "),
            "unexpected codex command: {codex_cmd}"
        );
        assert!(!codex_cmd.contains(" exec "));
        assert!(!codex_cmd.contains("--append-system-prompt"));
        assert!(!codex_cmd.contains("--system-prompt"));
        assert!(
            codex_cmd.contains(
                "developer_instructions=# Persona\nYou are a fast codebase researcher.\n\n# Plexi worker contract"
            ),
            "codex command must pass developer_instructions with # Persona and contract: {codex_cmd}"
        );
        assert!(codex_cmd.ends_with("'Run suite'"));

        // 4. agy: interactive (--prompt-interactive); prepends `# Persona\n<AGENT.md>\n\n` to task.
        let agy_def = make_def("agy", Some("gemini-2.5-pro"));
        let agy_adapter = super::harness_adapter("agy").unwrap();
        let agy_cmd =
            super::build_agent_spawn_command(&agy_def, agy_adapter, "Implement feature").unwrap();
        assert!(
            agy_cmd.starts_with("agy --model gemini-2.5-pro --prompt-interactive "),
            "unexpected agy command: {agy_cmd}"
        );
        assert!(!agy_cmd.contains("--append-system-prompt"));
        assert!(!agy_cmd.contains("--system-prompt"));
        assert!(
            agy_cmd.contains(
                "# Persona\nYou are a fast codebase researcher.\n\nImplement feature\n\n# Plexi worker contract"
            ),
            "agy command must inline # Persona before task: {agy_cmd}"
        );

        // 5. Refuses Assistant personas without [harness].
        let assistant_only = crate::agent::AgentDefinition::parse(
            "You are an assistant.",
            "[agent]\nid = \"helper\"\ndisplay_name = \"Helper\"\ndefault_tier = \"low\"\n\n[permissions]\ndefault_posture = \"ask\"\n",
        )
        .unwrap();
        let err = super::build_agent_spawn_command(&assistant_only, pi_adapter, "hi").unwrap_err();
        assert!(err.contains("[harness]"), "{err}");
    }

    #[test]
    fn pi_spawn_command_includes_no_extensions_no_mcp_and_optional_explicit_extensions() {
        let pi_adapter = super::harness_adapter("pi").unwrap();

        let default_def = crate::agent::AgentDefinition::parse(
            "Secret persona text that must not appear in flags log.",
            "[agent]\nid = \"worker\"\ndisplay_name = \"Worker\"\n\n[harness]\ncli = \"pi\"\nmodel = \"sonnet\"\n",
        )
        .unwrap();
        let default_cmd =
            super::build_agent_spawn_command(&default_def, pi_adapter, "do task").unwrap();
        assert!(
            default_cmd.starts_with(
                "pi --no-extensions --no-mcp --models '*' --model sonnet --append-system-prompt 'Secret persona text that must not appear in flags log.\n\n# Plexi worker contract"
            ),
            "unexpected default pi command: {default_cmd}"
        );
        assert!(!default_cmd.contains(" -p"));
        let default_flags = pi_adapter.spawn_flags_summary(
            Some("sonnet"),
            true,
            &default_def.harness.as_ref().unwrap().extensions,
        );
        assert_eq!(
            default_flags,
            "--no-extensions --no-mcp --models '*' --model sonnet --append-system-prompt <AGENT.md>"
        );
        assert!(!default_flags.contains("Secret persona text"));

        let ext_def = crate::agent::AgentDefinition::parse(
            "Secret persona text.",
            "[agent]\nid = \"worker\"\ndisplay_name = \"Worker\"\n\n[harness]\ncli = \"pi\"\nextensions = [\"/tmp/my ext.ts\", \"builtin:plan-mode\"]\n",
        )
        .unwrap();
        let ext_cmd = super::build_agent_spawn_command(&ext_def, pi_adapter, "do task").unwrap();
        assert!(
            ext_cmd.starts_with(
                "pi --no-extensions --no-mcp --models '*' -e '/tmp/my ext.ts' -e builtin:plan-mode --append-system-prompt 'Secret persona text.\n\n# Plexi worker contract"
            ),
            "unexpected explicit-extension pi command: {ext_cmd}"
        );
    }

    #[test]
    fn agent_create_writes_files_round_trips_and_validates_errors() {
        let profile = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        fs::create_dir_all(ws.path().join(".git")).unwrap();
        let subdir = ws.path().join("crates").join("sub");
        fs::create_dir_all(&subdir).unwrap();

        let opts = super::AgentCreateOptions {
            name: "researcher",
            cli: "pi",
            model: Some("openrouter/~z-ai/glm-flash-latest"),
            fallback: Some("claude"),
            description: Some("Fast codebase researcher"),
            persona: Some("You are a researcher. Read files and answer concisely."),
            persona_file: None,
            result_schema: Some(
                r#"{"type":"object","required":["answer"],"properties":{"answer":{"type":"string"}}}"#,
            ),
            ephemeral: true,
            display_name: None,
            global: false,
            cwd: Some(&subdir),
            flags: &[],
        };
        let created =
            super::agent_create_in(&opts, profile.path()).expect("create workspace agent");
        assert_eq!(
            created,
            ws.path().join(".plexi").join("agents").join("researcher")
        );
        assert!(created.join("AGENT.md").is_file());
        assert!(created.join("settings.toml").is_file());

        // Round-trips through AgentDefinition::load_dir.
        let loaded =
            crate::agent::AgentDefinition::load_dir(&created, crate::agent::AgentSource::Workspace)
                .expect("round-trip load_dir");
        assert_eq!(loaded.id, "researcher");
        assert_eq!(loaded.display_name, "researcher");
        assert_eq!(loaded.description, "Fast codebase researcher");
        let h = loaded.harness.expect("harness present");
        assert_eq!(h.cli, "pi");
        assert_eq!(
            h.model.as_deref(),
            Some("openrouter/~z-ai/glm-flash-latest")
        );
        assert_eq!(h.fallback.as_deref(), Some("claude"));
        assert!(h.ephemeral);
        assert!(h.result_schema.is_some());

        // Refuses to overwrite existing agent dir and suggests editing files.
        let overwrite_err = super::agent_create_in(&opts, profile.path()).unwrap_err();
        assert!(
            overwrite_err.contains(&created.display().to_string())
                && overwrite_err.contains("AGENT.md")
                && overwrite_err.contains("settings.toml"),
            "overwrite error must name path and suggest editing files: {overwrite_err}"
        );

        // Refuses invalid slug, invalid CLI, invalid schema, and both/neither persona flags.
        let bad_slug = super::AgentCreateOptions {
            name: "Bad_Slug!",
            ..opts
        };
        assert!(super::agent_create_in(&bad_slug, profile.path()).is_err());

        let bad_cli = super::AgentCreateOptions {
            name: "other",
            cli: "vim",
            ..opts
        };
        assert!(super::agent_create_in(&bad_cli, profile.path()).is_err());

        let bad_schema = super::AgentCreateOptions {
            name: "other",
            result_schema: Some("{not json"),
            ..opts
        };
        assert!(super::agent_create_in(&bad_schema, profile.path()).is_err());

        let both_personas = super::AgentCreateOptions {
            name: "other",
            persona: Some("inline"),
            persona_file: Some("file.md"),
            ..opts
        };
        assert!(super::agent_create_in(&both_personas, profile.path()).is_err());

        // Outside a workspace without --global fails with an error instead of falling back to global.
        let outside = tempfile::tempdir().unwrap();
        let no_ws_opts = super::AgentCreateOptions {
            name: "orphan",
            cwd: Some(outside.path()),
            global: false,
            ..opts
        };
        let no_ws_err = super::agent_create_in(&no_ws_opts, profile.path()).unwrap_err();
        assert!(
            no_ws_err.contains("--global"),
            "missing workspace error must mention --global: {no_ws_err}"
        );
        assert!(!profile.path().join("agents").join("orphan").exists());
    }

    #[test]
    fn agent_list_and_show_report_tiers_shadowing_and_spawn_command() {
        let profile = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        fs::create_dir_all(ws.path().join(".plexi")).unwrap();

        // Create global researcher + global reviewer.
        super::agent_create_in(
            &super::AgentCreateOptions {
                name: "researcher",
                cli: "claude",
                model: Some("claude-sonnet-4-5"),
                fallback: None,
                description: Some("Global researcher"),
                persona: Some("Global persona."),
                persona_file: None,
                result_schema: None,
                ephemeral: false,
                display_name: Some("Global Researcher"),
                global: true,
                cwd: Some(ws.path()),
                flags: &[],
            },
            profile.path(),
        )
        .unwrap();
        super::agent_create_in(
            &super::AgentCreateOptions {
                name: "reviewer",
                cli: "codex",
                model: None,
                fallback: None,
                description: Some("Global reviewer"),
                persona: Some("Review diffs."),
                persona_file: None,
                result_schema: None,
                ephemeral: false,
                display_name: None,
                global: true,
                cwd: Some(ws.path()),
                flags: &[],
            },
            profile.path(),
        )
        .unwrap();

        // Create workspace researcher that shadows global researcher.
        super::agent_create_in(
            &super::AgentCreateOptions {
                name: "researcher",
                cli: "pi",
                model: Some("openrouter/~z-ai/glm-flash-latest"),
                fallback: None,
                description: Some("Workspace researcher"),
                persona: Some("Workspace persona."),
                persona_file: None,
                result_schema: None,
                ephemeral: true,
                display_name: Some("Workspace Researcher"),
                global: false,
                cwd: Some(ws.path()),
                flags: &[],
            },
            profile.path(),
        )
        .unwrap();

        let list_text = super::agent_list_in(profile.path(), Some(ws.path()), false).unwrap();
        assert!(
            !list_text.contains("Plexi Assistant"),
            "the paused Assistant persona must leave the agent registry: {list_text}"
        );
        assert!(list_text.contains("default") && list_text.contains("built-in"));
        assert!(list_text.contains("researcher") && list_text.contains("workspace"));
        assert!(list_text.contains("pi (openrouter/~z-ai/glm-flash-latest)"));
        assert!(list_text.contains("[shadows global]"));
        assert!(list_text.contains("reviewer") && list_text.contains("global"));

        let list_json: serde_json::Value = serde_json::from_str(
            &super::agent_list_in(profile.path(), Some(ws.path()), true).unwrap(),
        )
        .unwrap();
        let arr = list_json.as_array().unwrap();
        // The two authored agents plus the built-in `default` (stint 1035).
        assert_eq!(arr.len(), 3);
        let builtin_default = arr.iter().find(|v| v["name"] == "default").unwrap();
        assert_eq!(builtin_default["tier"], "built-in");
        assert_eq!(builtin_default["cli"], "pi");
        let r = arr.iter().find(|v| v["name"] == "researcher").unwrap();
        assert_eq!(r["tier"], "workspace");
        assert_eq!(r["shadows_global"], true);
        assert_eq!(r["cli"], "pi");

        let show_json: serde_json::Value = serde_json::from_str(
            &super::agent_show_in("researcher", profile.path(), Some(ws.path()), true).unwrap(),
        )
        .unwrap();
        assert_eq!(show_json["tier"], "workspace");
        assert_eq!(show_json["shadows_global"], true);
        let spawn_cmd = show_json["spawn_command"].as_str().unwrap();
        assert!(spawn_cmd.contains("pi --no-extensions --no-mcp --models '*' --model 'openrouter/~z-ai/glm-flash-latest' --append-system-prompt 'Workspace persona."));
        assert!(!spawn_cmd.contains(" -p"));
        assert!(spawn_cmd.contains("<TASK>"));
    }

    /// Stint 1036: `agent list` names the directories it will not load, each
    /// with its path and the file that is missing or legacy, instead of
    /// returning a shorter list with no explanation.
    #[test]
    fn agent_list_reports_unrecognised_agent_directories() {
        let _channel = crate::config::set_test_channel("alpha");
        let profile = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        let legacy = ws.path().join(".plexi").join("agents").join("lead-120cadfb");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(legacy.join("head.json"), "{\"id\":\"lead-120cadfb\"}").unwrap();
        let pane = ws.path().join(".plexi").join("agents").join("writer");
        fs::create_dir_all(&pane).unwrap();
        fs::write(pane.join("AGENT.md"), "legacy-writer-marker\n").unwrap();

        let out = super::agent_list_in(profile.path(), Some(ws.path()), false).unwrap();
        assert!(out.contains(&legacy.display().to_string()), "{out}");
        assert!(out.contains("head.json"), "{out}");
        assert!(out.contains(&pane.display().to_string()), "{out}");
        assert!(out.contains("settings.toml"), "{out}");
        assert!(
            legacy.join("agent.json").is_file(),
            "the list migrates the legacy card rather than only reporting it"
        );
    }

    /// Stint 1035: `agent show default` used to refuse with the Assistant-persona
    /// error; it must print a concrete Pi spawn command and hand the user the
    /// workspace definition it seeds.
    #[test]
    fn agent_show_default_prints_a_pi_spawn_command_and_seeds_the_workspace_copy() {
        let profile = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        fs::create_dir_all(ws.path().join(".git")).unwrap();

        let json: serde_json::Value = serde_json::from_str(
            &super::agent_show_in("default", profile.path(), Some(ws.path()), true).unwrap(),
        )
        .unwrap();
        assert_eq!(json["harness"]["cli"], "pi");
        let spawn = json["spawn_command"]
            .as_str()
            .expect("a concrete spawn command, not the Assistant-persona refusal");
        assert!(
            spawn.starts_with("pi "),
            "unexpected default spawn command: {spawn}"
        );

        let seeded = ws.path().join(".plexi").join("agents").join("default");
        assert!(seeded.join("settings.toml").is_file());
        assert!(seeded.join("persona.md").is_file());

        // The human surface agrees with the JSON one.
        let text = super::agent_show_in("default", profile.path(), Some(ws.path()), false).unwrap();
        assert!(text.contains("CLI:           pi"), "{text}");
        assert!(text.contains("Spawn command: pi "), "{text}");
        assert!(!text.contains("Plexi Assistant"), "{text}");
    }

    /// Stint 1035: `spawn`/`start default` composes a Pi command and seeds the
    /// workspace definition, including in a fresh checkout that has a `.git` but
    /// no `.plexi/agents/` at all.
    #[test]
    fn agent_start_default_seeds_the_workspace_copy_and_composes_a_pi_command() {
        let fake_home = tempfile::tempdir().unwrap();
        let profile = fake_home.path().join(".plexi-alpha");
        fs::create_dir_all(&profile).unwrap();
        let repo = fake_home.path().join("src").join("repo");
        fs::create_dir_all(repo.join(".git")).unwrap();
        let stops = vec![fake_home.path().to_path_buf()];

        let payload = super::resolve_agent_start_in_stopping_at(
            "default",
            &profile,
            &repo,
            None,
            None,
            &stops,
        )
        .expect("default resolves in a fresh checkout");
        assert_eq!(payload["role"], "default");
        assert_eq!(payload["harness"], "pi");
        let cmd = payload["command"].as_str().unwrap();
        assert!(cmd.starts_with("pi "), "unexpected default start command: {cmd}");
        assert!(!cmd.contains(" -p"));
        assert_eq!(payload["cwd"], repo.to_string_lossy().as_ref());

        let seeded = repo.join(".plexi").join("agents").join("default");
        assert!(
            seeded.join("settings.toml").is_file(),
            "spawn/start must seed {}",
            seeded.display()
        );
        assert!(seeded.join("persona.md").is_file());
    }

    #[test]
    fn agent_show_redacts_secret_flags_in_every_inspection_field() {
        let profile = tempfile::tempdir().unwrap();
        let secret = "Authorization: Bearer abc123SECRET";
        let flags = ["--header".to_string(), secret.to_string()];
        super::agent_create_in(
            &super::AgentCreateOptions {
                name: "redacted-show",
                cli: "claude",
                model: None,
                fallback: None,
                description: None,
                persona: Some("Keep secrets private."),
                persona_file: None,
                result_schema: None,
                ephemeral: false,
                display_name: None,
                global: true,
                cwd: None,
                flags: &flags,
            },
            profile.path(),
        )
        .unwrap();

        let text = super::agent_show_in("redacted-show", profile.path(), None, false).unwrap();
        assert!(!text.contains(secret), "text leaked secret: {text}");
        assert!(text.contains("[REDACTED]"), "text did not redact: {text}");

        let json = super::agent_show_in("redacted-show", profile.path(), None, true).unwrap();
        assert!(!json.contains(secret), "json leaked secret: {json}");
        assert!(json.contains("[REDACTED]"), "json did not redact: {json}");
    }

    #[test]
    fn workspace_walk_stops_never_match_home_for_lookup_or_create() {
        let fake_home = tempfile::tempdir().unwrap();
        fs::create_dir_all(fake_home.path().join(".plexi").join("agents")).unwrap();
        fs::create_dir_all(fake_home.path().join(".git")).unwrap();
        let child = fake_home.path().join("Downloads");
        fs::create_dir_all(&child).unwrap();
        let stops = vec![fake_home.path().to_path_buf()];

        assert_eq!(
            crate::app::registry::resolve_agent_lookup_workspace_root_stopping_at(&child, &stops),
            None
        );
        assert_eq!(
            crate::app::registry::resolve_agent_create_workspace_root_stopping_at(&child, &stops),
            None
        );

        let project = fake_home.path().join("project");
        let nested = project.join("src");
        fs::create_dir_all(project.join(".plexi").join("agents")).unwrap();
        fs::create_dir_all(&nested).unwrap();
        assert_eq!(
            crate::app::registry::resolve_agent_lookup_workspace_root_stopping_at(&nested, &stops),
            Some(project.clone())
        );
        assert_eq!(
            crate::app::registry::resolve_agent_create_workspace_root_stopping_at(&nested, &stops),
            Some(project)
        );
    }

    #[test]
    fn pi_restore_command_falls_back_to_bare_launch_until_session_file_exists_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let sessions_dir = dir.path().join(".pi").join("agent").join("sessions");
        let proj_dir = sessions_dir.join("--Users-test-project--");
        fs::create_dir_all(&proj_dir).unwrap();
        let _pi_guard = super::set_test_pi_sessions_dir(sessions_dir.clone());

        let pi = super::AgentHarness::Pi;
        let claude = super::AgentHarness::Claude;

        // 1. `session_id: None` dispatches bare binary launch for any harness.
        assert_eq!(pi.restore_command_with_flags(None, &[]), "pi");
        assert_eq!(pi.restore_command_with_flags(Some("   "), &[]), "pi");
        assert_eq!(claude.restore_command_with_flags(None, &[]), "claude");
        assert_eq!(
            claude.restore_command_with_flags(Some("sess-claude-1"), &[]),
            "claude --resume sess-claude-1"
        );

        // 2. Unpersisted Pi session UUID (no matching file on disk) falls back to bare `pi`.
        assert_eq!(
            pi.restore_command_with_flags(Some("019dcdae-unpersisted"), &[]),
            "pi"
        );

        // 3. Once `*_019dcdae-persisted.jsonl` exists under `~/.pi/agent/sessions/`,
        //    `restore_command` resumes via `pi --session 019dcdae-persisted`.
        let persisted_jsonl = proj_dir.join("2026-10-17T05-00-00-000Z_019dcdae-persisted.jsonl");
        fs::write(&persisted_jsonl, "{\"role\":\"user\"}\n").unwrap();
        assert_eq!(
            pi.restore_command_with_flags(Some("019dcdae-persisted"), &[]),
            "pi --session 019dcdae-persisted"
        );

        // 4. Direct session file path that exists on disk also resumes via `pi --session <path>`.
        let direct_path = persisted_jsonl.to_string_lossy().to_string();
        assert_eq!(
            pi.restore_command_with_flags(Some(&direct_path), &[]),
            format!("pi --session {direct_path}")
        );
    }

    #[test]
    fn build_agent_start_command_is_interactive_with_persona_and_worker_contract() {
        let make_def = |cli: &str, model: Option<&str>| {
            let model_line = model
                .map(|m| format!("model = \"{m}\"\n"))
                .unwrap_or_default();
            crate::agent::AgentDefinition::parse(
                "You are a fast codebase researcher.\n",
                &format!(
                    "[agent]\nid = \"researcher\"\ndisplay_name = \"Researcher\"\n\n[harness]\ncli = \"{cli}\"\n{model_line}"
                ),
            )
            .expect("parse agent definition")
        };

        // 1. pi: interactive (no -p), applies persona + worker contract via --append-system-prompt.
        let pi_def = make_def("pi", Some("openrouter/~z-ai/glm-flash-latest"));
        let pi_adapter = super::harness_adapter("pi").unwrap();
        let pi_cmd = super::build_agent_start_command(&pi_def, pi_adapter, None).unwrap();
        assert!(
            pi_cmd.starts_with(
                "pi --no-extensions --no-mcp --models '*' --model 'openrouter/~z-ai/glm-flash-latest' --append-system-prompt "
            ),
            "unexpected pi start command: {pi_cmd}"
        );
        assert!(!pi_cmd.contains(" -p"), "interactive pi start must not pass -p: {pi_cmd}");
        assert!(pi_cmd.contains("You are a fast codebase researcher."));
        assert!(pi_cmd.contains("# Plexi worker contract"));

        let pi_with_msg =
            super::build_agent_start_command(&pi_def, pi_adapter, Some("Inspect auth")).unwrap();
        assert!(
            pi_with_msg.ends_with("'Inspect auth'"),
            "pi start with initial message must append quoted message: {pi_with_msg}"
        );
        assert!(!pi_with_msg.contains(" -p"));

        // 2. claude: interactive (no -p), applies persona + worker contract via --append-system-prompt.
        let claude_def = make_def("claude", Some("claude-sonnet-4-5"));
        let claude_adapter = super::harness_adapter("claude").unwrap();
        let claude_cmd =
            super::build_agent_start_command(&claude_def, claude_adapter, Some("Hello")).unwrap();
        assert!(
            claude_cmd.starts_with("claude --model claude-sonnet-4-5 --append-system-prompt "),
            "unexpected claude start command: {claude_cmd}"
        );
        assert!(!claude_cmd.contains(" -p"));
        assert!(claude_cmd.contains("You are a fast codebase researcher."));
        assert!(claude_cmd.contains("# Plexi worker contract"));
        assert!(claude_cmd.ends_with("Hello"));

        // 3. codex: interactive (no `exec`), applies persona + worker contract via developer_instructions.
        let codex_def = make_def("codex", Some("gpt-5-codex"));
        let codex_adapter = super::harness_adapter("codex").unwrap();
        let codex_cmd =
            super::build_agent_start_command(&codex_def, codex_adapter, Some("Run checks"))
                .unwrap();
        assert!(
            codex_cmd.starts_with(
                "codex --model gpt-5-codex --sandbox workspace-write -c sandbox_workspace_write.network_access=true -c "
            ),
            "unexpected codex start command: {codex_cmd}"
        );
        assert!(!codex_cmd.contains(" exec "), "interactive codex start must not pass exec: {codex_cmd}");
        assert!(codex_cmd.contains("developer_instructions=# Persona\nYou are a fast codebase researcher.\n\n# Plexi worker contract"));
        assert!(codex_cmd.ends_with("'Run checks'"));

        // 4. agy: interactive (`--prompt-interactive` instead of `--prompt`).
        let agy_def = make_def("agy", Some("gemini-2.5-pro"));
        let agy_adapter = super::harness_adapter("agy").unwrap();
        let agy_cmd =
            super::build_agent_start_command(&agy_def, agy_adapter, Some("Summarize")).unwrap();
        assert!(
            agy_cmd.starts_with("agy --model gemini-2.5-pro --prompt-interactive "),
            "unexpected agy start command: {agy_cmd}"
        );
        assert!(agy_cmd.contains("# Persona\nYou are a fast codebase researcher.\n\nSummarize\n\n# Plexi worker contract"));
    }

    #[test]
    fn agent_start_resolves_project_then_user_stopping_at_home_and_lists_available_on_unknown() {
        let fake_home = tempfile::tempdir().unwrap();
        let profile = fake_home.path().join(".plexi-alpha");
        fs::create_dir_all(&profile).unwrap();

        // Put a decoy `.plexi/agents/decoy` directly under $HOME — must be ignored by walk stop.
        let home_decoy = fake_home.path().join(".plexi").join("agents").join("decoy");
        fs::create_dir_all(&home_decoy).unwrap();
        fs::write(home_decoy.join("AGENT.md"), "Home decoy persona.\n").unwrap();
        fs::write(
            home_decoy.join("settings.toml"),
            "[agent]\nid = \"decoy\"\ndisplay_name = \"Decoy\"\n[harness]\ncli = \"pi\"\n",
        )
        .unwrap();

        // Seed global user agent `researcher` and `reviewer` in `~/.plexi-alpha/agents/`.
        super::agent_create_in(
            &super::AgentCreateOptions {
                name: "researcher",
                cli: "pi",
                model: Some("openrouter/~z-ai/glm-flash-latest"),
                fallback: None,
                description: Some("Global researcher"),
                persona: Some("You are the global researcher."),
                persona_file: None,
                result_schema: None,
                ephemeral: false,
                display_name: None,
                global: true,
                cwd: None,
                flags: &[],
            },
            &profile,
        )
        .unwrap();
        super::agent_create_in(
            &super::AgentCreateOptions {
                name: "reviewer",
                cli: "claude",
                model: None,
                fallback: None,
                description: Some("Global reviewer"),
                persona: Some("You are the global reviewer."),
                persona_file: None,
                result_schema: None,
                ephemeral: false,
                display_name: None,
                global: true,
                cwd: None,
                flags: &[],
            },
            &profile,
        )
        .unwrap();

        let stops = vec![fake_home.path().to_path_buf()];

        // Arbitrary project dir `~/temp/proj` with no `.plexi/agents`: resolves `researcher` from
        // user profile dir and sets pane `cwd` to `~/temp/proj`.
        let proj = fake_home.path().join("temp").join("proj");
        fs::create_dir_all(&proj).unwrap();
        let payload = super::resolve_agent_start_in_stopping_at(
            "researcher",
            &profile,
            &proj,
            Some("First turn"),
            None,
            &stops,
        )
        .expect("resolve global researcher from ~/temp/proj");
        assert_eq!(payload["type"], "agent_spawn");
        assert_eq!(payload["role"], "researcher");
        assert_eq!(payload["harness"], "pi");
        assert_eq!(payload["ephemeral"], false);
        assert_eq!(payload["cwd"], proj.to_string_lossy().as_ref());
        let cmd = payload["command"].as_str().unwrap();
        assert!(cmd.contains("You are the global researcher."));
        assert!(cmd.contains("# Plexi worker contract"));
        assert!(!cmd.contains(" -p"));

        // Inside a nested subdirectory of a project with `.plexi/agents/researcher`, walks up to
        // project root, shadows global, and sets pane `cwd` to the project root.
        let nested = proj.join("crates").join("sub");
        fs::create_dir_all(&nested).unwrap();
        fs::create_dir_all(proj.join(".plexi")).unwrap();
        super::agent_create_in(
            &super::AgentCreateOptions {
                name: "researcher",
                cli: "claude",
                model: Some("claude-sonnet-4-5"),
                fallback: None,
                description: Some("Project researcher"),
                persona: Some("You are the project researcher."),
                persona_file: None,
                result_schema: None,
                ephemeral: false,
                display_name: None,
                global: false,
                cwd: Some(&proj),
                flags: &[],
            },
            &profile,
        )
        .unwrap();
        let ws_payload = super::resolve_agent_start_in_stopping_at(
            "researcher",
            &profile,
            &nested,
            None,
            None,
            &stops,
        )
        .expect("resolve workspace researcher from nested dir");
        assert_eq!(ws_payload["role"], "researcher");
        assert_eq!(ws_payload["harness"], "claude");
        assert_eq!(ws_payload["cwd"], proj.to_string_lossy().as_ref());
        assert!(ws_payload["command"]
            .as_str()
            .unwrap()
            .contains("You are the project researcher."));

        // Unknown agent name fails and lists available agents; decoy at $HOME was never loaded.
        let err = super::resolve_agent_start_in_stopping_at(
            "nonexistent",
            &profile,
            &proj,
            None,
            None,
            &stops,
        )
        .unwrap_err();
        assert!(
            err.contains("available agents: default, researcher, reviewer"),
            "unknown agent error must list available agents (including the launchable built-in `default`, stint 1035): {err}"
        );
        assert!(
            super::resolve_agent_start_in_stopping_at("decoy", &profile, &proj, None, None, &stops)
                .is_err(),
            "$HOME/.plexi/agents must not be treated as a project workspace"
        );
    }

    #[test]
    fn test_custom_harness_flags_rejection_of_shell_injections() {
        let metacharacters = [
            ";", "&&", "||", "|", "&", "`", "$(", ">", "<", "\n", "\r", "\0",
        ];
        for meta in metacharacters {
            let flag = format!("--verbose{meta}");
            let err = super::validate_custom_harness_flags(
                super::AgentHarness::Claude.adapter(),
                &[flag.clone()],
                None,
            )
            .unwrap_err();
            assert!(
                err.contains("forbidden shell operator") || err.contains("forbidden"),
                "expected shell injection rejection for {flag:?}, got: {err}"
            );

            // Also test injection in value
            let err_val = super::validate_custom_harness_flags(
                super::AgentHarness::Claude.adapter(),
                &["--permission-mode".to_string(), format!("plan{meta}")],
                None,
            )
            .unwrap_err();
            assert!(
                err_val.contains("forbidden shell operator") || err_val.contains("forbidden"),
                "expected shell injection rejection in value for {meta:?}, got: {err_val}"
            );
        }
    }

    #[test]
    fn test_custom_harness_flags_preserves_literal_quotes_and_spaces() {
        let flags = vec![
            "--theme".to_string(),
            "Solarized Dark".to_string(),
            "--thinking".to_string(),
        ];
        let validated = super::validate_custom_harness_flags(
            super::AgentHarness::Pi.adapter(),
            &flags,
            None,
        )
        .expect("valid flags with spaces should pass");
        assert_eq!(validated, flags);

        let pi_adapter = super::AgentHarness::Pi.adapter();
        let cmd = pi_adapter.start_command_with_flags_and_extensions(
            None,
            None,
            None,
            &flags,
            &[],
        );
        // Literal spaces must be preserved with single quotes
        assert!(cmd.contains("'Solarized Dark'"), "cmd was: {cmd}");
        assert!(cmd.contains("--thinking"), "cmd was: {cmd}");

        // Test quotes inside values
        let codex_flags = vec![
            "-c".to_string(),
            "setting='quoted value'".to_string(),
        ];
        let validated_codex = super::validate_custom_harness_flags(
            super::AgentHarness::Codex.adapter(),
            &codex_flags,
            None,
        )
        .expect("quotes in value should pass");
        assert_eq!(validated_codex, codex_flags);

        let codex_adapter = super::AgentHarness::Codex.adapter();
        let codex_cmd = codex_adapter.start_command_with_flags_and_extensions(
            None,
            None,
            None,
            &codex_flags,
            &[],
        );
        // Single quotes must be safely escaped via '\''
        assert!(
            codex_cmd.contains("'setting='\\''quoted value'\\'''"),
            "codex_cmd was: {codex_cmd}"
        );
    }

    #[test]
    fn test_custom_harness_flags_security_overrides_rejected() {
        let security_overrides = [
            "--dangerously-skip-permissions",
            "--no-sandbox",
            "--sandbox=none",
            "--full-auto",
            "-y",
            "--yes",
        ];
        for harness in [
            super::AgentHarness::Claude,
            super::AgentHarness::Pi,
            super::AgentHarness::Codex,
            super::AgentHarness::Agy,
        ] {
            for ov in security_overrides {
                let err = super::validate_custom_harness_flags(
                    harness.adapter(),
                    &[ov.to_string()],
                    None,
                )
                .unwrap_err();
                assert!(
                    err.contains("security override") || err.contains("does not support flag"),
                    "expected rejection of {ov:?} for {harness:?}, got: {err}"
                );
            }
        }
    }

    #[test]
    fn test_custom_harness_flags_reject_security_override_values() {
        for flags in [
            vec!["--permission-mode".to_string(), "bypassPermissions".to_string()],
            vec!["--permission-mode=dontAsk".to_string()],
            vec!["--sandbox".to_string(), "danger-full-access".to_string()],
            vec!["--ask-for-approval=never".to_string()],
            vec!["--config=approval_policy=never".to_string()],
            vec!["-c".to_string(), "ask_for_approval=on-failure".to_string()],
        ] {
            let adapter = if flags[0].starts_with("--permission") {
                super::AgentHarness::Claude.adapter()
            } else {
                super::AgentHarness::Codex.adapter()
            };
            let error = super::validate_custom_harness_flags(adapter, &flags, None).unwrap_err();
            assert!(
                error.contains("reserved security override") || error.contains("reserved for Plexi sandbox authority"),
                "expected authority rejection for {flags:?}, got: {error}"
            );
        }
    }

    #[test]
    fn test_custom_harness_flags_reserved_lifecycle_overrides_rejected() {
        let lifecycle_overrides = [
            "--append-system-prompt",
            "--settings",
            "--resume",
            "-p",
            "--print",
            "exec",
            "--prompt",
            "--prompt-interactive",
        ];
        for harness in [
            super::AgentHarness::Claude,
            super::AgentHarness::Pi,
            super::AgentHarness::Codex,
            super::AgentHarness::Agy,
        ] {
            for ov in lifecycle_overrides {
                let err = super::validate_custom_harness_flags(
                    harness.adapter(),
                    &[ov.to_string()],
                    None,
                )
                .unwrap_err();
                assert!(
                    err.contains("reserved") || err.contains("does not support flag"),
                    "expected rejection of {ov:?} for {harness:?}, got: {err}"
                );
            }
        }
    }

    #[test]
    fn test_custom_harness_flags_model_conflicts_rejected() {
        // When model is configured in settings.toml, passing --model in flags must fail
        let err = super::validate_custom_harness_flags(
            super::AgentHarness::Claude.adapter(),
            &["--model".to_string(), "claude-3-opus".to_string()],
            Some("claude-3-5-sonnet"),
        )
        .unwrap_err();
        assert!(err.contains("conflicts with configured model"), "{err}");

        // When model is NOT configured, passing --model in flags is allowed
        let ok = super::validate_custom_harness_flags(
            super::AgentHarness::Claude.adapter(),
            &["--model".to_string(), "claude-3-5-sonnet".to_string()],
            None,
        );
        assert!(ok.is_ok(), "model flag without config conflict should be ok");
    }

    #[test]
    fn test_custom_harness_flags_adapter_unsupported_options_rejected() {
        // Script adapter supports no flags
        let err_script = super::validate_custom_harness_flags(
            super::AgentHarness::Script.adapter(),
            &["--verbose".to_string()],
            None,
        )
        .unwrap_err();
        assert!(err_script.contains("does not support flag '--verbose'"), "{err_script}");

        // Claude with -e (Pi extension flag)
        let err_claude = super::validate_custom_harness_flags(
            super::AgentHarness::Claude.adapter(),
            &["-e".to_string(), "ext.js".to_string()],
            None,
        )
        .unwrap_err();
        assert!(err_claude.contains("does not support flag '-e'"), "{err_claude}");

        // Pi with --effort (Codex flag)
        let err_pi = super::validate_custom_harness_flags(
            super::AgentHarness::Pi.adapter(),
            &["--effort".to_string(), "high".to_string()],
            None,
        )
        .unwrap_err();
        assert!(err_pi.contains("does not support flag '--effort'"), "{err_pi}");

        // Codex with --theme (Pi flag)
        let err_codex = super::validate_custom_harness_flags(
            super::AgentHarness::Codex.adapter(),
            &["--theme".to_string(), "dark".to_string()],
            None,
        )
        .unwrap_err();
        assert!(err_codex.contains("does not support flag '--theme'"), "{err_codex}");

        // Agy with --effort (Claude/Codex flag)
        let err_agy = super::validate_custom_harness_flags(
            super::AgentHarness::Agy.adapter(),
            &["--effort".to_string(), "high".to_string()],
            None,
        )
        .unwrap_err();
        assert!(err_agy.contains("does not support flag '--effort'"), "{err_agy}");
    }

    #[test]
    fn test_custom_harness_flags_missing_arg_and_positional_rejected() {
        // Option expecting value without value
        let err_missing = super::validate_custom_harness_flags(
            super::AgentHarness::Claude.adapter(),
            &["--mcp-config".to_string()],
            None,
        )
        .unwrap_err();
        assert!(err_missing.contains("missing required value"), "{err_missing}");

        // Unexpected positional argument
        let err_pos = super::validate_custom_harness_flags(
            super::AgentHarness::Claude.adapter(),
            &["unexpected_positional".to_string()],
            None,
        )
        .unwrap_err();
        assert!(err_pos.contains("unexpected positional argument"), "{err_pos}");
    }

    #[test]
    fn test_custom_harness_flags_parity_across_harnesses() {
        let claude_adapter = super::AgentHarness::Claude.adapter();
        let claude_flags = vec!["--verbose".to_string()];
        let claude_start = claude_adapter.start_command_with_flags_and_extensions(
            None,
            None,
            None,
            &claude_flags,
            &[],
        );
        assert!(claude_start.contains("claude --verbose"), "got: {claude_start}");

        let claude_spawn = claude_adapter.spawn_command_with_flags_and_extensions(
            "fix bug",
            None,
            None,
            &claude_flags,
            &[],
        );
        assert!(claude_spawn.contains("claude --verbose"), "got: {claude_spawn}");

        let claude_restore = claude_adapter.restore_command_with_flags(
            Some("sess-123"),
            &claude_flags,
        );
        assert_eq!(claude_restore, "claude --verbose --resume sess-123");

        let claude_resume = claude_adapter.resume_command_with_flags(
            "sess-123",
            &claude_flags,
        );
        assert_eq!(claude_resume, "claude --verbose --resume sess-123");

        let claude_bare = claude_adapter.restore_command_with_flags(
            None,
            &claude_flags,
        );
        assert_eq!(claude_bare, "claude --verbose");

        // Pi
        let pi_adapter = super::AgentHarness::Pi.adapter();
        let pi_flags = vec!["--verbose".to_string()];
        let pi_start = pi_adapter.start_command_with_flags_and_extensions(
            None,
            None,
            None,
            &pi_flags,
            &[],
        );
        assert!(pi_start.starts_with("pi ") && pi_start.contains("--verbose"), "got: {pi_start}");

        // Codex
        let codex_adapter = super::AgentHarness::Codex.adapter();
        let codex_flags = vec!["--verbose".to_string()];
        let codex_start = codex_adapter.start_command_with_flags_and_extensions(
            None,
            None,
            None,
            &codex_flags,
            &[],
        );
        assert!(codex_start.contains("codex --verbose"), "got: {codex_start}");

        // Agy
        let agy_adapter = super::AgentHarness::Agy.adapter();
        let agy_flags = vec!["--verbose".to_string()];
        let agy_start = agy_adapter.start_command_with_flags_and_extensions(
            None,
            None,
            None,
            &agy_flags,
            &[],
        );
        assert!(agy_start.contains("agy --verbose"), "got: {agy_start}");

        // Script
        let script_adapter = super::AgentHarness::Script.adapter();
        let script_start = script_adapter.start_command_with_flags_and_extensions(
            Some("echo hi"),
            None,
            None,
            &[],
            &[],
        );
        assert!(script_start.contains("/bin/sh -c 'echo hi'"), "got: {script_start}");
    }

    #[test]
    fn test_redact_harness_flags_redacts_secrets() {
        let flags = vec![
            "--mcp-config".to_string(),
            "/path/to/config.json".to_string(),
            "--header".to_string(),
            "Authorization: Bearer secret_token_value".to_string(),
        ];
        let redacted = super::redact_harness_flags(&flags);
        assert_eq!(redacted[0], "--mcp-config");
        assert_eq!(redacted[1], "/path/to/config.json");
        assert_eq!(redacted[2], "--header");
        assert!(
            redacted[3].contains("[REDACTED]"),
            "secret must be redacted: {:?}",
            redacted[3]
        );

        let inline = vec![
            "--config=openai_api_key=ZZSECRET2ZZ".to_string(),
            "-c=openai_api_key=ZZSECRET3ZZ".to_string(),
            "--header=Authorization: Bearer abc123SECRET".to_string(),
        ];
        let redacted = super::redact_harness_flags(&inline);
        assert_eq!(redacted, vec!["--config=[REDACTED]", "-c=[REDACTED]", "--header=[REDACTED]"]);
    }

    #[test]
    fn test_agent_flags_cli_inspect_and_json() {
        // Inspecting an adapter returns 0
        assert_eq!(super::agent_flags_cli(Some("claude"), None, false), 0);
        assert_eq!(super::agent_flags_cli(Some("claude"), None, true), 0);
        assert_eq!(super::agent_flags_cli(Some("pi"), None, true), 0);
        assert_eq!(super::agent_flags_cli(Some("codex"), None, true), 0);
        assert_eq!(super::agent_flags_cli(Some("agy"), None, true), 0);
        assert_eq!(super::agent_flags_cli(Some("script"), None, true), 0);
        assert_eq!(super::agent_flags_cli(None, None, true), 0);
        assert_eq!(super::agent_flags_cli(None, None, false), 0);
        // Invalid adapter returns non-zero
        assert_ne!(super::agent_flags_cli(Some("invalid-adapter-xyz"), None, false), 0);
    }
}

/// Read the same host projection painted by the Agents pane.
pub fn agent_fleet_cli(json: bool, filter: crate::protocol::FleetFilter) -> i32 {
    let content = match super::request(
        serde_json::json!({"type":"agent_fleet", "filter":filter}),
        "plexi-agent-fleet",
        "agent fleet",
    ) {
        Ok(content) => content,
        Err(code) => return code,
    };
    let view: crate::agent::fleet::AgentsView = match serde_json::from_str(&content) {
        Ok(view) => view,
        Err(e) => {
            eprintln!("error: could not read agent fleet: {e}");
            return 1;
        }
    };
    if json {
        println!("{content}");
        return 0;
    }
    if view.tasks.is_empty() && view.older.is_empty() {
        println!("No matching agents.");
        return 0;
    }
    for task in view.tasks {
        println!("{} — {}", task.title, task.counts());
        for row in task.rows {
            let detail = row
                .blocked_on
                .as_ref()
                .map(|b| format!(" · {}", b.label()))
                .or_else(|| row.reason.as_ref().map(|r| format!(" · {r}")))
                .unwrap_or_default();
            let next = row
                .next_hint
                .as_ref()
                .map(|h| format!(" · next: {h}"))
                .unwrap_or_default();
            println!(
                "  {} ({})  {}  {}{}{}  #{}",
                row.role,
                row.harness,
                row.display_state.label(),
                row.age,
                detail,
                next,
                row.pane_id
            );
        }
    }
    if !view.older.is_empty() {
        println!(
            "older ({} stale or previous-session failure{}) — dismiss with `plexi agent fleet dismiss --all-failed`",
            view.older.len(),
            if view.older.len() == 1 { "" } else { "s" }
        );
        for row in view.older {
            let reason = row
                .reason
                .as_ref()
                .map(|r| format!(" · {r}"))
                .unwrap_or_default();
            let next = row
                .next_hint
                .as_ref()
                .map(|h| format!(" · next: {h}"))
                .unwrap_or_default();
            println!(
                "  {} · {} ({})  {}  {}{}{}  #{}",
                row.role,
                row.task_title,
                row.harness,
                row.display_state.label(),
                row.age,
                reason,
                next,
                row.pane_id
            );
        }
    }
    0
}

pub fn agent_fleet_dismiss_cli(
    target: Option<&str>,
    all_failed: bool,
    json: bool,
) -> i32 {
    let parsed_target = match target {
        Some(raw) => {
            let trimmed = raw.trim().strip_prefix('#').unwrap_or(raw.trim());
            match trimmed.parse::<u64>() {
                Ok(id) => Some(id),
                Err(_) => {
                    eprintln!("error: invalid pane id '{raw}' (expected a numeric id like 215 or #215)");
                    return 1;
                }
            }
        }
        None => None,
    };
    let content = match super::request(
        serde_json::json!({
            "type": "agent_fleet_dismiss",
            "target": parsed_target,
            "all_failed": all_failed,
        }),
        "plexi-agent-fleet-dismiss",
        "agent fleet dismiss",
    ) {
        Ok(content) => content,
        Err(code) => return code,
    };
    if json {
        println!("{content}");
        return 0;
    }
    let val: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: invalid response: {e}");
            return 1;
        }
    };
    let dismissed: Vec<u64> = val["dismissed"]
        .as_array()
        .map(|arr| arr.iter().filter_map(|v| v.as_u64()).collect())
        .unwrap_or_default();
    if dismissed.is_empty() {
        if all_failed {
            println!("No failed agent runs to dismiss.");
            0
        } else {
            eprintln!(
                "error: no agent run found for #{}",
                parsed_target.unwrap_or(0)
            );
            1
        }
    } else {
        let list = dismissed
            .iter()
            .map(|id| format!("#{id}"))
            .collect::<Vec<_>>()
            .join(", ");
        println!("Dismissed {list} (retained in `plexi agent history`).");
        0
    }
}

pub fn agent_next_cli(json: bool) -> i32 {
    let content = match super::request(
        serde_json::json!({"type":"agent_next"}),
        "plexi-agent-next",
        "agent next",
    ) {
        Ok(content) => content,
        Err(code) => return code,
    };
    if json {
        println!("{content}");
    } else {
        match serde_json::from_str::<serde_json::Value>(&content) {
            Ok(value) => match value["pane_id"].as_u64() {
                Some(id) => {
                    let role = value["role"].as_str().unwrap_or("");
                    let harness = value["harness"].as_str().unwrap_or("");
                    let task_title = value["task_title"].as_str().unwrap_or("");
                    if !role.is_empty() && !task_title.is_empty() {
                        println!("{role} ({harness}) · {task_title}  #{id}");
                    } else if !role.is_empty() {
                        println!("{role} ({harness})  #{id}");
                    } else {
                        println!("{id}");
                    }
                }
                None => println!("No agents need attention."),
            },
            Err(error) => {
                eprintln!("error: invalid agent next response: {error}");
                return 1;
            }
        }
    }
    0
}

/// Agent tool operations retain the socket until the host answers the lifecycle wait.
pub fn agent_tool_cli(mut payload: serde_json::Value) -> i32 {
    // Build and stamp the protocol type before sending. Command-specific fields
    // must not leak into other deny_unknown_fields request variants.
    payload["response_file"] = "".into();
    let mut request: crate::protocol::AppRequest = match serde_json::from_value(payload) {
        Ok(request) => request,
        Err(error) => {
            eprintln!("error: invalid agent tool request: {error}");
            return 1;
        }
    };
    match &mut request {
        crate::protocol::AppRequest::AgentCall { parent_pane_id, call_credential, .. }
        | crate::protocol::AppRequest::AgentMessage { parent_pane_id, call_credential, .. } => {
            *parent_pane_id = std::env::var("PLEXI_PANE_ID").ok().and_then(|s| s.parse().ok());
            *call_credential = std::env::var("PLEXI_CALL_CREDENTIAL").ok();
        }
        _ => {
            eprintln!("error: expected an agent call or message request");
            return 1;
        }
    }
    if let crate::protocol::AppRequest::AgentCall { cwd, .. } = &mut request {
        *cwd = std::env::current_dir().ok().map(|p| p.to_string_lossy().into_owned());
    }
    let timeout = if matches!(request,
        crate::protocol::AppRequest::AgentCall { asynchronous: false, handoff: false, .. }
        | crate::protocol::AppRequest::AgentMessage { wait: true, .. }
    ) {
        std::time::Duration::from_secs(86405)
    } else {
        crate::rpc::DEFAULT_TIMEOUT
    };
    let payload = match serde_json::to_value(request) {
        Ok(payload) => payload,
        Err(error) => {
            eprintln!("error: serializing agent tool request: {error}");
            return 1;
        }
    };
    let reply = match super::request_with(payload, "agent-tool", "agent tool", timeout) {
        Ok(reply) => reply,
        Err(code) => return code,
    };
    println!("{reply}");
    if super::reply_error(&reply).is_some() { 1 } else { 0 }
}

fn is_safe_result_dir_key(key: &str) -> bool {
    let trimmed = key.trim();
    !trimmed.is_empty()
        && trimmed != "."
        && trimmed != ".."
        && !trimmed.contains('/')
        && !trimmed.contains('\\')
}

pub fn format_agent_done_markdown(
    agent_role: &str,
    summary: &str,
    committed: &serde_json::Value,
) -> String {
    let role = if agent_role.trim().is_empty() {
        "agent"
    } else {
        agent_role.trim()
    };
    let mut md = String::new();
    if summary.trim().is_empty() {
        md.push_str(&format!("# {role} done\n\n"));
    } else {
        md.push_str(&format!("# {role} done: {}\n\n", summary.trim()));
        md.push_str(&format!("**Summary:** {}\n\n", summary.trim()));
    }
    if let Some(text) = committed
        .get("markdown")
        .or_else(|| committed.get("report"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
    {
        md.push_str(text.trim());
        md.push_str("\n\n");
    }
    let pretty = serde_json::to_string_pretty(committed).unwrap_or_else(|_| committed.to_string());
    md.push_str("```json\n");
    md.push_str(&pretty);
    md.push_str("\n```\n");
    md
}

#[allow(clippy::too_many_arguments)]
pub fn persist_agent_done_result(
    pane_id: u64,
    agent_role: &str,
    harness: Option<&str>,
    session_id: Option<&str>,
    summary: &str,
    committed: &serde_json::Value,
    workspace_roots: &[PathBuf],
    profile_dir: &Path,
) -> Result<(PathBuf, PathBuf, String, serde_json::Value), String> {
    let markdown = format_agent_done_markdown(agent_role, summary, committed);
    let primary_base = workspace_roots
        .first()
        .map(|ws| ws.join(".plexi").join("results"))
        .unwrap_or_else(|| profile_dir.join("results"));
    let primary_dir = primary_base.join(pane_id.to_string());
    let primary_md = primary_dir.join("RESULT.md");
    let primary_json = primary_dir.join("result.json");

    let mut map = match committed {
        serde_json::Value::Object(obj) => obj.clone(),
        _ => serde_json::Map::new(),
    };
    map.entry("ok".to_string())
        .or_insert_with(|| serde_json::Value::Bool(true));
    map.entry("status".to_string())
        .or_insert_with(|| serde_json::Value::String("completed".to_string()));
    map.insert("pane_id".to_string(), serde_json::json!(pane_id));
    map.entry("agent".to_string())
        .or_insert_with(|| serde_json::Value::String(agent_role.to_string()));
    if let Some(h) = harness {
        map.entry("harness".to_string())
            .or_insert_with(|| serde_json::Value::String(h.to_string()));
    }
    if let Some(sid) = session_id {
        map.entry("session_id".to_string())
            .or_insert_with(|| serde_json::Value::String(sid.to_string()));
    }
    map.entry("summary".to_string())
        .or_insert_with(|| serde_json::Value::String(summary.to_string()));
    map.insert("result".to_string(), committed.clone());
    map.insert(
        "markdown".to_string(),
        serde_json::Value::String(markdown.clone()),
    );
    map.insert(
        "result_path".to_string(),
        serde_json::Value::String(primary_md.to_string_lossy().into_owned()),
    );
    map.insert(
        "json_path".to_string(),
        serde_json::Value::String(primary_json.to_string_lossy().into_owned()),
    );

    let json_val = serde_json::Value::Object(map);
    let json_bytes =
        serde_json::to_string_pretty(&json_val).map_err(|e| format!("serialize result: {e}"))?;

    let mut bases: Vec<PathBuf> = workspace_roots
        .iter()
        .map(|ws| ws.join(".plexi").join("results"))
        .collect();
    let profile_results = profile_dir.join("results");
    if !bases.contains(&profile_results) {
        bases.push(profile_results);
    }

    let mut keys = vec![pane_id.to_string()];
    if let Some(sid) = session_id.filter(|s| is_safe_result_dir_key(s)) {
        if !keys.iter().any(|k| k == sid) {
            keys.push(sid.to_string());
        }
    }
    if is_safe_result_dir_key(agent_role) && !keys.iter().any(|k| k == agent_role) {
        keys.push(agent_role.to_string());
    }

    for base in &bases {
        std::fs::create_dir_all(base)
            .map_err(|e| format!("create {}: {e}", base.display()))?;
        let _ = std::fs::write(base.join("latest.json"), &json_bytes);
        let _ = std::fs::write(base.join("RESULT.md"), &markdown);
        for key in &keys {
            let dir = base.join(key);
            std::fs::create_dir_all(&dir)
                .map_err(|e| format!("create {}: {e}", dir.display()))?;
            let md_path = dir.join("RESULT.md");
            let js_path = dir.join("result.json");
            std::fs::write(&md_path, &markdown)
                .map_err(|e| format!("write {}: {e}", md_path.display()))?;
            std::fs::write(&js_path, &json_bytes)
                .map_err(|e| format!("write {}: {e}", js_path.display()))?;
        }
    }

    log::info!(
        "agent_done: persisted result pane_id={pane_id} agent={agent_role} path={}",
        primary_md.display()
    );
    Ok((primary_md, primary_json, markdown, json_val))
}

pub fn read_persisted_agent_result_in(
    target: Option<&str>,
    cwd: Option<&Path>,
    profile_dir: &Path,
) -> Result<(PathBuf, String, serde_json::Value), String> {
    let normalized_target = target
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.strip_prefix('#').unwrap_or(s));

    let mut bases = Vec::new();
    if let Some(dir) = cwd {
        if let Some(ws) = crate::app::registry::resolve_agent_lookup_workspace_root(dir) {
            let b = ws.join(".plexi").join("results");
            if !bases.contains(&b) {
                bases.push(b);
            }
        }
        let b = dir.join(".plexi").join("results");
        if !bases.contains(&b) {
            bases.push(b);
        }
    }
    let profile_base = profile_dir.join("results");
    if !bases.contains(&profile_base) {
        bases.push(profile_base);
    }

    for base in &bases {
        if let Some(key) = normalized_target {
            if is_safe_result_dir_key(key) {
                let dir = base.join(key);
                let js_path = dir.join("result.json");
                let md_path = dir.join("RESULT.md");
                if js_path.is_file() {
                    let raw = std::fs::read_to_string(&js_path)
                        .map_err(|e| format!("read {}: {e}", js_path.display()))?;
                    let val: serde_json::Value = serde_json::from_str(&raw)
                        .map_err(|e| format!("parse {}: {e}", js_path.display()))?;
                    let md = std::fs::read_to_string(&md_path).unwrap_or_else(|_| {
                        val.get("markdown")
                            .and_then(|v| v.as_str())
                            .unwrap_or(&raw)
                            .to_string()
                    });
                    let reported_md = val
                        .get("result_path")
                        .and_then(|v| v.as_str())
                        .map(PathBuf::from)
                        .unwrap_or(md_path);
                    return Ok((reported_md, md, val));
                }
            }
            let latest_js = base.join("latest.json");
            if latest_js.is_file() {
                if let Ok(raw) = std::fs::read_to_string(&latest_js) {
                    if let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw) {
                        let matches_key = val
                            .get("pane_id")
                            .and_then(|v| v.as_u64())
                            .is_some_and(|id| id.to_string() == key)
                            || val
                                .get("agent")
                                .and_then(|v| v.as_str())
                                .is_some_and(|a| a == key)
                            || val
                                .get("session_id")
                                .and_then(|v| v.as_str())
                                .is_some_and(|s| s == key);
                        if matches_key {
                            let md_path = val
                                .get("result_path")
                                .and_then(|v| v.as_str())
                                .map(PathBuf::from)
                                .unwrap_or_else(|| base.join("RESULT.md"));
                            let md = std::fs::read_to_string(&md_path).unwrap_or_else(|_| {
                                val.get("markdown")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or(&raw)
                                    .to_string()
                            });
                            return Ok((md_path, md, val));
                        }
                    }
                }
            }
        } else {
            let latest_js = base.join("latest.json");
            if latest_js.is_file() {
                let raw = std::fs::read_to_string(&latest_js)
                    .map_err(|e| format!("read {}: {e}", latest_js.display()))?;
                let val: serde_json::Value = serde_json::from_str(&raw)
                    .map_err(|e| format!("parse {}: {e}", latest_js.display()))?;
                let md_path = val
                    .get("result_path")
                    .and_then(|v| v.as_str())
                    .map(PathBuf::from)
                    .unwrap_or_else(|| base.join("RESULT.md"));
                let md = std::fs::read_to_string(&md_path).unwrap_or_else(|_| {
                    val.get("markdown")
                        .and_then(|v| v.as_str())
                        .unwrap_or(&raw)
                        .to_string()
                });
                return Ok((md_path, md, val));
            }
        }
    }

    // Fallback to agent_history.jsonl when available.
    for record in crate::workspace::load_agent_history().into_iter().rev() {
        let Some(ref committed) = record.result else {
            continue;
        };
        let matches = match normalized_target {
            Some(key) => {
                record.pane_id.to_string() == key
                    || record.run_id == key
                    || record.config_name.as_deref() == Some(key)
                    || record.pane_name.as_deref() == Some(key)
                    || record.session_id.as_deref() == Some(key)
            }
            None => true,
        };
        if matches {
            let role = record
                .config_name
                .as_deref()
                .or(record.pane_name.as_deref())
                .unwrap_or(&record.harness);
            let summary = record.summary.as_deref().unwrap_or_default();
            let md = format_agent_done_markdown(role, summary, committed);
            let md_path = record
                .cwd
                .join(".plexi")
                .join("results")
                .join(record.pane_id.to_string())
                .join("RESULT.md");
            let mut map = match committed {
                serde_json::Value::Object(obj) => obj.clone(),
                _ => serde_json::Map::new(),
            };
            map.entry("ok".to_string())
                .or_insert_with(|| serde_json::Value::Bool(true));
            map.entry("status".to_string())
                .or_insert_with(|| serde_json::Value::String("completed".to_string()));
            map.insert("pane_id".to_string(), serde_json::json!(record.pane_id));
            map.entry("agent".to_string())
                .or_insert_with(|| serde_json::Value::String(role.to_string()));
            map.entry("summary".to_string())
                .or_insert_with(|| serde_json::Value::String(summary.to_string()));
            map.insert("result".to_string(), committed.clone());
            map.insert("markdown".to_string(), serde_json::Value::String(md.clone()));
            map.insert(
                "result_path".to_string(),
                serde_json::Value::String(md_path.to_string_lossy().into_owned()),
            );
            return Ok((md_path, md, serde_json::Value::Object(map)));
        }
    }

    match normalized_target {
        Some(key) => Err(format!("no completed agent result found for '{key}'")),
        None => Err("no completed agent result found".to_string()),
    }
}

/// `plexi agent result [<pane>] [--json]`
pub fn agent_result_cli(target: Option<&str>, json: bool) -> i32 {
    log::info!("agent_result:cli: target={target:?} json={json}");
    let cwd = std::env::current_dir().ok();
    let profile_dir = crate::config::config_dir();
    match read_persisted_agent_result_in(target, cwd.as_deref(), &profile_dir) {
        Ok((md_path, md, val)) => {
            if let Some(pane_id) = val.get("pane_id").and_then(|v| v.as_u64()) {
                let response_file = crate::rpc::response_file("agent-result-ack", "json");
                super::try_send_to_socket_quiet(serde_json::json!({
                    "type": "pane_result_read",
                    "pane_id": pane_id,
                    "wait": false,
                    "timeout_secs": 0.0,
                    "response_file": response_file,
                }));
            }
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&val).unwrap_or_else(|_| val.to_string())
                );
            } else {
                print!("{md}");
                if !md.ends_with('\n') {
                    println!();
                }
                println!("RESULT.md: {}", md_path.display());
            }
            0
        }
        Err(err) => {
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "ok": false,
                        "error": err,
                    })
                );
            } else {
                eprintln!("error: {err}");
            }
            1
        }
    }
}

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
            eprintln!(
                "error: no .plexi/ workspace found at or above {}.\n  \
                 Run `plexi workspace init` first.",
                cwd.display()
            );
            Err(1)
        }
    }
}

/// Multi-CLI harness identifiers supported natively by Plexi.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentHarness {
    Claude,
    Codex,
    Pi,
    Agy,
}

impl AgentHarness {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "claude" | "claude-code" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            "pi" => Some(Self::Pi),
            "agy" | "antigravity" => Some(Self::Agy),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Pi => "pi",
            Self::Agy => "agy",
        }
    }

    pub fn adapter(self) -> &'static HarnessAdapter {
        HARNESS_ADAPTERS
            .iter()
            .find(|a| a.harness == self)
            .expect("all AgentHarness variants exist in HARNESS_ADAPTERS")
    }

    pub fn resume_command(self, session_id: &str) -> String {
        self.adapter().resume_command(session_id)
    }

    pub fn restore_command(self, session_id: Option<&str>) -> String {
        self.adapter().restore_command(session_id)
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

/// First-class adapter mapping a CLI agent harness to its spawn, session-ID capture,
/// and resume command conventions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HarnessAdapter {
    pub harness: AgentHarness,
    pub binary: &'static str,
    pub task_flag: &'static str,
    pub resume_subcommand_or_flag: &'static str,
}

pub const HARNESS_ADAPTERS: [HarnessAdapter; 4] = [
    HarnessAdapter {
        harness: AgentHarness::Claude,
        binary: "claude",
        task_flag: "-p",
        resume_subcommand_or_flag: "--resume",
    },
    HarnessAdapter {
        harness: AgentHarness::Codex,
        binary: "codex",
        task_flag: "exec",
        resume_subcommand_or_flag: "resume",
    },
    HarnessAdapter {
        harness: AgentHarness::Pi,
        binary: "pi",
        task_flag: "-p",
        resume_subcommand_or_flag: "--session",
    },
    HarnessAdapter {
        harness: AgentHarness::Agy,
        binary: "agy",
        task_flag: "--prompt",
        resume_subcommand_or_flag: "--resume",
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

impl HarnessAdapter {
    /// Build the exact CLI command to resume a captured session ID.
    pub fn resume_command(&self, session_id: &str) -> String {
        let sid = shell_quote_arg(session_id.trim());
        format!("{} {} {}", self.binary, self.resume_subcommand_or_flag, sid)
    }

    /// Build the CLI command to restore an agent pane on workspace startup or standby wake.
    /// Falls back to a bare harness binary launch when `session_id` is absent or when a
    /// `pi` session ID has no persisted session file on disk.
    pub fn restore_command(&self, session_id: Option<&str>) -> String {
        let dir = pi_sessions_dir();
        self.restore_command_in(session_id, dir.as_deref())
    }

    pub fn restore_command_in(
        &self,
        session_id: Option<&str>,
        pi_sessions_root: Option<&Path>,
    ) -> String {
        let Some(sid) = session_id.and_then(non_empty_string) else {
            return self.binary.to_string();
        };
        if self.harness == AgentHarness::Pi && !pi_session_exists_in(sid, pi_sessions_root) {
            log::warn!(
                "agent_restore: Pi session '{}' has no persisted session file on disk; falling back to bare '{}' launch",
                sid,
                self.binary
            );
            return self.binary.to_string();
        }
        self.resume_command(sid)
    }

    /// Build the non-interactive / task-seeded CLI command to spawn a worker for `task`.
    ///
    /// `pi` always passes `--no-extensions --no-mcp` so headless workers never load user
    /// extensions or MCP servers, plus any explicit `-e <path>` entries.
    /// `pi` and `claude` receive persona text via `--append-system-prompt`.
    /// `codex` and `agy` have no system-prompt flag; callers (`build_agent_spawn_command`)
    /// inline `# Persona` into `task` before calling this method.
    pub fn spawn_command(
        &self,
        task: &str,
        model: Option<&str>,
        append_system_prompt: Option<&str>,
    ) -> String {
        self.spawn_command_with_extensions(task, model, append_system_prompt, &[])
    }

    pub fn spawn_command_with_extensions(
        &self,
        task: &str,
        model: Option<&str>,
        append_system_prompt: Option<&str>,
        extensions: &[String],
    ) -> String {
        let mut parts = vec![self.binary.to_string()];
        if self.harness == AgentHarness::Pi {
            parts.push("--no-extensions".to_string());
            parts.push("--no-mcp".to_string());
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
        if let Some(persona) = append_system_prompt.and_then(non_empty_string) {
            if matches!(self.harness, AgentHarness::Pi | AgentHarness::Claude) {
                parts.push("--append-system-prompt".to_string());
                parts.push(shell_quote_arg(persona));
            }
        }
        parts.push(self.task_flag.to_string());
        // Codex workers can edit their workspace and reach the host socket.
        // Never grant a profile writable root: it contains permission records.
        if self.harness == AgentHarness::Codex { parts.extend(["--skip-git-repo-check", "--sandbox", "workspace-write", "-c", "sandbox_workspace_write.network_access=true"].map(str::to_string)); }
        parts.push(shell_quote_arg(task));
        parts.join(" ")
    }

    /// Summarize the composed CLI flags for logging without including persona or task prose.
    pub fn spawn_flags_summary(
        &self,
        model: Option<&str>,
        has_append_system_prompt: bool,
        extensions: &[String],
    ) -> String {
        let mut flags = Vec::new();
        if self.harness == AgentHarness::Pi {
            flags.push("--no-extensions".to_string());
            flags.push("--no-mcp".to_string());
            for ext in extensions {
                if let Some(e) = non_empty_string(ext) {
                    flags.push("-e".to_string());
                    flags.push(shell_quote_arg(e));
                }
            }
        }
        if let Some(m) = model.and_then(non_empty_string) {
            flags.push("--model".to_string());
            flags.push(shell_quote_arg(m));
        }
        if has_append_system_prompt
            && matches!(self.harness, AgentHarness::Pi | AgentHarness::Claude)
        {
            flags.push("--append-system-prompt".to_string());
            flags.push("<AGENT.md>".to_string());
        }
        flags.push(self.task_flag.to_string());
        if self.harness == AgentHarness::Codex { flags.extend(["--skip-git-repo-check", "--sandbox", "workspace-write", "-c", "sandbox_workspace_write.network_access=true"].map(str::to_string)); }
        flags.join(" ")
    }

    /// Extract a session ID from a hook JSON payload or structured telemetry value.
    pub fn capture_session_id(&self, payload: &serde_json::Value) -> Option<String> {
        let candidates = match self.harness {
            AgentHarness::Claude => &["session_id", "sessionId"][..],
            AgentHarness::Codex => &["session_id", "thread_id", "conversation_id"][..],
            AgentHarness::Pi => &["session_id", "sessionId", "sessionFile"][..],
            AgentHarness::Agy => &["session_id", "conversation_id", "conversationId"][..],
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
            "unknown harness cli '{}' (expected pi, claude, codex, or agy)",
            harness_cfg.cli
        )
    })?;
    if super::binary_in_path(primary.binary) {
        return Ok(primary);
    }
    if let Some(ref fb_name) = harness_cfg.fallback {
        let fallback = harness_adapter(fb_name).ok_or_else(|| {
            format!(
                "unknown fallback '{}' (expected pi, claude, codex, or agy)",
                fb_name
            )
        })?;
        if super::binary_in_path(fallback.binary) {
            return Ok(fallback);
        }
    }
    Ok(primary)
}

/// Build the `# Plexi worker contract` section appended to every spawned worker's task.
pub fn format_worker_contract(definition: &crate::agent::AgentDefinition) -> String {
    let mut contract = format!(
        "# Plexi worker contract\n\
         You are agent `{}` running in a Plexi pane.\n\
         You have one non-interactive turn. Do not ask for confirmation.\n\
         Always finish by running: plexi agent done --summary \"<one line>\" --result '<json>'.\n\
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

/// Pure function building the exact shell command `plexi agent spawn` runs for
/// `(definition, adapter, task)`.
///
/// - Refuses agents without a `[harness]` table.
/// - `pi` passes `--no-extensions --no-mcp` (plus any explicit `-e <path>` from `[harness].extensions`).
/// - `pi` and `claude` pass `--append-system-prompt <AGENT.md text>`.
/// - `codex` and `agy` have no system-prompt flag, so `# Persona\n<AGENT.md text>\n\n`
///   is prepended to the task text.
/// - Always appends `# Plexi worker contract` (including the result schema verbatim when set).
pub fn build_agent_spawn_command(
    definition: &crate::agent::AgentDefinition,
    adapter: &HarnessAdapter,
    task: &str,
) -> Result<String, String> {
    let harness_cfg = definition.harness.as_ref().ok_or_else(|| {
        format!(
            "agent '{}' has no [harness] table in settings.toml (it is an Assistant persona and cannot be spawned as a CLI worker)",
            definition.id
        )
    })?;
    let persona = definition.prompt.trim();
    let contract = format_worker_contract(definition);
    let (append_prompt, full_task) = match adapter.harness {
        AgentHarness::Pi | AgentHarness::Claude => (Some(persona), format!("{task}\n\n{contract}")),
        AgentHarness::Codex | AgentHarness::Agy => (
            None,
            format!("# Persona\n{persona}\n\n{task}\n\n{contract}"),
        ),
    };
    let flags_summary = adapter.spawn_flags_summary(
        harness_cfg.model.as_deref(),
        append_prompt.and_then(non_empty_string).is_some(),
        &harness_cfg.extensions,
    );
    log::info!(
        "agent_spawn: role={} harness={} flags=\"{}\"",
        definition.id,
        adapter.harness.as_str(),
        flags_summary
    );
    if harness_cfg.extensions.is_empty() {
        Ok(adapter.spawn_command(&full_task, harness_cfg.model.as_deref(), append_prompt))
    } else {
        Ok(adapter.spawn_command_with_extensions(
            &full_task,
            harness_cfg.model.as_deref(),
            append_prompt,
            &harness_cfg.extensions,
        ))
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
            "invalid agent name 'default': 'default' is reserved for the built-in assistant"
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

    if let Some(def) = registry
        .active(trimmed)
        .filter(|d| d.source != crate::agent::AgentSource::BuiltIn)
    {
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
    Err(format!(
        "agent '{trimmed}' not found (searched: {})",
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
}

pub fn agent_create_in(
    opts: &AgentCreateOptions<'_>,
    profile_dir: &Path,
) -> Result<PathBuf, String> {
    validate_agent_slug(opts.name)?;
    let cli_trimmed = opts.cli.trim();
    if !matches!(cli_trimmed, "pi" | "claude" | "codex" | "agy") {
        return Err(format!(
            "invalid --cli '{}': expected one of pi, claude, codex, agy",
            opts.cli
        ));
    }
    if let Some(fb) = opts.fallback.and_then(non_empty_string) {
        if !matches!(fb, "pi" | "claude" | "codex" | "agy") {
            return Err(format!(
                "invalid --fallback '{fb}': expected one of pi, claude, codex, agy"
            ));
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
    let mut rows = Vec::new();
    for def in registry.agents() {
        if def.source == crate::agent::AgentSource::BuiltIn {
            continue;
        }
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

    if rows.is_empty() {
        return Ok("No agents found in workspace or global registry.".to_string());
    }

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
    Ok(lines.join("\n"))
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
    let spawn_command = match &def.harness {
        Some(h) => {
            let adapter = resolve_harness_adapter(h)?;
            Some(build_agent_spawn_command(&def, adapter, "<TASK>")?)
        }
        None => None,
    };

    if json {
        let harness_json = def.harness.as_ref().map(|h| {
            serde_json::json!({
                "cli": h.cli,
                "model": h.model,
                "fallback": h.fallback,
                "ephemeral": h.ephemeral,
                "result_schema": h.result_schema,
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
    let workspace_root = effective_cwd
        .as_deref()
        .and_then(crate::app::registry::resolve_agent_lookup_workspace_root);
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
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    let Some(ref harness_cfg) = def.harness else {
        eprintln!(
            "error: agent '{}' has no [harness] table in settings.toml (it is an Assistant persona and cannot be spawned as a CLI worker)",
            def.id
        );
        return 1;
    };
    let adapter = match resolve_harness_adapter(harness_cfg) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    let cmd_string = match build_agent_spawn_command(&def, adapter, task) {
        Ok(cmd) => cmd,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    let effective_ephemeral = ephemeral || harness_cfg.ephemeral;
    let parent_pane_id = from_pane_id.or_else(|| {
        std::env::var("PLEXI_PANE_ID")
            .ok()
            .and_then(|v| v.parse().ok())
    });

    let mut payload = serde_json::json!({
        "type": "agent_spawn",
        "role": def.id,
        "harness": adapter.harness.as_str(),
        "command": cmd_string,
        "task": task,
        "ephemeral": effective_ephemeral,
    });
    if let Some(sub) = subcontext.and_then(non_empty_string) {
        payload["subcontext"] = serde_json::Value::String(sub.to_string());
    }
    if let Some(pid) = parent_pane_id {
        payload["parent_pane_id"] = serde_json::json!(pid);
    }
    if let Some(dir) = effective_cwd.as_ref().and_then(|p| p.to_str()) {
        payload["cwd"] = serde_json::Value::String(dir.to_string());
    }
    if let Some(ref schema) = harness_cfg.result_schema {
        payload["result_schema"] = schema.clone();
    }

    let content = match super::request(payload, "agent-spawn-response", "agent spawn") {
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
    println!("{content}");
    0
}

/// Report print-mode worker exit even when the interactive pane shell stays alive.
/// Repository checks, sandbox settings, and approval flags remain harness-owned.
pub(crate) fn supervised_worker_command(command: &str, reporter: &str, harness: &str) -> String {
    // Keep worker model scopes explicit here when pi gains a supported flag. Do not inherit or edit
    // the user's ~/.pi model patterns: the harness owns that configuration.
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
    super::send_to_socket(payload)
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
        status |= install_pi_extension(&script_str);
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

fn agent_state_script(binary: &str) -> String {
    format!(
        r#"#!/usr/bin/env bash
set -euo pipefail
if [ -z "${{PLEXI_SOCKET:-}}" ] || [ -z "${{PLEXI_PANE_ID:-}}" ]; then
    exit 0
fi
INPUT=$(cat)
EVENT=$(jq -r '.hook_event_name // empty' <<< "$INPUT" 2>/dev/null || true)
AGENT_NAME="${{PLEXI_AGENT_NAME:-claude-code}}"

truncate_detail() {{
    local max="$1"
    local value="$2"
    if [ "${{#value}}" -gt "$max" ]; then
        printf '%s' "${{value:0:$max}}"
    else
        printf '%s' "$value"
    fi
}}

basename_detail() {{
    local value="$1"
    if [ -n "$value" ]; then
        basename "$value"
    fi
}}

tool_detail() {{
    local tool_name value base
    tool_name=$(jq -r '.tool_name // .toolName // empty' <<< "$INPUT" 2>/dev/null || true)
    case "$tool_name" in
        Bash|bash)
            value=$(jq -r '.tool_input.command // .tool_input.cmd // .input.command // .args.command // empty' <<< "$INPUT" 2>/dev/null || true)
            [ -n "$value" ] && printf 'Bash: %s' "$(truncate_detail 60 "$value")" || printf 'Bash'
            ;;
        apply_patch)
            value=$(jq -r '.tool_input.command // .input.command // empty' <<< "$INPUT" 2>/dev/null || true)
            [ -n "$value" ] && printf 'Patch: %s' "$(truncate_detail 60 "$value")" || printf 'Patch'
            ;;
        Edit|edit)
            value=$(jq -r '.tool_input.file_path // .tool_input.path // .input.file_path // .input.path // .args.file_path // .args.path // empty' <<< "$INPUT" 2>/dev/null || true)
            base=$(basename_detail "$value")
            [ -n "$base" ] && printf 'Edit: %s' "$base" || printf 'Edit'
            ;;
        Write|write)
            value=$(jq -r '.tool_input.file_path // .tool_input.path // .input.file_path // .input.path // .args.file_path // .args.path // empty' <<< "$INPUT" 2>/dev/null || true)
            base=$(basename_detail "$value")
            [ -n "$base" ] && printf 'Write: %s' "$base" || printf 'Write'
            ;;
        Read|read)
            value=$(jq -r '.tool_input.file_path // .tool_input.path // .input.file_path // .input.path // .args.file_path // .args.path // empty' <<< "$INPUT" 2>/dev/null || true)
            base=$(basename_detail "$value")
            [ -n "$base" ] && printf 'Read: %s' "$base" || printf 'Read'
            ;;
        WebSearch)
            value=$(jq -r '.tool_input.query // .input.query // empty' <<< "$INPUT" 2>/dev/null || true)
            [ -n "$value" ] && printf 'WebSearch: %s' "$(truncate_detail 50 "$value")" || printf 'WebSearch'
            ;;
        WebFetch)
            value=$(jq -r '.tool_input.url // .input.url // empty' <<< "$INPUT" 2>/dev/null || true)
            [ -n "$value" ] && printf 'WebFetch: %s' "$(truncate_detail 60 "$value")" || printf 'WebFetch'
            ;;
        Agent)
            value=$(jq -r '.tool_input.description // .tool_input.prompt // .input.description // .input.prompt // empty' <<< "$INPUT" 2>/dev/null || true)
            [ -n "$value" ] && printf 'Agent: %s' "$(truncate_detail 60 "$value")" || printf 'Agent'
            ;;
        "")
            ;;
        *)
            printf '%s' "$tool_name"
            ;;
    esac
}}

DETAIL=""
case "$EVENT" in
    PreToolUse|tool_call)             STATE="working"; DETAIL=$(tool_detail) ;;
    PermissionRequest)                STATE="blocked"; DETAIL=$(tool_detail) ;;
    UserPromptSubmit|agent_start|before_agent_start)
                                      STATE="working" ;;
    PostToolUse|PostToolBatch|tool_result)
                                      STATE="working"; DETAIL=$(tool_detail) ;;
    SessionStart|session_start)       STATE="idle" ;;
    Stop|agent_end)                   STATE="idle" ;;
    StopFailure)                     STATE="idle" ;;
    SessionEnd|session_shutdown)     STATE="idle" ;;
    UsageLimit|BootFailure)          STATE="blocked" ;;
    SubagentStart|SubagentStop)       exit 0 ;;
    *)                                exit 0 ;;
esac

SESSION_ID=$(jq -r '.session_id // .sessionId // empty' <<< "$INPUT" 2>/dev/null || true)
ARGS=(agent report --state "$STATE" --agent "$AGENT_NAME")
[ -n "$SESSION_ID" ] && ARGS+=(--session-id "$SESSION_ID")
[ -n "$DETAIL" ] && ARGS+=(--detail "$DETAIL")
PLEXI_AGENT_EVENT="$EVENT" "{binary}" "${{ARGS[@]}}" >/dev/null 2>&1 || true
exit 0
"#
    )
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

fn install_pi_extension(script_str: &str) -> i32 {
    let extension_path = pi_extension_path();
    install_pi_extension_at(&extension_path, script_str)
}

pub(crate) fn install_pi_extension_at(extension_path: &Path, script_str: &str) -> i32 {
    let content = pi_extension_script(script_str);
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

fn pi_extension_script(script_str: &str) -> String {
    format!(
        r#"// Plexi-managed agent state extension.
import {{ spawn }} from "node:child_process";
import {{ existsSync }} from "node:fs";
import type {{ ExtensionAPI }} from "@earendil-works/pi-coding-agent";

const hookCommand = {script_json};
const PLEXI_PI_PACKAGE_ACTIVE = "__PLEXI_PI_PACKAGE_ACTIVE__";
let turnStarted = false;

function isPiPlexiPackageActive(): boolean {{
  return Boolean((globalThis as Record<string, unknown>)[PLEXI_PI_PACKAGE_ACTIVE]) ||
    process.env.PLEXI_PI_PACKAGE_ACTIVE === "1";
}}

function isSessionPersisted(ctx: any): boolean {{
  if (turnStarted) return true;
  try {{
    const file = ctx?.sessionManager?.getSessionFile?.() ?? "";
    return typeof file === "string" && file.trim().length > 0 && existsSync(file.trim());
  }} catch {{
    return false;
  }}
}}

function sessionId(ctx: any, requirePersisted = false): string {{
  if (requirePersisted && !isSessionPersisted(ctx)) return "";
  try {{
    const id = ctx?.sessionManager?.getSessionId?.();
    if (typeof id === "string" && id.trim()) return id.trim();
    const file = ctx?.sessionManager?.getSessionFile?.();
    if (typeof file === "string" && file.trim()) return file.trim();
  }} catch {{
  }}
  return "";
}}

function report(eventName: string, payload: Record<string, unknown>, ctx: any) {{
  if (isPiPlexiPackageActive()) return;
  if (!process.env.PLEXI_SOCKET || !process.env.PLEXI_PANE_ID) return;
  if (eventName === "session_start") {{
    turnStarted = false;
  }} else if (
    eventName === "before_agent_start" ||
    eventName === "agent_start" ||
    eventName === "tool_call" ||
    eventName === "tool_result" ||
    eventName === "agent_end"
  ) {{
    turnStarted = true;
  }}
  const requirePersisted = eventName === "session_start" || eventName === "session_shutdown";
  try {{
    const child = spawn(hookCommand, [], {{
      env: {{ ...process.env, PLEXI_AGENT_NAME: "pi" }},
      stdio: ["pipe", "ignore", "ignore"],
    }});
    child.on("error", () => {{}});
    child.stdin.on("error", () => {{}});
    child.stdin.end(JSON.stringify({{
      hook_event_name: eventName,
      session_id: sessionId(ctx, requirePersisted),
      ...payload,
    }}));
  }} catch {{
  }}
}}

export default function (pi: ExtensionAPI) {{
  pi.on("session_start", (event, ctx) => report("session_start", {{ reason: event.reason }}, ctx));
  pi.on("before_agent_start", (_event, ctx) => report("before_agent_start", {{}}, ctx));
  pi.on("agent_start", (_event, ctx) => report("agent_start", {{}}, ctx));
  pi.on("tool_call", (event, ctx) => report("tool_call", {{
    tool_name: event.toolName,
    tool_input: event.args,
    tool_use_id: event.toolCallId,
  }}, ctx));
  pi.on("tool_result", (event, ctx) => report("tool_result", {{
    tool_name: event.toolName,
    tool_input: event.input,
    tool_use_id: event.toolCallId,
    is_error: event.isError,
  }}, ctx));
  pi.on("agent_end", (_event, ctx) => report("agent_end", {{}}, ctx));
  pi.on("session_shutdown", (_event, ctx) => report("session_shutdown", {{}}, ctx));
}}
"#,
        script_json = serde_json::to_string(script_str).unwrap_or_else(|_| "\"plexi\"".to_string())
    )
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
    fn pane_lifecycle_hook_preserves_stop_failure_and_session_end() {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;
        use std::process::{Command, Stdio};
        let dir = tempfile::tempdir().unwrap();
        let capture = dir.path().join("capture");
        fs::write(&capture, "#!/bin/sh\nprintf '%s\\n' \"$@\"\nprintf 'hook_event=%s\\n' \"${PLEXI_AGENT_EVENT:-}\"\n").unwrap();
        fs::set_permissions(&capture, fs::Permissions::from_mode(0o755)).unwrap();
        let script = dir.path().join("hook.sh");
        fs::write(
            &script,
            super::agent_state_script(capture.to_str().unwrap())
                .replace(">/dev/null 2>&1 || true", "|| true"),
        )
        .unwrap();
        let reports: Vec<_> = ["Stop", "StopFailure", "SessionEnd"]
            .into_iter()
            .map(|event| {
                let mut child = Command::new("bash")
                    .arg(&script)
                    .env("PLEXI_SOCKET", "unused-test-socket")
                    .env("PLEXI_PANE_ID", "7")
                    .env("PLEXI_AGENT_NAME", "claude-code")
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .unwrap();
                write!(
                    child.stdin.take().unwrap(),
                    "{}",
                    serde_json::json!({
                        "hook_event_name": event, "session_id": "test-session"
                    })
                )
                .unwrap();
                let output = child.wait_with_output().unwrap();
                assert!(
                    output.status.success(),
                    "{}",
                    String::from_utf8_lossy(&output.stderr)
                );
                let args = String::from_utf8(output.stdout).unwrap();
                assert!(
                    args.contains("agent\nreport\n"),
                    "hook did not report: {args}"
                );
                args
            })
            .collect();
        assert_ne!(
            reports[0], reports[1],
            "normal stop and failure lost their distinction at the source"
        );
        assert_ne!(
            reports[0], reports[2],
            "normal stop and session end lost their distinction at the source"
        );
        assert_ne!(
            reports[1], reports[2],
            "failure and session end lost their distinction at the source"
        );
        for (report, event) in reports.iter().zip(["Stop", "StopFailure", "SessionEnd"]) {
            assert!(
                report.contains(&format!("hook_event={event}\n")),
                "raw provenance missing: {report}"
            );
            assert!(
                !report.contains("--event\n"),
                "shared hook must retain old CLI arguments"
            );
        }
    }

    #[test]
    fn generated_hooks_detect_codex_and_pi() {
        let shell = super::agent_state_script("/usr/local/bin/plexi");
        assert!(shell.contains("PLEXI_AGENT_NAME"));
        assert!(shell.contains("PreToolUse|tool_call"));
        assert!(shell.contains("--agent \"$AGENT_NAME\""));

        let pi = super::pi_extension_script("/tmp/claude-code-agent-state.sh");
        assert!(pi.contains("PLEXI_AGENT_NAME: \"pi\""));
        assert!(pi.contains("pi.on(\"tool_call\""));
        assert!(pi.contains("pi.on(\"agent_end\""));
    }

    #[test]
    fn channel_neutral_hook_script_uses_the_bare_plexi_shim() {
        let shell = super::agent_state_script("plexi");
        assert!(shell.contains("\"plexi\" \"${ARGS[@]}\""));
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
            super::install_pi_extension_at(&ext_path, "/tmp/claude-code-agent-state.sh"),
            1
        );
        assert!(super::migrate_legacy_pi_extension_at(&ext_path).is_err());
        assert_eq!(fs::read_to_string(&ext_path).unwrap(), custom_content);
        assert!(!bak_path.exists());

        // 2. Legacy Plexi-generated extension is recognized, backed up to .plexi-hook.bak, and removed on migration.
        fs::remove_file(&ext_path).unwrap();
        assert_eq!(
            super::install_pi_extension_at(&ext_path, "/tmp/claude-code-agent-state.sh"),
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
        assert_eq!(String::from_utf8(output.stdout).unwrap(), "exec\n--skip-git-repo-check\n--sandbox\nworkspace-write\n-c\nsandbox_workspace_write.network_access=true\nsubmit a ballot\n");
        assert_eq!(adapter.spawn_flags_summary(None, false, &[]), "exec --skip-git-repo-check --sandbox workspace-write -c sandbox_workspace_write.network_access=true");
        assert!(!command.contains("--dangerously"));
        assert!(!command.contains("writable_roots"));
        assert!(!command.contains("--add-dir"));
    }
    #[test]
    fn multi_cli_harness_adapters_build_spawn_resume_and_capture_session_ids() {
        let claude = super::harness_adapter("claude").expect("claude adapter");
        assert_eq!(
            claude.resume_command("sess-claude-1"),
            "claude --resume sess-claude-1"
        );
        assert_eq!(
            claude.spawn_command("fix bug", Some("claude-sonnet-4-5"), None),
            "claude --model claude-sonnet-4-5 -p 'fix bug'"
        );
        assert_eq!(
            claude.capture_session_id(&serde_json::json!({"session_id": "s-1"})),
            Some("s-1".to_string())
        );

        let codex = super::harness_adapter("codex").expect("codex adapter");
        assert_eq!(codex.resume_command("thread-99"), "codex resume thread-99");
        assert_eq!(
            codex.spawn_command("run tests", None, None),
            "codex exec --skip-git-repo-check --sandbox workspace-write -c sandbox_workspace_write.network_access=true 'run tests'"
        );
        assert_eq!(
            codex.capture_session_id(&serde_json::json!({"thread_id": "t-99"})),
            Some("t-99".to_string())
        );

        let pi = super::harness_adapter("pi").expect("pi adapter");
        assert_eq!(pi.resume_command("pi-sess-7"), "pi --session pi-sess-7");
        assert_eq!(
            pi.spawn_command("audit module", None, None),
            "pi --no-extensions --no-mcp -p 'audit module'"
        );
        assert_eq!(
            pi.capture_session_id(&serde_json::json!({"sessionId": "pi-7"})),
            Some("pi-7".to_string())
        );

        let agy = super::harness_adapter("agy").expect("agy adapter");
        assert_eq!(agy.resume_command("conv-123"), "agy --resume conv-123");
        assert_eq!(
            agy.spawn_command("implement stint", None, None),
            "agy --prompt 'implement stint'"
        );
        assert_eq!(
            agy.capture_session_id(&serde_json::json!({"conversation_id": "conv-123"})),
            Some("conv-123".to_string())
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

        // 1. pi: uses --no-extensions --no-mcp, --append-system-prompt, --model verbatim, and appends worker contract + schema to task.
        let pi_def = make_def("pi", Some("openrouter/~z-ai/glm-flash-latest"));
        let pi_adapter = super::harness_adapter("pi").unwrap();
        let pi_cmd =
            super::build_agent_spawn_command(&pi_def, pi_adapter, "Find where AgentRegistry loads")
                .unwrap();
        assert!(
            pi_cmd.starts_with(
                "pi --no-extensions --no-mcp --model 'openrouter/~z-ai/glm-flash-latest' --append-system-prompt 'You are a fast codebase researcher.' -p "
            ),
            "unexpected pi command: {pi_cmd}"
        );
        assert!(pi_cmd.contains("--no-extensions"));
        assert!(pi_cmd.contains("--no-mcp"));
        assert!(pi_cmd.contains("Find where AgentRegistry loads"));
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

        // 2. claude: uses --append-system-prompt, -p, and appends worker contract.
        let claude_def = make_def("claude", Some("claude-sonnet-4-5"));
        let claude_adapter = super::harness_adapter("claude").unwrap();
        let claude_cmd =
            super::build_agent_spawn_command(&claude_def, claude_adapter, "Audit module").unwrap();
        assert!(
            claude_cmd.starts_with(
                "claude --model claude-sonnet-4-5 --append-system-prompt 'You are a fast codebase researcher.' -p "
            ),
            "unexpected claude command: {claude_cmd}"
        );
        assert!(claude_cmd.contains("# Plexi worker contract"));
        assert!(!claude_cmd.contains("--append-system-prompt-file"));
        assert!(!claude_cmd.contains("# Persona"));

        // 3. codex: no system-prompt flag; prepends `# Persona\n<AGENT.md>\n\n` to task.
        let codex_def = make_def("codex", Some("gpt-5-codex"));
        let codex_adapter = super::harness_adapter("codex").unwrap();
        let codex_cmd =
            super::build_agent_spawn_command(&codex_def, codex_adapter, "Run suite").unwrap();
        assert!(
            codex_cmd.starts_with("codex --model gpt-5-codex exec --skip-git-repo-check --sandbox workspace-write -c sandbox_workspace_write.network_access=true "),
            "unexpected codex command: {codex_cmd}"
        );
        assert!(!codex_cmd.contains("--append-system-prompt"));
        assert!(!codex_cmd.contains("--system-prompt"));
        assert!(
            codex_cmd.contains(
                "# Persona\nYou are a fast codebase researcher.\n\nRun suite\n\n# Plexi worker contract"
            ),
            "codex command must inline # Persona before task: {codex_cmd}"
        );

        // 4. agy: no system-prompt flag; prepends `# Persona\n<AGENT.md>\n\n` to task.
        let agy_def = make_def("agy", Some("gemini-2.5-pro"));
        let agy_adapter = super::harness_adapter("agy").unwrap();
        let agy_cmd =
            super::build_agent_spawn_command(&agy_def, agy_adapter, "Implement feature").unwrap();
        assert!(
            agy_cmd.starts_with("agy --model gemini-2.5-pro --prompt "),
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
                "pi --no-extensions --no-mcp --model sonnet --append-system-prompt 'Secret persona text that must not appear in flags log.' -p "
            ),
            "unexpected default pi command: {default_cmd}"
        );
        let default_flags = pi_adapter.spawn_flags_summary(
            Some("sonnet"),
            true,
            &default_def.harness.as_ref().unwrap().extensions,
        );
        assert_eq!(
            default_flags,
            "--no-extensions --no-mcp --model sonnet --append-system-prompt <AGENT.md> -p"
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
                "pi --no-extensions --no-mcp -e '/tmp/my ext.ts' -e builtin:plan-mode --append-system-prompt 'Secret persona text.' -p "
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
            },
            profile.path(),
        )
        .unwrap();

        let list_text = super::agent_list_in(profile.path(), Some(ws.path()), false).unwrap();
        assert!(list_text.contains("researcher") && list_text.contains("workspace"));
        assert!(list_text.contains("pi (openrouter/~z-ai/glm-flash-latest)"));
        assert!(list_text.contains("[shadows global]"));
        assert!(list_text.contains("reviewer") && list_text.contains("global"));

        let list_json: serde_json::Value = serde_json::from_str(
            &super::agent_list_in(profile.path(), Some(ws.path()), true).unwrap(),
        )
        .unwrap();
        let arr = list_json.as_array().unwrap();
        assert_eq!(arr.len(), 2);
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
        assert!(spawn_cmd.contains("pi --no-extensions --no-mcp --model 'openrouter/~z-ai/glm-flash-latest' --append-system-prompt 'Workspace persona.' -p"));
        assert!(spawn_cmd.contains("<TASK>"));
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
        assert_eq!(pi.restore_command(None), "pi");
        assert_eq!(pi.restore_command(Some("   ")), "pi");
        assert_eq!(claude.restore_command(None), "claude");
        assert_eq!(
            claude.restore_command(Some("sess-claude-1")),
            "claude --resume sess-claude-1"
        );

        // 2. Unpersisted Pi session UUID (no matching file on disk) falls back to bare `pi`.
        assert_eq!(pi.restore_command(Some("019dcdae-unpersisted")), "pi");

        // 3. Once `*_019dcdae-persisted.jsonl` exists under `~/.pi/agent/sessions/`,
        //    `restore_command` resumes via `pi --session 019dcdae-persisted`.
        let persisted_jsonl = proj_dir.join("2026-10-17T05-00-00-000Z_019dcdae-persisted.jsonl");
        fs::write(&persisted_jsonl, "{\"role\":\"user\"}\n").unwrap();
        assert_eq!(
            pi.restore_command(Some("019dcdae-persisted")),
            "pi --session 019dcdae-persisted"
        );

        // 4. Direct session file path that exists on disk also resumes via `pi --session <path>`.
        let direct_path = persisted_jsonl.to_string_lossy().to_string();
        assert_eq!(
            pi.restore_command(Some(&direct_path)),
            format!("pi --session {direct_path}")
        );
    }
}

/// Read the same host projection painted by the Agents pane.
pub fn agent_fleet_cli(json: bool, filter: crate::protocol::FleetFilter) -> i32 {
    let content = match super::request(serde_json::json!({"type":"agent_fleet", "filter":filter}), "plexi-agent-fleet", "agent fleet") {
        Ok(content) => content, Err(code) => return code,
    };
    let view: crate::agent::fleet::AgentsView = match serde_json::from_str(&content) {
        Ok(view) => view, Err(e) => { eprintln!("error: could not read agent fleet: {e}"); return 1; }
    };
    if json { println!("{content}"); return 0; }
    if view.tasks.is_empty() { println!("No matching agents."); }
    for task in view.tasks {
        println!("{} — {}", task.title, task.counts());
        for row in task.rows {
            println!("  #{} {} ({})  {}{}", row.pane_id, row.role, row.harness, row.display_state.label(), row.blocked_on.map(|b| format!(" — {}", b.label())).unwrap_or_default());
        }
    }
    0
}

pub fn agent_next_cli(json: bool) -> i32 {
    let content = match super::request(serde_json::json!({"type":"agent_next"}), "plexi-agent-next", "agent next") {
        Ok(content) => content,
        Err(code) => return code,
    };
    if json { println!("{content}"); }
    else {
        match serde_json::from_str::<serde_json::Value>(&content) {
            Ok(value) => match value["pane_id"].as_u64() {
                Some(id) => println!("{id}"),
                None => println!("No agents need attention."),
            },
            Err(error) => { eprintln!("error: invalid agent next response: {error}"); return 1; }
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

//! The file-writing layer: `plexi workspace init` scaffolding and the
//! in-place upserts that keep user comments in `secrets.toml` intact.

use std::path::Path;

use super::resolver::{WorkspaceConfig, WorkspaceSecrets};

/// `plexi workspace init` scaffolds all workspace files under `channel_dir`
/// (e.g. `.plexi-alpha/` or `.plexi/` for main):
///   - `<root>/<channel_dir>/workspace.toml` with a fresh UUID (idempotent)
///   - `<root>/<channel_dir>/secrets.toml` with `fallback = true`
///   - `<root>/<channel_dir>/.gitignore` so secrets never end up in git
///
/// `channel_dir` is the dot-prefixed workspace channel directory name
/// (e.g. `.plexi-alpha`, `.plexi`, `.plexi-pr-N`).
///
/// Returns the resolved `WorkspaceConfig` so the caller can echo the UUID.
pub fn init_workspace(workspace_root: &Path, channel_dir: &str) -> Result<WorkspaceConfig, String> {
    // Write workspace.toml under the channel dir
    let ws_path = workspace_root.join(channel_dir).join("workspace.toml");
    let cfg = if ws_path.exists() {
        let raw = std::fs::read_to_string(&ws_path)
            .map_err(|e| format!("read {}: {e}", ws_path.display()))?;
        toml::from_str::<WorkspaceConfig>(&raw)
            .map_err(|e| format!("parse {}: {e}", ws_path.display()))?
    } else {
        // Create the channel dir and write a fresh workspace.toml
        let dir = workspace_root.join(channel_dir);
        std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
        let id = uuid::Uuid::new_v4().to_string();
        std::fs::write(&ws_path, format!("id = \"{id}\"\n"))
            .map_err(|e| format!("write {}: {e}", ws_path.display()))?;
        WorkspaceConfig { id, context: None }
    };

    // secrets.toml under the channel dir
    let secrets_path = workspace_root.join(channel_dir).join("secrets.toml");
    if !secrets_path.exists() {
        let template = "# Workspace secret routing — see issue #322.\n\
                        # fallback: when no [apps.<id>] / [default] route matches a canonical\n\
                        # secret, true allows reading plexi:user:<name>; false errors loudly.\n\
                        fallback = true\n\
                        \n\
                        # [apps.<app-id>]\n\
                        # OPENAI_API_KEY = \"openai_personal\"\n\
                        \n\
                        # [default]\n\
                        # GITHUB_TOKEN = \"github_personal\"\n\
                        \n\
                        # [terminal.env]\n\
                        # inject = [\"OPENAI_API_KEY\"]\n";
        std::fs::write(&secrets_path, template)
            .map_err(|e| format!("write {}: {e}", secrets_path.display()))?;
    }

    // stub apps.toml under the channel dir
    let apps_toml = workspace_root.join(channel_dir).join("apps.toml");
    if !apps_toml.exists() {
        let stub = concat!(
            "schema_version = 1\n\n",
            "# Declare workspace app dependencies here.\n",
            "# Run `plexi app install` in this directory to install them.\n",
            "#\n",
            "# Example:\n",
            "#\n",
            "# [[app]]\n",
            "# id      = \"gh-issues\"\n",
            "# source  = \"local:gh-issues\"\n",
            "# version = \"bundled\"\n",
            "#\n",
            "# [[app]]\n",
            "# id      = \"my-tool\"\n",
            "# source  = \"github:org/my-tool\"\n",
            "# version = \"v1.0.0\"\n",
        );
        std::fs::write(&apps_toml, stub)
            .map_err(|e| format!("write {}: {e}", apps_toml.display()))?;
    }

    // stub commands.toml under the channel dir
    let commands_toml = workspace_root.join(channel_dir).join("commands.toml");
    if !commands_toml.exists() {
        let stub = concat!(
            "# Workspace commands — run with: plexi run <name>\n",
            "#\n",
            "# Simple form:   build = \"cargo build\"\n",
            "# With metadata: dev = { run = \"npm run dev\", description = \"Start dev server\" }\n",
            "# With secrets:  deploy = { run = \"./deploy.sh\", secrets = [\"API_KEY\"] }\n",
            "\n",
            "[commands]\n",
            "guess = \"$PLEXI_CONFIG_DIR/scripts/guess\"\n",
        );
        std::fs::write(&commands_toml, stub)
            .map_err(|e| format!("write {}: {e}", commands_toml.display()))?;
    }

    write_gitignore_if_absent(workspace_root)?;
    Ok(cfg)
}

/// Ensure channel-neutral app state cannot be committed with a context root.
///
/// App state is personal, single-user, local data — never committed, never
/// shared. Existing user rules are preserved byte-for-byte and the required
/// entry is appended only when absent.
pub(crate) fn ensure_app_state_gitignore(workspace_root: &Path) -> Result<(), String> {
    use std::io::Write;

    let dir = workspace_root.join(".plexi");
    std::fs::create_dir_all(&dir).map_err(|error| format!("create {}: {error}", dir.display()))?;
    let path = dir.join(".gitignore");
    let contents = match std::fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(format!("read {}: {error}", path.display())),
    };
    if contents.lines().any(|line| line.trim() == "app_states/") {
        return Ok(());
    }
    let prefix = if contents.is_empty() || contents.ends_with('\n') {
        ""
    } else {
        "\n"
    };
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|error| format!("open {} for append: {error}", path.display()))?;
    file.write_all(format!("{prefix}app_states/\n").as_bytes())
        .map_err(|error| format!("append {}: {error}", path.display()))?;
    file.sync_all()
        .map_err(|error| format!("sync {}: {error}", path.display()))
}

/// Default contents for `<root>/.plexi/.gitignore`. Anything that holds a
/// secret value or is generated host state lives here.
const GITIGNORE_TEMPLATE: &str = "# Auto-generated by plexi workspace init.\n\
                                  # Edit this file freely — re-running init never overwrites it.\n\
                                  secrets.toml\n\
                                  cache/\n\
                                  agents/*/memory/\n\
                                  agents/*/logs/\n";

/// Write `<root>/.plexi/.gitignore` only when the file does not already exist.
/// User edits to an existing file are preserved verbatim.
fn write_gitignore_if_absent(workspace_root: &Path) -> Result<(), String> {
    let dir = workspace_root.join(crate::config::workspace_channel_dir());
    std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let path = dir.join(".gitignore");
    if path.exists() {
        return Ok(());
    }
    std::fs::write(&path, GITIGNORE_TEMPLATE)
        .map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(())
}

// ── Route auto-write ─────────────────────────────────────────────────────────

/// After a `plexi secret set` Keychain write, record the canonical→friendly
/// route in `<workspace_root>/.plexi/secrets.toml` under `[default]`.
///
/// - File absent: creates it with `fallback = true` + the route.
/// - Canonical already maps to the same friendly: no-op (idempotent).
/// - Canonical maps to a different friendly: updates the existing entry in-place.
/// - Canonical not present: injects a new entry, preserving existing content.
pub fn write_default_route(
    workspace_root: &Path,
    canonical: &str,
    friendly: &str,
) -> Result<(), String> {
    let secrets_path = workspace_root
        .join(crate::config::workspace_channel_dir())
        .join("secrets.toml");

    if !secrets_path.exists() {
        let dir = workspace_root.join(crate::config::workspace_channel_dir());
        std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
        let content = format!("fallback = true\n\n[default]\n{canonical} = \"{friendly}\"\n");
        return std::fs::write(&secrets_path, content)
            .map_err(|e| format!("write {}: {e}", secrets_path.display()));
    }

    let raw = std::fs::read_to_string(&secrets_path)
        .map_err(|e| format!("read {}: {e}", secrets_path.display()))?;

    // Idempotency: bail out only when the mapping already points at the same friendly name.
    if let Ok(Some(router)) = WorkspaceSecrets::load(workspace_root) {
        if router.default.get(canonical).map(|s| s.as_str()) == Some(friendly) {
            return Ok(());
        }
    }

    // Insert or update the canonical→friendly mapping.
    let updated = upsert_default_route_line(&raw, canonical, friendly);
    std::fs::write(&secrets_path, updated)
        .map_err(|e| format!("write {}: {e}", secrets_path.display()))
}

/// Update `[terminal.env] inject = [...]` in workspace `secrets.toml`.
///
/// Workspace-scoped secrets default to terminal injection on when created by
/// the native Secrets app or CLI. This helper is also used by the native app
/// toggle so the TOML file remains the durable source of policy.
pub fn write_terminal_env_inject(
    workspace_root: &Path,
    canonical: &str,
    enabled: bool,
) -> Result<(), String> {
    let secrets_path = workspace_root
        .join(crate::config::workspace_channel_dir())
        .join("secrets.toml");

    if !secrets_path.exists() {
        let dir = workspace_root.join(crate::config::workspace_channel_dir());
        std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
        let inject = if enabled {
            format!("  \"{canonical}\",\n")
        } else {
            String::new()
        };
        let content = format!("fallback = true\n\n[terminal.env]\ninject = [\n{inject}]\n");
        return std::fs::write(&secrets_path, content)
            .map_err(|e| format!("write {}: {e}", secrets_path.display()));
    }

    let raw = std::fs::read_to_string(&secrets_path)
        .map_err(|e| format!("read {}: {e}", secrets_path.display()))?;

    let mut names = WorkspaceSecrets::parse(&raw)
        .map(|router| router.terminal.env.inject)
        .unwrap_or_default();
    let already_present = names.iter().any(|name| name == canonical);
    if enabled && !already_present {
        names.push(canonical.to_string());
    } else if !enabled && already_present {
        names.retain(|name| name != canonical);
    } else {
        return Ok(());
    }

    let updated = upsert_terminal_env_inject_section(&raw, &names);
    std::fs::write(&secrets_path, updated)
        .map_err(|e| format!("write {}: {e}", secrets_path.display()))
}

/// Insert or update `canonical = "friendly"` in the `[default]` section of a
/// raw `secrets.toml` string, creating the section if absent.
/// If an existing `canonical = "..."` line is found, it is replaced in-place.
/// All other content and comments are preserved.
fn upsert_default_route_line(raw: &str, canonical: &str, friendly: &str) -> String {
    let entry_line = format!("{canonical} = \"{friendly}\"");
    let lines: Vec<&str> = raw.lines().collect();
    let trailing_newline = raw.ends_with('\n');

    if let Some(start) = lines.iter().position(|l| {
        let t = l.trim();
        t == "[default]" || (t.starts_with("[default]") && t[9..].trim_start().starts_with('#'))
    }) {
        // End of section: next uncommented table header, or EOF.
        let end = lines[start + 1..]
            .iter()
            .position(|l| {
                let t = l.trim();
                t.starts_with('[') && !t.starts_with('#')
            })
            .map(|p| start + 1 + p)
            .unwrap_or(lines.len());

        // Check for an existing entry for this canonical key and replace it if found.
        let canonical_prefix = format!("{canonical} = ");
        let existing = lines[start + 1..end]
            .iter()
            .position(|l| l.trim().starts_with(&canonical_prefix))
            .map(|p| start + 1 + p);

        let result = if let Some(idx) = existing {
            let mut parts: Vec<&str> = Vec::with_capacity(lines.len());
            parts.extend_from_slice(&lines[..idx]);
            parts.push(&entry_line);
            parts.extend_from_slice(&lines[idx + 1..]);
            parts
        } else {
            // No existing entry — append inside section before next section header.
            let mut parts: Vec<&str> = Vec::with_capacity(lines.len() + 1);
            parts.extend_from_slice(&lines[..end]);
            parts.push(&entry_line);
            parts.extend_from_slice(&lines[end..]);
            parts
        };

        let joined = result.join("\n");
        if trailing_newline {
            format!("{joined}\n")
        } else {
            joined
        }
    } else {
        // No [default] section — append one.
        let base = raw.trim_end_matches('\n');
        format!("{base}\n\n[default]\n{entry_line}\n")
    }
}

fn upsert_terminal_env_inject_section(raw: &str, names: &[String]) -> String {
    let mut names = names.to_vec();
    names.sort();
    names.dedup();

    let section = render_terminal_env_inject_section(&names);
    let lines: Vec<&str> = raw.lines().collect();
    let trailing_newline = raw.ends_with('\n');

    if let Some(start) = lines.iter().position(|line| {
        let trimmed = line.trim();
        trimmed == "[terminal.env]"
            || (trimmed.starts_with("[terminal.env]")
                && trimmed[14..].trim_start().starts_with('#'))
    }) {
        let end = lines[start + 1..]
            .iter()
            .position(|line| {
                let trimmed = line.trim();
                trimmed.starts_with('[') && !trimmed.starts_with('#')
            })
            .map(|pos| start + 1 + pos)
            .unwrap_or(lines.len());

        let mut parts: Vec<&str> = Vec::with_capacity(lines.len() + section.lines().count());
        parts.extend_from_slice(&lines[..start]);
        parts.extend(section.trim_end_matches('\n').lines());
        parts.extend_from_slice(&lines[end..]);
        let joined = parts.join("\n");
        if trailing_newline {
            format!("{joined}\n")
        } else {
            joined
        }
    } else {
        let base = raw.trim_end_matches('\n');
        if base.is_empty() {
            section
        } else {
            format!("{base}\n\n{section}")
        }
    }
}

fn render_terminal_env_inject_section(names: &[String]) -> String {
    let mut out = String::from("[terminal.env]\ninject = [\n");
    for name in names {
        out.push_str("  \"");
        out.push_str(name);
        out.push_str("\",\n");
    }
    out.push_str("]\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_config_load_or_init_writes_uuid_when_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let channel_dir = crate::config::workspace_channel_dir();
        let cfg = init_workspace(tmp.path(), &channel_dir).expect("load_or_init");
        assert!(uuid::Uuid::parse_str(&cfg.id).is_ok());
        // Idempotent — second call returns the same id.
        let cfg2 = init_workspace(tmp.path(), &channel_dir).expect("second load");
        assert_eq!(cfg.id, cfg2.id);
    }

    #[test]
    fn init_workspace_creates_workspace_and_secrets_files() {
        let tmp = tempfile::tempdir().unwrap();
        let channel_dir = crate::config::workspace_channel_dir();
        init_workspace(tmp.path(), &channel_dir).expect("init_workspace");
        assert!(tmp
            .path()
            .join(&channel_dir)
            .join("workspace.toml")
            .is_file());
        let secrets_raw =
            std::fs::read_to_string(tmp.path().join(&channel_dir).join("secrets.toml")).unwrap();
        // Generated secrets.toml must parse cleanly with the required field.
        let parsed = WorkspaceSecrets::parse(&secrets_raw).expect("template parses");
        assert!(parsed.fallback);
    }

    #[test]
    fn init_writes_gitignore_when_absent() {
        let tmp = tempfile::tempdir().unwrap();
        let channel_dir = crate::config::workspace_channel_dir();
        let gitignore = tmp.path().join(&channel_dir).join(".gitignore");
        assert!(!gitignore.exists());

        init_workspace(tmp.path(), &channel_dir).expect("init_workspace");

        assert!(
            gitignore.is_file(),
            "init must create {channel_dir}/.gitignore"
        );
        let raw = std::fs::read_to_string(&gitignore).unwrap();
        assert!(raw.contains("secrets.toml"), "got: {raw}");
        assert!(raw.contains("cache/"), "got: {raw}");
    }

    #[test]
    fn init_preserves_existing_gitignore() {
        let tmp = tempfile::tempdir().unwrap();
        let channel_dir = crate::config::workspace_channel_dir();
        let dir = tmp.path().join(&channel_dir);
        std::fs::create_dir_all(&dir).unwrap();
        let gitignore = dir.join(".gitignore");
        let custom = "# my own rules\nfoo\nbar\n";
        std::fs::write(&gitignore, custom).unwrap();

        init_workspace(tmp.path(), &channel_dir).expect("init_workspace");

        let raw = std::fs::read_to_string(&gitignore).unwrap();
        assert_eq!(
            raw, custom,
            "init must NOT overwrite an existing .gitignore"
        );
    }

    #[test]
    fn app_state_gitignore_preserves_rules_and_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let neutral_dir = tmp.path().join(".plexi");
        std::fs::create_dir_all(&neutral_dir).unwrap();
        let gitignore = neutral_dir.join(".gitignore");
        std::fs::write(&gitignore, "# personal\ncustom/\n").unwrap();

        ensure_app_state_gitignore(tmp.path()).expect("first ensure");
        ensure_app_state_gitignore(tmp.path()).expect("second ensure");

        assert_eq!(
            std::fs::read_to_string(gitignore).unwrap(),
            "# personal\ncustom/\napp_states/\n"
        );
    }

    /// The standing ruling made effective: in a real git repository, a state
    /// file under `<root>/.plexi/app_states/` must be invisible to git after
    /// the ensure runs — a user cannot accidentally commit their app state.
    #[test]
    fn app_state_gitignore_is_effective_in_a_real_repo() {
        let repo = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .current_dir(repo.path())
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_SYSTEM", "/dev/null")
                .output()
                .expect("run git")
        };
        assert!(git(&["init", "-q"]).status.success(), "git init");

        ensure_app_state_gitignore(repo.path()).expect("ensure gitignore");
        let state_dir = repo.path().join(".plexi").join("app_states");
        std::fs::create_dir_all(&state_dir).unwrap();
        std::fs::write(state_dir.join("todo.json"), b"{\"k\":1}").unwrap();

        let check = git(&["check-ignore", "-q", ".plexi/app_states/todo.json"]);
        assert!(
            check.status.success(),
            "git must ignore the state file (check-ignore exit {:?})",
            check.status.code()
        );
        let status = git(&["status", "--porcelain"]);
        let listing = String::from_utf8_lossy(&status.stdout).to_string();
        assert!(
            !listing.contains("app_states"),
            "git status must not surface app state: {listing:?}"
        );
    }

    // ── write_default_route tests ──────────────────────────────────────────────

    #[test]
    fn write_default_route_creates_file_when_absent() {
        let tmp = tempfile::tempdir().unwrap();
        let channel_dir = crate::config::workspace_channel_dir();
        std::fs::create_dir_all(tmp.path().join(&channel_dir)).unwrap();
        write_default_route(tmp.path(), "AGE", "AGE").expect("should succeed");
        let raw =
            std::fs::read_to_string(tmp.path().join(&channel_dir).join("secrets.toml")).unwrap();
        assert!(raw.contains("fallback = true"), "missing fallback: {raw}");
        assert!(raw.contains("[default]"), "missing section: {raw}");
        assert!(raw.contains("AGE = \"AGE\""), "missing route: {raw}");
        WorkspaceSecrets::parse(&raw).expect("created file must parse");
    }

    #[test]
    fn write_default_route_idempotent_when_route_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let channel_dir = crate::config::workspace_channel_dir();
        std::fs::create_dir_all(tmp.path().join(&channel_dir)).unwrap();
        let initial = "fallback = true\n\n[default]\nAGE = \"AGE\"\n";
        std::fs::write(tmp.path().join(&channel_dir).join("secrets.toml"), initial).unwrap();
        write_default_route(tmp.path(), "AGE", "AGE").expect("should succeed");
        let raw =
            std::fs::read_to_string(tmp.path().join(&channel_dir).join("secrets.toml")).unwrap();
        // Content must not grow — no duplicate entries.
        assert_eq!(
            raw.matches("AGE = \"AGE\"").count(),
            1,
            "duplicate entry: {raw}"
        );
    }

    #[test]
    fn write_default_route_appends_to_existing_default_section() {
        let tmp = tempfile::tempdir().unwrap();
        let channel_dir = crate::config::workspace_channel_dir();
        std::fs::create_dir_all(tmp.path().join(&channel_dir)).unwrap();
        let initial = "fallback = true\n\n[default]\nGITHUB_TOKEN = \"gh_personal\"\n";
        std::fs::write(tmp.path().join(&channel_dir).join("secrets.toml"), initial).unwrap();
        write_default_route(tmp.path(), "AGE", "AGE").expect("should succeed");
        let raw =
            std::fs::read_to_string(tmp.path().join(&channel_dir).join("secrets.toml")).unwrap();
        assert!(
            raw.contains("GITHUB_TOKEN = \"gh_personal\""),
            "existing entry lost: {raw}"
        );
        assert!(raw.contains("AGE = \"AGE\""), "new entry missing: {raw}");
        WorkspaceSecrets::parse(&raw).expect("file must still parse");
    }

    #[test]
    fn write_default_route_appends_new_section_when_none_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let channel_dir = crate::config::workspace_channel_dir();
        std::fs::create_dir_all(tmp.path().join(&channel_dir)).unwrap();
        let initial = "fallback = true\n\n# [default]\n# GITHUB_TOKEN = \"gh\"\n";
        std::fs::write(tmp.path().join(&channel_dir).join("secrets.toml"), initial).unwrap();
        write_default_route(tmp.path(), "AGE", "AGE").expect("should succeed");
        let raw =
            std::fs::read_to_string(tmp.path().join(&channel_dir).join("secrets.toml")).unwrap();
        assert!(raw.contains("[default]"), "section missing: {raw}");
        assert!(raw.contains("AGE = \"AGE\""), "entry missing: {raw}");
        WorkspaceSecrets::parse(&raw).expect("file must parse");
    }

    #[test]
    fn write_default_route_with_alias_writes_friendly_name() {
        let tmp = tempfile::tempdir().unwrap();
        let channel_dir = crate::config::workspace_channel_dir();
        std::fs::create_dir_all(tmp.path().join(&channel_dir)).unwrap();
        write_default_route(tmp.path(), "OPENAI_API_KEY", "openai_personal")
            .expect("should succeed");
        let raw =
            std::fs::read_to_string(tmp.path().join(&channel_dir).join("secrets.toml")).unwrap();
        assert!(
            raw.contains("OPENAI_API_KEY = \"openai_personal\""),
            "route wrong: {raw}"
        );
    }

    #[test]
    fn write_default_route_updates_existing_entry_when_alias_changes() {
        let tmp = tempfile::tempdir().unwrap();
        let channel_dir = crate::config::workspace_channel_dir();
        std::fs::create_dir_all(tmp.path().join(&channel_dir)).unwrap();
        let initial = "fallback = true\n\n[default]\nOPENAI_API_KEY = \"old_alias\"\n";
        std::fs::write(tmp.path().join(&channel_dir).join("secrets.toml"), initial).unwrap();
        write_default_route(tmp.path(), "OPENAI_API_KEY", "new_alias").expect("should succeed");
        let raw =
            std::fs::read_to_string(tmp.path().join(&channel_dir).join("secrets.toml")).unwrap();
        assert!(
            raw.contains("OPENAI_API_KEY = \"new_alias\""),
            "updated entry missing: {raw}"
        );
        assert!(!raw.contains("old_alias"), "stale entry not removed: {raw}");
        WorkspaceSecrets::parse(&raw).expect("must parse");
    }

    #[test]
    fn write_terminal_env_inject_creates_file_when_absent() {
        let tmp = tempfile::tempdir().unwrap();
        let channel_dir = crate::config::workspace_channel_dir();
        std::fs::create_dir_all(tmp.path().join(&channel_dir)).unwrap();

        write_terminal_env_inject(tmp.path(), "OPENROUTER_API_KEY", true).expect("should succeed");

        let raw =
            std::fs::read_to_string(tmp.path().join(&channel_dir).join("secrets.toml")).unwrap();
        let parsed = WorkspaceSecrets::parse(&raw).expect("must parse");
        assert!(parsed.fallback);
        assert_eq!(
            parsed.terminal.env.inject,
            vec!["OPENROUTER_API_KEY".to_string()]
        );
    }

    #[test]
    fn write_terminal_env_inject_adds_and_removes_name() {
        let tmp = tempfile::tempdir().unwrap();
        let channel_dir = crate::config::workspace_channel_dir();
        std::fs::create_dir_all(tmp.path().join(&channel_dir)).unwrap();
        let initial = "fallback = true\n\n[default]\nOPENAI_API_KEY = \"openai\"\n";
        std::fs::write(tmp.path().join(&channel_dir).join("secrets.toml"), initial).unwrap();

        write_terminal_env_inject(tmp.path(), "OPENAI_API_KEY", true).expect("enable");
        write_terminal_env_inject(tmp.path(), "OPENROUTER_API_KEY", true).expect("enable");
        write_terminal_env_inject(tmp.path(), "OPENAI_API_KEY", false).expect("disable");

        let raw =
            std::fs::read_to_string(tmp.path().join(&channel_dir).join("secrets.toml")).unwrap();
        let parsed = WorkspaceSecrets::parse(&raw).expect("must parse");
        assert_eq!(
            parsed.default.get("OPENAI_API_KEY").map(String::as_str),
            Some("openai")
        );
        assert_eq!(
            parsed.terminal.env.inject,
            vec!["OPENROUTER_API_KEY".to_string()]
        );
    }

    #[test]
    fn write_terminal_env_inject_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let channel_dir = crate::config::workspace_channel_dir();
        std::fs::create_dir_all(tmp.path().join(&channel_dir)).unwrap();
        let initial = "fallback = true\n\n[terminal.env]\ninject = [\n  \"OPENAI_API_KEY\",\n]\n";
        std::fs::write(tmp.path().join(&channel_dir).join("secrets.toml"), initial).unwrap();

        write_terminal_env_inject(tmp.path(), "OPENAI_API_KEY", true).expect("enable");

        let raw =
            std::fs::read_to_string(tmp.path().join(&channel_dir).join("secrets.toml")).unwrap();
        assert_eq!(
            raw.matches("OPENAI_API_KEY").count(),
            1,
            "duplicate entry: {raw}"
        );
    }

    #[test]
    fn upsert_default_route_line_appends_when_no_section() {
        let raw = "fallback = true\n";
        let out = upsert_default_route_line(raw, "X", "x_alias");
        assert!(out.contains("[default]"), "{out}");
        assert!(out.contains("X = \"x_alias\""), "{out}");
        WorkspaceSecrets::parse(&out).expect("must parse: {out}");
    }

    #[test]
    fn upsert_default_route_line_inserts_before_next_section() {
        let raw = "fallback = true\n\n[default]\nA = \"a\"\n\n[apps.foo]\nB = \"b\"\n";
        let out = upsert_default_route_line(raw, "C", "c");
        // C must appear inside [default], before [apps.foo]
        let default_pos = out.find("[default]").unwrap();
        let apps_pos = out.find("[apps.foo]").unwrap();
        let c_pos = out.find("C = \"c\"").unwrap();
        assert!(
            c_pos > default_pos && c_pos < apps_pos,
            "C not in [default]: {out}"
        );
        WorkspaceSecrets::parse(&out).expect("must parse");
    }

    #[test]
    fn upsert_default_route_line_handles_inline_comment_on_section_header() {
        let raw = "fallback = true\n\n[default] # route table\nA = \"a\"\n";
        let out = upsert_default_route_line(raw, "B", "b_alias");
        assert!(out.contains("B = \"b_alias\""), "entry missing: {out}");
        assert!(
            out.contains("[default] # route table"),
            "header modified: {out}"
        );
        WorkspaceSecrets::parse(&out).expect("must parse");
    }

    #[test]
    fn upsert_default_route_line_replaces_existing_entry() {
        let raw = "fallback = true\n\n[default]\nFOO = \"old\"\nBAR = \"bar\"\n";
        let out = upsert_default_route_line(raw, "FOO", "new");
        assert!(out.contains("FOO = \"new\""), "replacement missing: {out}");
        assert!(
            !out.contains("FOO = \"old\""),
            "old entry not removed: {out}"
        );
        assert!(out.contains("BAR = \"bar\""), "sibling entry lost: {out}");
        WorkspaceSecrets::parse(&out).expect("must parse");
    }
}

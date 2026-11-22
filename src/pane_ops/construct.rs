//! The one terminal constructor (stint 1632).
//!
//! Every terminal pane is built by [`PlexiApp::construct_terminal`] from an
//! owned [`TerminalLaunch`] carrying an admitted (or validated restored)
//! grant's live handle, in this order:
//!
//! 1. the handle exists for the pane's real id before anything can
//!    authenticate;
//! 2. bind the working directory exactly as the launch states it;
//! 3. build the permitted environment, resolving only the secret classes the
//!    grant is eligible for, from the directory the grant bounds them to;
//! 4. issue the call credential and the host-MCP bearer from the handle,
//!    last, and start the backend.
//!
//! Both credentials carry the handle, so every authenticator resolves the
//! grant and the current scope revision from the credential itself. A sealed
//! grant gets neither credential. The returned pane owns its credentials;
//! dropping a pane that was never installed (or closing an installed one)
//! revokes the handle and both credentials. Placement receives a finished
//! pane; it never chooses a directory, a secret class, or an authority.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::app::PlexiApp;
use crate::broker::gate::PermissionMonitor;
use crate::broker::pane_authority::LivePaneAuthority;
use crate::host::pane::{TerminalPane, TerminalSpawnError};
use crate::host::pane_env::PaneBase;
use crate::host::shell;

use super::create::PaneContextEnv;

/// The command a terminal starts with instead of an interactive shell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LaunchCommand {
    pub(crate) command: String,
    pub(crate) close_on_exit: bool,
}

/// The host-MCP bearer a pane receives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum McpBearer {
    /// A bearer registered for this pane's binding.
    Pane,
    /// A team worker's agent run token. No pane bearer is minted under it.
    RunToken(String),
}

/// Everything a terminal is constructed from, decided before construction.
/// Who the pane is comes only from `authority`.
#[derive(Clone, Debug)]
pub(crate) struct TerminalLaunch {
    pub(crate) authority: Arc<LivePaneAuthority>,
    pub(crate) context: PaneContextEnv,
    /// Exactly the directory the shell starts in; `None` is the backend
    /// default.
    pub(crate) cwd: Option<PathBuf>,
    pub(crate) command: Option<LaunchCommand>,
    /// The pane closes when its shell exits, with or without a command.
    pub(crate) ephemeral: bool,
    /// Worker metadata the pane's environment carries.
    pub(crate) metadata: PaneMetadata,
    pub(crate) mcp: McpBearer,
    pub(crate) profile_dir: PathBuf,
}

/// The agent-worker metadata a launch states, exported under its fixed
/// `PLEXI_` names. Typed so no caller can place an arbitrary variable in a
/// pane's environment (stint 1682).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct PaneMetadata {
    pub(crate) parent_pane: Option<u64>,
    pub(crate) spawned_by: Option<String>,
    pub(crate) ephemeral: bool,
    pub(crate) harness: Option<String>,
    pub(crate) role: Option<String>,
    pub(crate) agent_id: Option<String>,
    pub(crate) task: Option<String>,
    pub(crate) resume_command: Option<String>,
}

impl PaneMetadata {
    /// Every name this writes, for the reservation test.
    #[cfg(test)]
    pub(crate) const NAMES: &'static [&'static str] = &[
        "PLEXI_PARENT_PANE",
        "PLEXI_SPAWNED_BY",
        "PLEXI_EPHEMERAL",
        "PLEXI_AGENT_HARNESS",
        "PLEXI_AGENT_ROLE",
        "PLEXI_AGENT_ID",
        "PLEXI_AGENT_TASK",
        "PLEXI_RESUME_COMMAND",
    ];

    fn write(&self, env: &mut HashMap<String, String>) {
        let mut put = |key: &str, value: Option<String>| {
            if let Some(value) = value {
                env.insert(key.to_string(), value);
            }
        };
        put(
            "PLEXI_PARENT_PANE",
            self.parent_pane.map(|id| id.to_string()),
        );
        put("PLEXI_SPAWNED_BY", self.spawned_by.clone());
        put("PLEXI_EPHEMERAL", self.ephemeral.then(|| "1".to_string()));
        put("PLEXI_AGENT_HARNESS", self.harness.clone());
        put("PLEXI_AGENT_ROLE", self.role.clone());
        put("PLEXI_AGENT_ID", self.agent_id.clone());
        put("PLEXI_AGENT_TASK", self.task.clone());
        put("PLEXI_RESUME_COMMAND", self.resume_command.clone());
    }
}

impl TerminalLaunch {
    /// The pane id the grant was admitted for.
    pub(crate) fn pane_id(&self) -> u64 {
        self.authority.pane_id()
    }
}

/// The credentials issued for one pane and the binding they were issued
/// from. Owned by the pane: each credential is held by its exact handle, so
/// revoking them can never touch another pane's, even one that reuses this
/// pane id. They are revoked when the pane is logically removed
/// ([`Self::revoke`]), and at the latest when they drop.
pub(crate) struct PaneCredentials {
    call: Option<(Arc<PermissionMonitor>, String)>,
    /// This pane's own host-MCP registration, or the run token whose
    /// association with this pane it holds. `None` with no MCP server.
    mcp: Option<String>,
    authority: Arc<LivePaneAuthority>,
}

impl std::fmt::Debug for PaneCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PaneCredentials")
            .field("pane_id", &self.authority.pane_id())
            .field("incarnation", &self.authority.incarnation())
            .field("call", &self.call.is_some())
            .field("mcp", &self.mcp.is_some())
            .finish()
    }
}

impl PaneCredentials {
    /// Issue the pane's credentials from its installed binding and publish
    /// them into `env`. This is the first instant either can authenticate.
    fn issue(
        authority: Arc<LivePaneAuthority>,
        profile_dir: &std::path::Path,
        mcp: &McpBearer,
        env: &mut HashMap<String, String>,
    ) -> Self {
        let binding = authority.clone();
        let pane_id = binding.pane_id();
        if binding.grant().posture().is_err() {
            log::info!("pane_env: pane {pane_id} is sealed; no credential issued");
            let credentials = Self {
                call: None,
                mcp: None,
                authority,
            };
            #[cfg(test)]
            probes::published(&credentials, env);
            return credentials;
        }
        let call = match PermissionMonitor::for_profile(profile_dir) {
            Ok(monitor) => {
                let token = monitor.issue_pane_credential(&binding);
                env.insert("PLEXI_CALL_CREDENTIAL".into(), token.clone());
                Some((monitor, token))
            }
            Err(error) => {
                log::error!("pane_env: no call credential for pane {pane_id}: {error}");
                None
            }
        };
        let mcp = match mcp {
            McpBearer::Pane => {
                crate::app::host_mcp::register_pane_bearer(&binding).map(|(port, token)| {
                    env.insert("PLEXI_HOST_MCP_PORT".into(), port.to_string());
                    env.insert("PLEXI_HOST_MCP_TOKEN".into(), token.clone());
                    token
                })
            }
            McpBearer::RunToken(token) => {
                if let Some(port) = crate::app::host_mcp::bound_port() {
                    env.insert("PLEXI_HOST_MCP_PORT".into(), port.to_string());
                }
                env.insert("PLEXI_HOST_MCP_TOKEN".into(), token.clone());
                // The run acts for this pane too: requests under its token
                // are bounded by the run's limits and this pane's ceiling.
                crate::app::host_mcp::associate_run_token(token, &binding);
                Some(token.clone())
            }
        };
        let credentials = Self {
            call,
            mcp,
            authority,
        };
        #[cfg(test)]
        probes::published(&credentials, env);
        credentials
    }

    pub(crate) fn authority(&self) -> &Arc<LivePaneAuthority> {
        &self.authority
    }

    /// Revoke the handle and both credentials now. Idempotent: the tokens
    /// are taken, so a later call (or the drop) revokes nothing further.
    pub(crate) fn revoke(&mut self) {
        self.authority.revoke();
        let call = self.call.take();
        let mcp = self.mcp.take();
        if call.is_none() && mcp.is_none() {
            return;
        }
        if let Some((monitor, token)) = call {
            monitor.revoke_credential(&token);
        }
        if let Some(token) = mcp {
            crate::app::host_mcp::revoke_pane_bearer(&token);
        }
        log::info!(
            "pane_credentials: revoked pane={} incarnation={}",
            self.authority.pane_id(),
            self.authority.incarnation()
        );
    }

    /// A holder of `authority` with no credential issued (tests).
    #[cfg(test)]
    pub(crate) fn without_tokens(authority: Arc<LivePaneAuthority>) -> Self {
        Self {
            call: None,
            mcp: None,
            authority,
        }
    }

    #[cfg(test)]
    pub(crate) fn mcp_token(&self) -> Option<&str> {
        self.mcp.as_deref()
    }

    #[cfg(test)]
    pub(crate) fn call_token(&self) -> Option<&str> {
        self.call.as_ref().map(|(_, token)| token.as_str())
    }
}

impl Drop for PaneCredentials {
    fn drop(&mut self) {
        self.revoke();
    }
}

impl PlexiApp {
    /// Construct one terminal pane from `launch`. See the module docs for
    /// the order. The pane is not installed anywhere; dropping it on a
    /// failed placement releases everything construction acquired.
    pub(crate) fn construct_terminal(
        &self,
        launch: TerminalLaunch,
    ) -> Result<TerminalPane, TerminalSpawnError> {
        Self::construct_terminal_in(
            &self.ctx,
            &self.pty_event_tx,
            &self.colors,
            self.default_font_size,
            self.pane_base.as_deref(),
            launch,
        )
    }

    /// [`Self::construct_terminal`] for a caller that has no `PlexiApp` yet
    /// (workspace restore). `base` is the host's pane base; without one
    /// (Unix) no terminal can be built.
    pub(crate) fn construct_terminal_in(
        ctx: &egui::Context,
        pty_event_tx: &std::sync::mpsc::Sender<(u64, egui_term::PtyEvent)>,
        colors: &crate::ui::theme::Colors,
        default_font_size: f32,
        base: Option<&PaneBase>,
        launch: TerminalLaunch,
    ) -> Result<TerminalPane, TerminalSpawnError> {
        let TerminalLaunch {
            authority,
            context,
            cwd,
            command,
            ephemeral,
            metadata,
            mcp,
            profile_dir,
        } = launch;
        // 1. The handle for the real id exists before anything can
        // authenticate.
        let grant = authority.grant();
        let pane_id = grant.pane_id;
        let agent_pane = !grant.is_desktop();
        let secrets = grant.secrets.policy();

        // 2 and 3. The bound directory and the environment permitted for it,
        // with secrets resolved only from the directory the grant bounds.
        log::info!(
            "construct_terminal: pane_id={pane_id} context_id={} cwd={cwd:?} secrets={secrets:?} secret_dir={:?} agent_pane={agent_pane}",
            context.context_id,
            grant.secrets.directory
        );
        #[cfg(unix)]
        let (shell_path, mut env, cwd) = {
            // The base comes from the account and the host profile, never
            // from this process's environment (stint 1682).
            let Some(base) = base else {
                log::error!("construct_terminal: pane {pane_id} has no pane base");
                return Err(TerminalSpawnError {
                    pane_id,
                    program: "<shell>".into(),
                    working_directory: cwd,
                    cause:
                        "the host has no pane environment base (account lookup failed at launch)"
                            .into(),
                });
            };
            #[cfg(test)]
            let shell_path =
                shell::test_shell_override().unwrap_or_else(|| base.shell().to_string());
            #[cfg(not(test))]
            let shell_path = base.shell().to_string();
            let mut env = base.env(pane_id, grant.is_desktop(), &shell_path);
            if shell_path.ends_with("/zsh") || shell_path.ends_with("/zsh-5") {
                match shell::ensure_shell_integration() {
                    Ok(zdotdir) => {
                        env.insert("ZDOTDIR".into(), zdotdir.to_string_lossy().into_owned());
                    }
                    Err(error) => log::warn!("Failed to set up shell integration: {error}"),
                }
            }
            insert_secrets(
                &mut env,
                shell::secret_env(grant.secrets.directory.as_deref(), secrets),
            );
            let cwd = pane_start_dir(pane_id, cwd.or_else(|| context.root.clone()), base.home());
            (shell_path, env, Some(cwd))
        };
        #[cfg(windows)]
        let (shell_path, mut env) = {
            let _ = base;
            (
                shell::detect_shell(),
                shell::windows_pane_env(grant.secrets.directory.as_deref(), secrets),
            )
        };
        env.insert("PLEXI_RUNNING".into(), "1".into());
        env.insert("PLEXI_PANE_ID".into(), pane_id.to_string());
        #[cfg(windows)]
        env.insert("PLEXI_SOCKET".into(), crate::platform::ipc::endpoint());
        env.insert("PLEXI_CONTEXT_ID".into(), context.context_id.to_string());
        env.insert("PLEXI_CONTEXT_NAME".into(), context.name.clone());
        env.insert(
            "PLEXI_CONTEXT_DESCRIPTION".into(),
            context.description.clone(),
        );
        if let Some(root) = &context.root {
            env.insert(
                "PLEXI_CONTEXT_ROOT".into(),
                root.to_string_lossy().into_owned(),
            );
        }
        env.insert("PLEXI_CONTEXT_DEPTH".into(), context.depth.to_string());
        if agent_pane {
            env.insert("PLEXI_AGENT_SPAWNED".into(), "1".into());
        }
        metadata.write(&mut env);
        crate::broker::seal::scrub_pane_env(&mut env, agent_pane);
        let args = shell::default_shell_args(&shell_path, agent_pane);
        let mut settings = egui_term::BackendSettings {
            shell: shell_path,
            args,
            env,
            dynamic_colors: crate::ui::theme::terminal_dynamic_colors(colors),
            working_directory: cwd,
            byte_tap: None,
        };
        if let Some(command) = &command {
            super::apply_initial_cmd(&mut settings, &command.command, command.close_on_exit);
        }

        // 4. Credentials last, from the installed binding; then the backend.
        let credentials = PaneCredentials::issue(authority, &profile_dir, &mcp, &mut settings.env);
        let mut pane = TerminalPane::start(
            pane_id,
            ctx.clone(),
            pty_event_tx.clone(),
            settings,
            default_font_size,
            credentials,
        )?;
        pane.ephemeral |= ephemeral;
        Ok(pane)
    }
}

/// Where a Unix pane starts (stint 1682): the admitted directory (the
/// launch's, else its context root), else the account home; never the host
/// process's own cwd. An admitted directory that no longer exists (a deleted
/// context root) still gets the user a terminal, in the account home, with a
/// warning naming the missing directory. One that exists but cannot be
/// entered is passed on, and the spawn fails with that error.
#[cfg(unix)]
fn pane_start_dir(
    pane_id: u64,
    admitted: Option<std::path::PathBuf>,
    home: &std::path::Path,
) -> std::path::PathBuf {
    match admitted {
        Some(dir) => match std::fs::symlink_metadata(&dir) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                log::warn!(
                    "construct_terminal: pane {pane_id} admitted directory {} does not exist; starting in the account home {}",
                    dir.display(),
                    home.display()
                );
                home.to_path_buf()
            }
            _ => dir,
        },
        None => home.to_path_buf(),
    }
}

/// Add the secret writer's output. Every name it emits is an exportable
/// secret name or its own `PLEXI_TERMINAL_ENV_*` restoration pair, so it
/// can never replace a base or metadata variable.
#[cfg(unix)]
fn insert_secrets(env: &mut HashMap<String, String>, secrets: HashMap<String, String>) {
    for (key, value) in secrets {
        if env.contains_key(&key) {
            log::error!("construct_terminal: secret writer emitted base name {key}; dropped");
            continue;
        }
        env.insert(key, value);
    }
}

/// Test probes at the two instants construction order is about: credential
/// publication and backend entry.
#[cfg(test)]
pub(crate) mod probes {
    use std::cell::RefCell;
    use std::collections::HashMap;

    type Published = Box<dyn FnMut(u64, &HashMap<String, String>)>;
    type BackendEntry = Box<dyn FnMut(&egui_term::BackendSettings) -> Result<(), String>>;

    thread_local! {
        static PUBLISHED: RefCell<Option<Published>> = RefCell::new(None);
        static BACKEND_ENTRY: RefCell<Option<BackendEntry>> = RefCell::new(None);
    }

    /// Run `probe` with the pane id and environment at the instant the pane's
    /// credentials are published, on this thread, until the guard drops.
    pub(crate) fn on_publish(probe: impl FnMut(u64, &HashMap<String, String>) + 'static) -> Guard {
        PUBLISHED.with(|p| *p.borrow_mut() = Some(Box::new(probe)));
        Guard(Slot::Published)
    }

    /// Run `probe` with the backend settings just before the backend starts;
    /// an `Err` fails the start.
    pub(crate) fn on_backend_entry(
        probe: impl FnMut(&egui_term::BackendSettings) -> Result<(), String> + 'static,
    ) -> Guard {
        BACKEND_ENTRY.with(|p| *p.borrow_mut() = Some(Box::new(probe)));
        Guard(Slot::BackendEntry)
    }

    enum Slot {
        Published,
        BackendEntry,
    }

    /// Removes its probe when dropped.
    pub(crate) struct Guard(Slot);

    impl Drop for Guard {
        fn drop(&mut self) {
            match self.0 {
                Slot::Published => PUBLISHED.with(|p| *p.borrow_mut() = None),
                Slot::BackendEntry => BACKEND_ENTRY.with(|p| *p.borrow_mut() = None),
            }
        }
    }

    pub(super) fn published(credentials: &super::PaneCredentials, env: &HashMap<String, String>) {
        let pane_id = credentials.authority().pane_id();
        PUBLISHED.with(|p| {
            if let Some(probe) = p.borrow_mut().as_mut() {
                probe(pane_id, env);
            }
        });
    }

    /// The environment `launch` would start its backend with, stopping at
    /// backend entry so no process starts.
    pub(crate) fn env_at_backend_entry(launch: super::TerminalLaunch) -> HashMap<String, String> {
        let seen = std::rc::Rc::new(RefCell::new(None));
        let record = seen.clone();
        let _entry = on_backend_entry(move |settings| {
            *record.borrow_mut() = Some(settings.env.clone());
            Err("stopped at backend entry".into())
        });
        let (tx, _rx) = std::sync::mpsc::channel();
        let colors = crate::ui::theme::Colors::from_config(&crate::config::ThemeConfig::default());
        let base = crate::host::pane_env::PaneBase::test_base();
        let started = crate::app::PlexiApp::construct_terminal_in(
            &egui::Context::default(),
            &tx,
            &colors,
            14.0,
            base.as_deref(),
            launch,
        );
        assert!(
            started.is_err(),
            "the probe stops construction at backend entry"
        );
        let env = seen.borrow_mut().take();
        env.expect("construction reached backend entry")
    }

    pub(crate) fn backend_entry(settings: &egui_term::BackendSettings) -> Result<(), String> {
        BACKEND_ENTRY.with(|p| match p.borrow_mut().as_mut() {
            Some(probe) => probe(settings),
            None => Ok(()),
        })
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// Stint 1682: every variable a base or metadata writer emits, and the
    /// credentials and session marker added after them, is a name no secret
    /// may take. So no stored secret can ever travel as base environment.
    #[test]
    fn every_name_a_base_or_metadata_writer_emits_is_reserved() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let profile = tmp.path().join("profile");
        let shared = tmp.path().join("shared");
        std::fs::create_dir_all(&profile).unwrap();
        std::fs::create_dir_all(&shared).unwrap();
        let _profile = crate::config::set_test_profile_dir(profile.clone());
        let _shared = crate::config::set_test_shared_dir(shared);
        let _shell = shell::set_test_shell_override("/bin/zsh");
        let cwd = tmp.path().to_path_buf();
        let context = super::super::create::PaneContextEnv {
            context_id: 1,
            name: "main".into(),
            description: "d".into(),
            root: Some(cwd.clone()),
            depth: 1,
        };
        let mut seen: Vec<String> = Vec::new();
        for (pane_id, desktop) in [(9_316_820, true), (9_316_821, false)] {
            let mut grant = crate::broker::pane_authority::PaneAuthority::for_test(
                pane_id,
                crate::broker::pane_authority::Ceiling::Baseline,
                context.context_id,
                cwd.clone(),
            );
            if desktop {
                grant.ownership = crate::broker::pane_authority::Ownership::Desktop;
            }
            let launch = TerminalLaunch {
                authority: LivePaneAuthority::admitted(grant),
                context: context.clone(),
                cwd: Some(cwd.clone()),
                command: Some(LaunchCommand {
                    command: "true".into(),
                    close_on_exit: true,
                }),
                ephemeral: true,
                metadata: PaneMetadata {
                    parent_pane: Some(1),
                    spawned_by: Some("human".into()),
                    ephemeral: true,
                    harness: Some("claude".into()),
                    role: Some("worker".into()),
                    agent_id: Some("a".into()),
                    task: Some("t".into()),
                    resume_command: Some("r".into()),
                },
                mcp: McpBearer::Pane,
                profile_dir: profile.clone(),
            };
            let record = std::rc::Rc::new(std::cell::RefCell::new(None));
            let sink = record.clone();
            let _entry = probes::on_backend_entry(move |settings| {
                *sink.borrow_mut() = Some(settings.env.clone());
                Err("stopped at backend entry".into())
            });
            let (tx, _rx) = std::sync::mpsc::channel();
            let colors =
                crate::ui::theme::Colors::from_config(&crate::config::ThemeConfig::default());
            let base = PaneBase::fixture_with_every_variable();
            assert!(PlexiApp::construct_terminal_in(
                &egui::Context::default(),
                &tx,
                &colors,
                14.0,
                Some(&base),
                launch,
            )
            .is_err());
            let env = record.borrow_mut().take().expect("backend entry reached");
            if desktop {
                assert!(env.contains_key("SSH_AUTH_SOCK"), "{env:?}");
                assert!(env.contains_key("ZDOTDIR"), "{env:?}");
            } else {
                assert!(
                    !env.contains_key("SSH_AUTH_SOCK"),
                    "restricted pane got the agent"
                );
            }
            seen.extend(env.into_keys());
        }
        seen.extend(PaneMetadata::NAMES.iter().map(|name| name.to_string()));
        seen.sort();
        seen.dedup();
        let unreserved: Vec<&String> = seen
            .iter()
            .filter(|name| !crate::workspace::secrets::is_reserved_env_name(name))
            .collect();
        assert!(
            unreserved.is_empty(),
            "unreserved pane names: {unreserved:?}"
        );
        for name in [
            "HOME",
            "PATH",
            "LANG",
            "TMPDIR",
            "WINDOWID",
            "PLEXI_SOCKET",
            "PLEXI_AGENT_TASK",
        ] {
            assert!(
                seen.iter().any(|seen| seen == name),
                "{name} missing: {seen:?}"
            );
        }
    }
    /// Stint 1682: a pane whose admitted directory (a launch cwd or a context
    /// root) was deleted still opens, in the account home, with a warning
    /// naming the missing directory; one that exists is used as admitted,
    /// even if it cannot be entered (the spawn then fails with that error);
    /// none at all means the account home, never the host's own cwd.
    #[test]
    fn a_deleted_admitted_directory_starts_the_pane_in_the_account_home() {
        let logs = crate::testing::test_log::capture();
        let home = std::path::Path::new("/account/home-1682");
        let gone = tempfile::tempdir().unwrap().path().join("deleted-root");
        assert_eq!(pane_start_dir(1, Some(gone.clone()), home), home);
        assert!(
            logs.records()
                .iter()
                .any(|(level, line)| *level == log::Level::Warn
                    && line.contains(&gone.display().to_string())
                    && line.contains("does not exist")),
            "the missing directory is named in a warning"
        );
        let present = tempfile::tempdir().unwrap();
        assert_eq!(
            pane_start_dir(2, Some(present.path().to_path_buf()), home),
            present.path()
        );
        assert_eq!(pane_start_dir(3, None, home), home);
    }
}

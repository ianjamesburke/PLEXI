//! The one terminal constructor (stint 1632).
//!
//! Every terminal pane is built by [`PlexiApp::construct_terminal`] from an
//! owned [`TerminalLaunch`], in this order:
//!
//! 1. create the pane's authority binding for its real id;
//! 2. bind the working directory exactly as the launch states it;
//! 3. build the permitted environment, resolving only the secret classes the
//!    launch is eligible for, from that directory;
//! 4. issue the call credential and the host-MCP bearer from the installed
//!    binding, last, and start the backend.
//!
//! Both credentials carry the binding, so every authenticator resolves the
//! ceiling from the credential itself. The returned pane owns its
//! credentials; dropping a pane that was never installed (or closing an
//! installed one) revokes both. Placement receives a finished pane;
//! it never chooses a directory, a secret class, or an authority.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::app::PlexiApp;
use crate::broker::gate::PermissionMonitor;
use crate::broker::pane_authority::PaneAuthority;
use crate::broker::PermissionPosture;
use crate::host::pane::{TerminalPane, TerminalSpawnError};
use crate::host::shell::{self, SecretPolicy};
use crate::spatial::tiling::PaneId;

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
#[derive(Clone, Debug)]
pub(crate) struct TerminalLaunch {
    pub(crate) pane_id: PaneId,
    pub(crate) posture: Option<PermissionPosture>,
    pub(crate) spawned_by: Option<String>,
    pub(crate) context: PaneContextEnv,
    /// Exactly the directory the shell starts in; `None` is the backend
    /// default, for which no directory-derived secret is resolved.
    pub(crate) cwd: Option<PathBuf>,
    pub(crate) secrets: SecretPolicy,
    pub(crate) agent_pane: bool,
    pub(crate) command: Option<LaunchCommand>,
    /// The pane closes when its shell exits, with or without a command.
    pub(crate) ephemeral: bool,
    /// Worker metadata (`PLEXI_AGENT_*`, `PLEXI_PARENT_PANE`, ...).
    pub(crate) env: Vec<(String, String)>,
    pub(crate) mcp: McpBearer,
    pub(crate) profile_dir: PathBuf,
}

/// The credentials issued for one pane and the binding they were issued
/// from. Owned by the pane: each credential is held by its exact handle, so
/// revoking them can never touch another pane's, even one that reuses this
/// pane id. They are revoked when the pane is logically removed
/// ([`Self::revoke`]), and at the latest when they drop.
pub(crate) struct PaneCredentials {
    call: Option<(Arc<PermissionMonitor>, String)>,
    /// This pane's own host-MCP registration. `None` under a team run token
    /// or with no MCP server.
    mcp: Option<String>,
    authority: Arc<PaneAuthority>,
}

impl std::fmt::Debug for PaneCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PaneCredentials")
            .field("pane_id", &self.authority.pane_id)
            .field("call", &self.call.is_some())
            .field("mcp", &self.mcp.is_some())
            .finish()
    }
}

impl PaneCredentials {
    /// Issue the pane's credentials from its installed binding and publish
    /// them into `env`. This is the first instant either can authenticate.
    fn issue(
        authority: Arc<PaneAuthority>,
        profile_dir: &std::path::Path,
        mcp: &McpBearer,
        env: &mut HashMap<String, String>,
    ) -> Self {
        let binding = authority.clone();
        let pane_id = binding.pane_id;
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
                None
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

    pub(crate) fn authority(&self) -> &Arc<PaneAuthority> {
        &self.authority
    }

    /// Revoke both credentials now. Idempotent: the handles are taken, so a
    /// later call (or the drop) revokes nothing further.
    pub(crate) fn revoke(&mut self) {
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
        log::info!("pane_credentials: revoked pane={}", self.authority.pane_id);
    }

    /// Point this pane's MCP registration at a moved context root.
    pub(crate) fn rebind_workspace(&self, workspace_root: PathBuf) {
        if let Some(token) = &self.mcp {
            crate::app::host_mcp::rebind_pane_bearer(token, workspace_root);
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
            launch,
        )
    }

    /// [`Self::construct_terminal`] for a caller that has no `PlexiApp` yet
    /// (workspace restore).
    pub(crate) fn construct_terminal_in(
        ctx: &egui::Context,
        pty_event_tx: &std::sync::mpsc::Sender<(u64, egui_term::PtyEvent)>,
        colors: &crate::ui::theme::Colors,
        default_font_size: f32,
        launch: TerminalLaunch,
    ) -> Result<TerminalPane, TerminalSpawnError> {
        let TerminalLaunch {
            pane_id,
            posture,
            spawned_by,
            context,
            cwd,
            secrets,
            agent_pane,
            command,
            ephemeral,
            env: worker_env,
            mcp,
            profile_dir,
        } = launch;
        // 1. Authority for the real id, before anything can authenticate.
        let workspace_root = context
            .root
            .clone()
            .or_else(|| cwd.clone())
            .unwrap_or_default();
        let authority = Arc::new(PaneAuthority {
            pane_id,
            posture,
            spawned_by,
            context_id: context.context_id,
            workspace_root,
        });

        // 2 and 3. The bound directory and the environment permitted for it.
        log::info!(
            "construct_terminal: pane_id={pane_id} context_id={} cwd={cwd:?} secrets={secrets:?} agent_pane={agent_pane}",
            context.context_id
        );
        let mut env = shell::build_env_with_secrets(cwd.as_deref(), secrets);
        for name in [
            "PLEXI_PARENT_PANE",
            "PLEXI_EPHEMERAL",
            "PLEXI_AGENT_HARNESS",
            "PLEXI_AGENT_ROLE",
            "PLEXI_AGENT_ID",
            "PLEXI_AGENT_SPAWNED",
        ] {
            env.remove(name);
        }
        env.insert("PLEXI_PANE_ID".into(), pane_id.to_string());
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
        crate::broker::seal::scrub_pane_env(&mut env, agent_pane);
        env.extend(worker_env);
        let shell_path = shell::detect_shell();
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
        let pane_id = credentials.authority().pane_id;
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
        let started = crate::app::PlexiApp::construct_terminal_in(
            &egui::Context::default(),
            &tx,
            &colors,
            14.0,
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

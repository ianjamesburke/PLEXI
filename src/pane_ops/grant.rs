//! Admitting a terminal's grant (stint 1632): the host-side half of the one
//! grant builder, [`PaneAuthority::admit`]. Every terminal producer names its
//! verified source here and receives the live handle its pane is constructed
//! under. A wire pane id may select placement; it never selects a source.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::app::PlexiApp;
use crate::broker::pane_authority::{
    AdmittedRole, CredentialScope, GrantRequest, GrantSource, LivePaneAuthority, PaneAuthority,
};
use crate::spatial::tiling::PaneId;

use super::construct::{LaunchCommand, McpBearer, TerminalLaunch};
use super::create::PaneContextEnv;

/// The credential scope of a context: its id and canonical root, or the
/// directory when the context has no root.
pub(crate) fn credential_scope(context: &PaneContextEnv, cwd: Option<&Path>) -> CredentialScope {
    let root = context
        .root
        .clone()
        .or_else(|| cwd.map(Path::to_path_buf))
        .unwrap_or_default();
    CredentialScope {
        context_id: context.context_id,
        workspace_root: crate::platform::path::canonical_or_self(&root),
    }
}

impl PlexiApp {
    /// The grant source a verified caller pane provides: its own grant. A
    /// caller with no established grant provides a sealed one, so the new
    /// pane is sealed too.
    pub(crate) fn caller_grant_source(&self, caller: PaneId) -> GrantSource {
        match self.pane_authority(caller) {
            Some(live) => GrantSource::VerifiedPane(Box::new(live.grant().clone())),
            None => {
                log::info!("pane_grant: caller pane {caller} has no grant; its children are sealed");
                let scope = CredentialScope {
                    context_id: 0,
                    workspace_root: PathBuf::new(),
                };
                GrantSource::VerifiedPane(Box::new(PaneAuthority::sealed(
                    &crate::config::config_dir(),
                    caller,
                    crate::broker::pane_authority::Provenance::LegacyImport,
                    None,
                    "unknown".into(),
                    scope,
                )))
            }
        }
    }

    /// The display provenance of a pane admitted from `source`.
    pub(crate) fn spawned_by_for(&self, source: &GrantSource) -> String {
        match source {
            GrantSource::DesktopInput | GrantSource::PluginInstance { .. } => "human".into(),
            GrantSource::HostSystem => "system".into(),
            GrantSource::VerifiedPane(caller) => match &caller.role {
                Some(role) => format!("agent:{role}"),
                None => self.derive_spawned_by_for_parent(Some(caller.pane_id)),
            },
            GrantSource::OutsideProcess => "outside".into(),
            GrantSource::SpawnQueue => "queue".into(),
            GrantSource::Team => "team".into(),
        }
    }

    /// Admit a grant for a new pane id and return its live handle.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn admit_grant(
        &self,
        pane_id: PaneId,
        source: GrantSource,
        parent: Option<PaneId>,
        role: Option<AdmittedRole>,
        scope: CredentialScope,
        secret_directory: Option<PathBuf>,
        folder_secrets: bool,
        agent_command: bool,
        spawned_by: Option<String>,
    ) -> Arc<LivePaneAuthority> {
        let spawned_by = spawned_by.unwrap_or_else(|| self.spawned_by_for(&source));
        // A plain pane a verified pane asked for hangs under it.
        let parent = parent.or(match &source {
            GrantSource::VerifiedPane(caller) => Some(caller.pane_id),
            _ => None,
        });
        LivePaneAuthority::admitted(PaneAuthority::admit(
            &crate::config::config_dir(),
            GrantRequest {
                pane_id,
                source,
                parent,
                role,
                scope,
                secret_directory,
                folder_secrets,
                agent_command,
                spawned_by,
            },
        ))
    }

    /// A plain terminal launch admitted from `source` into `context`, in
    /// `cwd`. Secrets, if the source may have any, are bound to `cwd`.
    pub(crate) fn terminal_launch(
        &mut self,
        source: GrantSource,
        context: PaneContextEnv,
        cwd: Option<PathBuf>,
        command: Option<LaunchCommand>,
    ) -> TerminalLaunch {
        self.terminal_launch_with(source, context, cwd, command, true)
    }

    /// [`Self::terminal_launch`], choosing whether folder secrets are wanted.
    pub(crate) fn terminal_launch_with(
        &mut self,
        source: GrantSource,
        context: PaneContextEnv,
        cwd: Option<PathBuf>,
        command: Option<LaunchCommand>,
        folder_secrets: bool,
    ) -> TerminalLaunch {
        let pane_id = self.host.alloc_pane_id();
        let scope = credential_scope(&context, cwd.as_deref());
        let authority = self.admit_grant(
            pane_id,
            source,
            None,
            None,
            scope,
            cwd.clone(),
            folder_secrets,
            false,
            None,
        );
        TerminalLaunch {
            authority,
            context,
            cwd,
            ephemeral: command.as_ref().is_some_and(|c| c.close_on_exit),
            command,
            env: Vec::new(),
            mcp: McpBearer::Pane,
            profile_dir: crate::config::config_dir(),
        }
    }

    /// A terminal the person opened through the host's own input.
    pub(crate) fn desktop_terminal_launch(
        &mut self,
        context: PaneContextEnv,
        cwd: Option<PathBuf>,
        command: Option<LaunchCommand>,
    ) -> TerminalLaunch {
        self.terminal_launch(GrantSource::DesktopInput, context, cwd, command)
    }

    /// A terminal a plugin view instance asked for.
    pub(crate) fn plugin_terminal_launch(
        &mut self,
        plugin_pane: PaneId,
        context: PaneContextEnv,
        cwd: Option<PathBuf>,
        command: Option<LaunchCommand>,
    ) -> TerminalLaunch {
        self.terminal_launch(
            GrantSource::PluginInstance {
                pane_id: plugin_pane,
            },
            context,
            cwd,
            command,
        )
    }
}

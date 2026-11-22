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
            Some(live) => {
                let mut grant = live.grant().clone();
                // A desktop shell running a detected agent acts for that
                // agent now: what it opens is agent-spawned, never the
                // person's (stint 1632).
                if grant.is_desktop() && !self.is_human_owned_pane(caller) {
                    log::info!(
                        "pane_grant: caller pane {caller} runs an agent; its children are restricted"
                    );
                    grant.ownership = crate::broker::pane_authority::Ownership::Restricted;
                }
                GrantSource::VerifiedPane(Box::new(grant))
            }
            None => {
                log::info!(
                    "pane_grant: caller pane {caller} has no grant; its children are sealed"
                );
                let scope = CredentialScope {
                    context_id: 0,
                    workspace_root: PathBuf::new(),
                };
                GrantSource::VerifiedPane(Box::new(PaneAuthority::sealed(
                    crate::broker::pane_authority::SealReason::Unestablished,
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
            metadata: Default::default(),
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

    /// The way out of a sealed pane: a new terminal the person opens beside
    /// it, in its directory, admitted from their own input. The sealed pane
    /// is left as it is.
    pub(crate) fn open_pane_beside_sealed(&mut self, sealed: PaneId) -> Option<PaneId> {
        let (win_idx, tile) = self.find_pane_in_any_window(sealed)?;
        let cwd = self.windows[win_idx].get_focused_pane_cwd(tile);
        let context = self.pane_context_env_for_window(win_idx);
        let launch = self.desktop_terminal_launch(context, cwd, None);
        let opened = self.spawn_terminal_pane_at(
            super::create::SplitPlacement {
                win_idx,
                target_tile: tile,
                vertical: true,
                new_pane_first: false,
                keep_focus: false,
            },
            launch,
        );
        match &opened {
            Ok(pane) => log::info!("sealed_notice: opened pane {pane} beside sealed pane {sealed}"),
            Err(error) => log::warn!("sealed_notice: no pane beside sealed pane {sealed}: {error}"),
        }
        opened.ok()
    }

    /// After panes move between contexts, publish each moved terminal's new
    /// context and root through its handle, so its credentials, its peer
    /// lookup and its saved record agree on where it lives (stint 1632).
    pub(crate) fn rescope_moved_panes(&mut self) {
        let mut moved = Vec::new();
        for win in &self.windows {
            for (pane_id, pane) in win.panes.iter() {
                let Some(authority) = pane.terminal_incarnation().and_then(|t| t.authority())
                else {
                    continue;
                };
                if authority.scope().context_id != win.context_id {
                    moved.push((*pane_id, authority.clone(), win.context_id));
                }
            }
        }
        for (pane_id, authority, context_id) in moved {
            let context = self.pane_context_env_for_context(context_id);
            let scope = credential_scope(&context, None);
            log::info!("pane_grant: pane {pane_id} now lives in context {context_id}");
            authority.publish_move(scope.context_id, scope.workspace_root);
        }
    }
}

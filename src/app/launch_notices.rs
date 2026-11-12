//! One-time launch checks: is this binary the channel's installed build, are
//! old copies of the plugin lying around, and did the saved workspace fail to
//! restore? Each is reported, never acted on.
use super::PlexiApp;

impl PlexiApp {
    /// Log and notify once at startup. Build identity is compared by build id,
    /// so the managed Applications copy counts as the installed build.
    pub(crate) fn surface_launch_notices(&mut self) {
        let receipt = crate::distribution::installed().unwrap_or_else(|error| {
            log::warn!("launch identity: cannot read installation: {error}");
            None
        });
        match &receipt {
            Some(receipt) => match crate::distribution::installation_mismatch(
                env!("PLEXI_BUILD_ID"),
                crate::distribution::version_text(),
                receipt,
            ) {
                Some(notice) => {
                    log::warn!(
                        "launch identity: running build={} installed build={}: {notice}",
                        env!("PLEXI_BUILD_ID"),
                        receipt.active.build_id
                    );
                    self.queue_launch_notice("Plexi build mismatch", notice);
                }
                None => log::info!(
                    "launch identity: running {} is the installed build",
                    crate::distribution::version_text()
                ),
            },
            None => log::info!(
                "launch identity: running {} without a managed installation",
                crate::distribution::version_text()
            ),
        }
        #[cfg(target_os = "macos")]
        self.surface_stray_bundles(receipt.as_ref());
        self.surface_workspace_load_failure();
    }

    /// A saved workspace that could not be restored is reported once, with
    /// the file and the reason. The session's saves are already refused.
    pub(crate) fn surface_workspace_load_failure(&mut self) {
        let Some(block) = self.workspace_save_block.clone() else {
            return;
        };
        log::info!("workspace_load: surfacing restore failure to the user");
        self.enqueue_notification(
            crate::app::notifications::NotifySource::HostInternal,
            crate::app::notifications::PendingNotification {
                notify_id: crate::app::notifications::new_notify_id("workspace-load"),
                title: "Workspace not restored".to_string(),
                body: block,
                // The workspace file belongs to the whole host profile.
                scope: crate::protocol::NotifyScope::Global,
                ..Default::default()
            },
        );
    }

    /// Only the Applications folders are scanned here: Downloads and Desktop
    /// are privacy-protected and reading them at launch would prompt. The
    /// installer scans every candidate directory.
    #[cfg(target_os = "macos")]
    fn surface_stray_bundles(
        &mut self,
        receipt: Option<&plexi_distribution::transaction::Receipt>,
    ) {
        use plexi_distribution::{release, strays};
        let Some(home) = dirs::home_dir() else {
            return;
        };
        let channel = crate::config::build_channel().unwrap_or_else(|| "stable".into());
        let channel = release::normalized_channel(&channel);
        let dirs = [
            std::path::PathBuf::from("/Applications"),
            home.join("Applications"),
        ];
        let found = strays::find_for_installation(channel, &home, receipt, &dirs);
        if found.is_empty() {
            log::info!("launch identity: no stray {channel} bundles");
            return;
        }
        log::warn!(
            "launch identity: {} stray {channel} bundles: {found:?}",
            found.len()
        );
        let list = found
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join("\n");
        self.queue_launch_notice(
            "Old Plexi copies found",
            format!("These copies are not the managed installation and may launch an old build:\n{list}"),
        );
    }

    fn queue_launch_notice(&mut self, title: &str, body: String) {
        self.enqueue_notification(
            crate::app::notifications::NotifySource::HostInternal,
            crate::app::notifications::PendingNotification {
                notify_id: crate::app::notifications::new_notify_id("launch-identity"),
                title: title.to_string(),
                body,
                // Build identity is a property of the host, not of one context.
                scope: crate::protocol::NotifyScope::Global,
                ..Default::default()
            },
        );
    }
}

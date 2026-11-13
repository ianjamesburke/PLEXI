//! One-time launch checks: is this binary the channel's installed build, are
//! old copies of the plugin lying around, did the saved workspace fail to
//! restore, and does the profile still hold a data shape Plexi no longer
//! converts? Each is reported, never acted on.
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
        self.surface_retired_data_notices();
    }

    /// A saved workspace that could not be restored is reported once, with
    /// the file and the reason. The session's saves are already refused. A
    /// retired version 2 layout (stint 1312) arrives here too.
    pub(crate) fn surface_workspace_load_failure(&mut self) {
        let block = self.workspace_save_block();
        if let Some(block) = &block {
            log::error!("workspace_load: surfacing restore failure to the user: {block}");
        }
        self.refresh_integrity_notice("workspace-load", "Workspace not restored", block);
    }

    /// Notes and ledger shapes whose converters were deleted (stint 1312): a
    /// pre-tier notes folder and an `agent-ledger.jsonl` ledger. Each is
    /// reported with the file and the fix and never touched, so it is
    /// reported again on every launch until the user moves it.
    pub(crate) fn surface_retired_data_notices(&mut self) {
        let checks = [
            (
                "retired-notes",
                "Old notes folder found",
                crate::notes::pre_tier_notes_error(
                    self.workspace_load_failure
                        .as_ref()
                        .is_some_and(crate::workspace::WorkspaceLoadError::is_retired_version),
                ),
            ),
            (
                "retired-ledger",
                "Old AI ledger found",
                crate::plexi_ai::ledger::retired_ledger_error(),
            ),
        ];
        let mut reported = 0usize;
        for (condition, title, error) in checks {
            if let Some(error) = &error {
                log::error!("retired data: {error}");
                reported += 1;
            }
            self.refresh_integrity_notice(condition, title, error);
        }
        log::info!("retired data: launch check reported={reported}");
    }

    /// A notice about the user's own data. It ignores the notifications
    /// master switch and stays until acknowledged. The condition is standing,
    /// so it has one identity across launches: a notice restored from an
    /// earlier launch is replaced, never duplicated, and is cleared once the
    /// condition (`body` is `None`) no longer holds.
    fn refresh_integrity_notice(&mut self, condition: &str, title: &str, body: Option<String>) {
        let notify_id = crate::app::notifications::condition_notify_id(condition);
        let before = self.pending_notifications.len();
        self.pending_notifications
            .retain(|notification| notification.notify_id != notify_id);
        let replaced = before - self.pending_notifications.len();
        if self.current_notify_id.as_deref() == Some(notify_id.as_str()) {
            self.current_notify_id = None;
        }
        let Some(body) = body else {
            if replaced > 0 {
                log::info!("launch notice: {title:?} cleared, its condition no longer holds");
                self.save_notifications();
                self.sync_attention_queue();
            }
            return;
        };
        log::info!("launch notice: {title:?} replaced={replaced}");
        let queued = self.enqueue_notification(
            crate::app::notifications::NotifySource::HostIntegrity,
            crate::app::notifications::PendingNotification {
                notify_id,
                title: title.to_string(),
                body,
                // Profile data belongs to the whole host, not one context.
                scope: crate::protocol::NotifyScope::Global,
                required: true,
                ..Default::default()
            },
        );
        if !queued {
            log::error!("launch notice: {title:?} was not queued");
        }
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

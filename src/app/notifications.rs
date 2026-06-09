//! Notification queue helpers — visibility, sorting, selection, and mutation.

use super::PlexiApp;

#[derive(Clone)]
pub(crate) struct PendingNotification {
    pub notify_id: String,
    pub sender_pane_id: u64,
    /// Stable context identity the notification originated from (stamped at drain time).
    pub source_context_id: u64,
    /// Stable window identity the notification originated from. Used by
    /// `NotifyScope::Window` to restrict visibility to the originating window.
    pub source_window_id: u64,
    pub level: String,
    pub title: String,
    pub body: String,
    pub kind: crate::app_protocol::NotifyKind,
    pub options: Vec<crate::app_protocol::NotifyOption>,
    pub input_prompt: Option<String>,
    pub required: bool,
    /// Higher = more urgent. Used to pick next after dismiss + to order
    /// Cmd+]/Cmd+[ preview traversal. Arrival order (index in the queue
    /// Vec) breaks ties — oldest wins.
    pub priority: u32,
    /// Visibility scope. Affects which contexts the notification appears in.
    pub scope: crate::app_protocol::NotifyScope,
    /// Optional inline image attachment (#74). Decoded lazily on first
    /// render; oversized payloads (> 50 KB decoded) surface a placeholder
    /// instead of decoding. The decoded texture is cached separately on
    /// `PlexiApp::notification_images` keyed by `notify_id` — this struct
    /// stays Clone-cheap (no GPU handles inside it).
    pub image_inline: Option<crate::app_protocol::NotificationImage>,
    /// Optional pipe-referenced image attachment (#74). The host drains the
    /// matching binary ring on first render and caches the texture under
    /// `PlexiApp::notification_images`.
    pub image_pipe_id: Option<String>,
    /// Path to a file the CLI polls for the chosen key. Set when the
    /// notification was queued by `plexi notify --choice ...`. The host writes
    /// the chosen value here when the user picks an option so the blocking CLI
    /// process can read it and exit.
    pub response_file: Option<String>,
    pub timeout_secs: Option<u64>,
    pub on_dismiss: Option<String>,
    /// When the notification was pushed to the queue. Used for timeout tracking.
    pub enqueued_at: std::time::Instant,
    /// True when the originating app pane has exited. The notification stays
    /// in the queue so the user can read it, but action buttons are hidden.
    pub tombstoned: bool,
    /// When `Some(t)`, the notification is invisible and exempt from timeout
    /// until `t` has elapsed (snooze). `None` means deliver immediately.
    pub deliver_after: Option<std::time::Instant>,
}

/// Serializable snapshot of a `PendingNotification`. Session-only handles
/// (`image_pipe_id`, `response_file`, `deliver_after`) are dropped on save and
/// restored as `None`; `tombstoned` is forced to `true` on load because the
/// source pane is gone after a restart.
#[derive(serde::Serialize, serde::Deserialize)]
struct PersistedNotification {
    notify_id: String,
    sender_pane_id: u64,
    source_context_id: u64,
    #[serde(default)]
    source_window_id: u64,
    level: String,
    title: String,
    body: String,
    kind: crate::app_protocol::NotifyKind,
    options: Vec<crate::app_protocol::NotifyOption>,
    #[serde(default)]
    input_prompt: Option<String>,
    required: bool,
    priority: u32,
    scope: crate::app_protocol::NotifyScope,
    #[serde(default)]
    image_inline: Option<crate::app_protocol::NotificationImage>,
    #[serde(default)]
    timeout_secs: Option<u64>,
    #[serde(default)]
    on_dismiss: Option<String>,
    /// Unix timestamp (seconds since UNIX_EPOCH) when the notification was enqueued.
    enqueued_at_secs: u64,
    tombstoned: bool,
    // deliver_after (snooze) is session-only — not persisted.
    // image_pipe_id and response_file are session handles — not persisted.
}

/// Write `notifications` to `path` atomically (write temp, then rename).
/// Called at every mutation site so unread notifications survive restarts.
pub(crate) fn save_pending_notifications_to(
    notifications: &[PendingNotification],
    path: &std::path::Path,
) {
    let now_sys = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let persisted: Vec<PersistedNotification> = notifications
        .iter()
        .map(|n| {
            let age_secs = std::time::Instant::now()
                .duration_since(n.enqueued_at)
                .as_secs();
            let enqueued_at_secs = now_sys.saturating_sub(age_secs);
            PersistedNotification {
                notify_id: n.notify_id.clone(),
                sender_pane_id: n.sender_pane_id,
                source_context_id: n.source_context_id,
                source_window_id: n.source_window_id,
                level: n.level.clone(),
                title: n.title.clone(),
                body: n.body.clone(),
                kind: n.kind.clone(),
                options: n.options.clone(),
                input_prompt: n.input_prompt.clone(),
                required: n.required,
                priority: n.priority,
                scope: n.scope,
                image_inline: n.image_inline.clone(),
                timeout_secs: n.timeout_secs,
                on_dismiss: n.on_dismiss.clone(),
                enqueued_at_secs,
                tombstoned: n.tombstoned,
            }
        })
        .collect();
    match serde_json::to_string(&persisted) {
        Ok(json) => {
            let tmp = path.with_extension("json.tmp");
            match std::fs::write(&tmp, &json).and_then(|_| std::fs::rename(&tmp, path)) {
                Ok(_) => log::info!(
                    "notify:persist: saved {} notification(s)",
                    notifications.len()
                ),
                Err(e) => log::warn!("notify:persist: failed to write {:?}: {e}", path),
            }
        }
        Err(e) => log::warn!("notify:persist: failed to serialize: {e}"),
    }
}

/// Load persisted notifications from `path`. Drops entries older than 7 days.
/// All restored notifications are tombstoned — their source pane is gone.
pub(crate) fn load_pending_notifications_from(path: &std::path::Path) -> Vec<PendingNotification> {
    let Ok(json) = std::fs::read_to_string(path) else {
        return vec![];
    };
    let Ok(persisted) = serde_json::from_str::<Vec<PersistedNotification>>(&json) else {
        log::warn!("notify:persist: failed to deserialize {:?}", path);
        return vec![];
    };
    let now_sys = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    const TTL_SECS: u64 = 7 * 24 * 3600;
    let restored: Vec<PendingNotification> = persisted
        .into_iter()
        .filter_map(|p| {
            let age_secs = now_sys.saturating_sub(p.enqueued_at_secs);
            if age_secs > TTL_SECS {
                return None;
            }
            let enqueued_at = std::time::Instant::now()
                .checked_sub(std::time::Duration::from_secs(age_secs))
                .unwrap_or_else(std::time::Instant::now);
            Some(PendingNotification {
                notify_id: p.notify_id,
                sender_pane_id: p.sender_pane_id,
                source_context_id: p.source_context_id,
                source_window_id: p.source_window_id,
                level: p.level,
                title: p.title,
                body: p.body,
                kind: p.kind,
                options: p.options,
                input_prompt: p.input_prompt,
                required: p.required,
                priority: p.priority,
                scope: p.scope,
                image_inline: p.image_inline,
                image_pipe_id: None, // pipe handle gone after restart
                response_file: None, // file handle gone after restart
                timeout_secs: p.timeout_secs,
                on_dismiss: p.on_dismiss,
                enqueued_at,
                tombstoned: true,    // source pane is gone
                deliver_after: None, // snooze is session-only
            })
        })
        .collect();
    log::info!(
        "notify:persist: restored {} notification(s) from disk",
        restored.len()
    );
    restored
}

impl PlexiApp {
    /// Persist current pending notifications to `config_dir()/notifications.json`.
    pub(crate) fn save_notifications(&self) {
        save_pending_notifications_to(
            &self.pending_notifications,
            &crate::config::config_dir().join("notifications.json"),
        );
    }

    // ── Notification-queue helpers ──────────────────────────────────────────
    //
    // The notification modal tracks the currently-displayed entry by
    // `notify_id`, not by index. These helpers centralise the
    // priority-sort / selection logic so callers can't accidentally reach
    // past the end of the Vec or pick by stale offset.
    //
    // Sort order: `priority DESC, arrival-index ASC`. Arrival index = the
    // entry's current position in `pending_notifications`, which reflects
    // push order (we never reorder the Vec; dismissal removes by id).
    //
    // Visibility by scope:
    //   Window  — only when source_context == active context (default; most restrictive).
    //             Equivalent to Context in today's single-window-per-context model;
    //             the distinction will matter when multi-window contexts land.
    //   Context — same as Window today; reserved for the multi-window distinction.
    //   Global  — always visible.
    // The raw `pending_notifications` Vec stays flat; only the *view* changes
    // with the active workspace.

    /// True when this notification should appear in the current workspace view.
    pub(crate) fn notification_is_visible(&self, n: &PendingNotification) -> bool {
        if n.deliver_after
            .map_or(false, |t| t > std::time::Instant::now())
        {
            return false;
        }
        match n.scope {
            crate::app_protocol::NotifyScope::Global => true,
            crate::app_protocol::NotifyScope::Window => {
                n.source_window_id == self.windows[self.active_window].window_id
            }
            crate::app_protocol::NotifyScope::Context => {
                n.source_context_id == self.router.active().context_id
            }
        }
    }

    /// Return ids of all *visible* notifications (for the current context),
    /// ordered by (required desc, priority desc, arrival asc). Empty Vec when none visible.
    pub(crate) fn sorted_notification_ids(&self) -> Vec<String> {
        let mut indexed: Vec<(usize, u32, bool, &str)> = self
            .pending_notifications
            .iter()
            .enumerate()
            .filter(|(_, n)| self.notification_is_visible(n))
            .map(|(i, n)| (i, n.priority, n.required, n.notify_id.as_str()))
            .collect();
        // required pins to top, then priority DESC, ties broken by arrival ASC.
        indexed.sort_by(|a, b| b.2.cmp(&a.2).then(b.1.cmp(&a.1)).then(a.0.cmp(&b.0)));
        indexed
            .into_iter()
            .map(|(_, _, _, id)| id.to_string())
            .collect()
    }

    /// Return the id of the highest-priority *visible* notification,
    /// breaking ties by oldest arrival. `None` when none visible.
    pub(crate) fn select_highest_priority(&self) -> Option<String> {
        self.sorted_notification_ids().into_iter().next()
    }

    /// (1-based position-in-sort-order, total visible len) for the current
    /// notify id, or `None` when modal is empty / current id is missing from
    /// visible queue. Renderer uses this for the "X of N" indicator.
    pub(crate) fn position_of_current(&self) -> Option<(usize, usize)> {
        let current = self.current_notify_id.as_ref()?;
        let sorted = self.sorted_notification_ids();
        let pos = sorted.iter().position(|id| id == current)?;
        Some((pos + 1, sorted.len()))
    }

    /// Move `current_notify_id` forward (`direction = 1`) or backward
    /// (`direction = -1`) through the visible priority-sorted queue. No wrap
    /// at the ends. Called by Cmd+] / Cmd+[.
    pub(crate) fn cycle_notification(&mut self, direction: i32) {
        if !self.show_notification_modal {
            return;
        }
        let sorted = self.sorted_notification_ids();
        if sorted.is_empty() {
            return;
        }
        let Some(current) = self.current_notify_id.as_ref() else {
            // Queue has entries but nothing is current — pick highest.
            self.current_notify_id = sorted.into_iter().next();
            return;
        };
        let Some(pos) = sorted.iter().position(|id| id == current) else {
            // Current id not in visible queue any more (context switch or dismiss).
            // Fall back to highest-priority visible.
            self.current_notify_id = sorted.into_iter().next();
            return;
        };
        let next_pos = match direction {
            d if d > 0 && pos + 1 < sorted.len() => pos + 1,
            d if d < 0 && pos > 0 => pos - 1,
            _ => return, // clamp at both ends
        };
        self.current_notify_id = Some(sorted[next_pos].clone());
    }

    /// Check every pending notification for expiry. For each that has exceeded
    /// its `timeout_secs`, deliver a `NotifyAction` dismiss event and remove it.
    /// Also wakes snoozed notifications whose `deliver_after` has elapsed and
    /// auto-reopens the modal when a high-priority one wakes. Called once per
    /// second from `update()`.
    pub(crate) fn tick_notification_timeouts(&mut self) {
        let now = std::time::Instant::now();
        let threshold = self.notifications_interrupt_threshold;
        let focus_mode = self.notifications_focus_mode;
        let mut expired_ids: Vec<String> = Vec::new();
        let mut woken_priority_met = false;
        // Single mutable pass: wake snoozed entries, collect expired ids.
        for n in &mut self.pending_notifications {
            if let Some(t) = n.deliver_after {
                if t > now {
                    continue; // still snoozed — skip timeout check too
                }
                if !focus_mode && n.priority >= threshold {
                    woken_priority_met = true;
                }
                log::info!("notify:snooze: woke notify_id={}", n.notify_id);
                n.deliver_after = None;
                n.enqueued_at = now;
            }
            if let Some(timeout) = n.timeout_secs {
                if n.enqueued_at.elapsed() >= std::time::Duration::from_secs(timeout) {
                    expired_ids.push(n.notify_id.clone());
                }
            }
        }
        if woken_priority_met {
            self.show_notification_modal = true;
            if self.current_notify_id.is_none() {
                self.current_notify_id = self.select_highest_priority();
            }
        }
        for id in &expired_ids {
            let Some(pos) = self
                .pending_notifications
                .iter()
                .position(|n| &n.notify_id == id)
            else {
                continue;
            };
            let n = self.pending_notifications.remove(pos);
            let dismiss_value = n
                .on_dismiss
                .clone()
                .unwrap_or_else(|| "timeout".to_string());
            log::info!(
                "notification '{}' timed out after {}s — delivering on_dismiss='{}'",
                n.title,
                n.timeout_secs.unwrap_or(0),
                dismiss_value
            );
            if !n.notify_id.is_empty() && !n.notify_id.starts_with("__host__:") {
                let cmds = vec![crate::app::app_trait::AppCommand::DeliverNotifyAction {
                    pane_id: n.sender_pane_id,
                    notify_id: n.notify_id.clone(),
                    action_label: "timeout".to_string(),
                    value: Some(dismiss_value),
                    response_file: n.response_file.clone(),
                    host_action: None,
                }];
                self.dispatch_notify_action_cmds(cmds);
            }
            // If this was the pinned notification, clear it so the next highest
            // becomes current on the next frame.
            if self.current_notify_id.as_deref() == Some(&n.notify_id) {
                self.current_notify_id = None;
            }
        }
        if !expired_ids.is_empty() {
            self.save_notifications();
        }
    }

    /// Mark all pending notifications from `pane_id` as tombstoned. Called
    /// when an app pane is closed. Tombstoned notifications remain in the queue
    /// so the user can read them, but their action buttons are hidden.
    pub(crate) fn tombstone_pane_notifications(&mut self, pane_id: crate::spatial::tiling::PaneId) {
        for n in &mut self.pending_notifications {
            if n.sender_pane_id == pane_id {
                n.tombstoned = true;
                log::info!(
                    "notification '{}' tombstoned (pane {pane_id} closed)",
                    n.title
                );
            }
        }
        self.save_notifications();
    }

    /// Count of window- or context-scoped notifications whose source_context_id == the id of ctx_idx.
    /// Used for per-context sidebar badges on inactive contexts. Global notifications
    /// are excluded — they already appear everywhere via notification_is_visible.
    /// Returns 0 for parked contexts — they produce no visual noise.
    pub(crate) fn context_notification_count(&self, ctx_idx: usize) -> usize {
        let ctx = self.router.get(ctx_idx);
        if ctx.parked {
            return 0;
        }
        let ctx_id = ctx.context_id;
        self.pending_notifications
            .iter()
            .filter(|n| {
                matches!(
                    n.scope,
                    crate::app_protocol::NotifyScope::Window
                        | crate::app_protocol::NotifyScope::Context
                ) && n.source_context_id == ctx_id
            })
            .count()
    }

    /// Recursively sum notification count for a context and all its descendants.
    /// Used for the notification badge on Portal tiles.
    pub(crate) fn context_notification_count_recursive(&self, ctx_id: u64) -> usize {
        self.context_notification_count_recursive_limited(ctx_id, 0)
    }

    fn context_notification_count_recursive_limited(&self, ctx_id: u64, depth: u32) -> usize {
        // Depth > 3 is impossible in valid data (creation is capped), but guard
        // against cycles in manually-edited workspace files to prevent stack overflow.
        if depth > 3 {
            return 0;
        }
        let direct = self
            .pending_notifications
            .iter()
            .filter(|n| {
                matches!(
                    n.scope,
                    crate::app_protocol::NotifyScope::Window
                        | crate::app_protocol::NotifyScope::Context
                ) && n.source_context_id == ctx_id
            })
            .count();
        direct
            + self
                .router
                .iter()
                .filter(|c| c.parent_id == Some(ctx_id))
                .map(|c| self.context_notification_count_recursive_limited(c.context_id, depth + 1))
                .sum::<usize>()
    }

    /// Count of visible notifications for the active context (context-scoped
    /// from active + all globals). Used for the toolbar badge.
    pub(crate) fn visible_notification_count(&self) -> usize {
        self.pending_notifications
            .iter()
            .filter(|n| self.notification_is_visible(n))
            .count()
    }

    /// Auto-dismiss non-required notifications whose `sender_pane_id` matches
    /// the currently focused pane. Called once per frame from `update_preamble`.
    ///
    /// Required notifications (interactive prompts that need an explicit
    /// response) are exempt and must always be dismissed manually.
    pub(crate) fn auto_dismiss_sender_focused_notifications(&mut self) {
        let win = &self.windows[self.active_window];
        let focused_pane_id = win
            .focused_pane
            .and_then(|tile_id| Self::find_pane_in_tile(&win.tree, tile_id));

        let Some(focused_id) = focused_pane_id else {
            return;
        };

        let auto_dismiss_ids: Vec<String> = self
            .pending_notifications
            .iter()
            .filter(|n| !n.required && n.sender_pane_id == focused_id)
            .map(|n| n.notify_id.clone())
            .collect();

        if auto_dismiss_ids.is_empty() {
            return;
        }

        for id in &auto_dismiss_ids {
            let Some(pos) = self
                .pending_notifications
                .iter()
                .position(|n| &n.notify_id == id)
            else {
                continue;
            };
            let n = self.pending_notifications.remove(pos);
            log::info!(
                "notify:auto_dismiss: pane_id={focused_id} focused — dismissing notify_id={} title={:?}",
                n.notify_id,
                n.title
            );
            if self.current_notify_id.as_deref() == Some(&n.notify_id) {
                self.current_notify_id = None;
            }
        }

        if self.show_notification_modal && self.sorted_notification_ids().is_empty() {
            self.show_notification_modal = false;
            log::info!("notify:auto_dismiss: modal closed — no visible notifications remain");
        }

        self.save_notifications();
    }
}

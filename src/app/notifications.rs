//! Notification queue helpers — visibility, sorting, selection, and mutation.

use super::PlexiApp;

#[derive(Clone)]
pub(crate) struct PendingNotification {
    pub notify_id: String,
    pub sender_pane_id: u64,
    /// Host-resolved pane id allowed to dismiss this notification via
    /// `plexi notify dismiss` (`DismissNotification`'s `peer_pid` resolved
    /// through `resolve_socket_peer_pane`). 0 = no verified sender —
    /// unreachable by any dismiss request. Distinct from `sender_pane_id`,
    /// which drives auto-dismiss-on-focus and is deliberately left 0 for CLI
    /// notifications regardless of sender identity.
    pub dismiss_owner_pane_id: u64,
    /// Stable context identity the notification originated from (stamped at drain time).
    pub source_context_id: u64,
    /// Stable window identity the notification originated from. Used by
    /// `NotifyScope::Window` to restrict visibility to the originating window.
    pub source_window_id: u64,
    pub title: String,
    pub body: String,
    pub kind: crate::app_protocol::NotifyKind,
    pub options: Vec<crate::app_protocol::NotifyOption>,
    pub input_prompt: Option<String>,
    pub required: bool,
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
    /// Session-only (never persisted — see `PersistedNotification`): whether
    /// `sender_pane_id` was in view (per `PlexiApp::pane_is_in_view`) as of
    /// the last frame. Drives the resurface-on-navigation-into-view policy in
    /// `PlexiApp::resurface_in_view_notifications` — a rising edge (false →
    /// true) re-opens the modal once; it stays flat while the pane remains in
    /// view, so an explicit dismissal in that state is final until the pane
    /// leaves view and returns. Restored notifications always start `false`.
    pub origin_in_view: bool,
}

/// Serializable snapshot of a `PendingNotification`. Session-only handles
/// (`image_pipe_id`, `response_file`, `deliver_after`) are dropped on save and
/// restored as `None`; `tombstoned` is forced to `true` on load because the
/// source pane is gone after a restart.
#[derive(serde::Serialize, serde::Deserialize)]
struct PersistedNotification {
    notify_id: String,
    sender_pane_id: u64,
    #[serde(default)]
    dismiss_owner_pane_id: u64,
    source_context_id: u64,
    #[serde(default)]
    source_window_id: u64,
    title: String,
    body: String,
    kind: crate::app_protocol::NotifyKind,
    options: Vec<crate::app_protocol::NotifyOption>,
    #[serde(default)]
    input_prompt: Option<String>,
    required: bool,
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
                dismiss_owner_pane_id: n.dismiss_owner_pane_id,
                source_context_id: n.source_context_id,
                source_window_id: n.source_window_id,
                title: n.title.clone(),
                body: n.body.clone(),
                kind: n.kind.clone(),
                options: n.options.clone(),
                input_prompt: n.input_prompt.clone(),
                required: n.required,
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
                // Forced to 0 (unowned) on restore, same reasoning as
                // `tombstoned: true` below: pane ids are re-issued fresh each
                // session, so a persisted owner id could otherwise
                // misattribute a restored notification to an unrelated pane
                // that happens to reuse the same id post-restart.
                dismiss_owner_pane_id: 0,
                source_context_id: p.source_context_id,
                source_window_id: p.source_window_id,
                title: p.title,
                body: p.body,
                kind: p.kind,
                options: p.options,
                input_prompt: p.input_prompt,
                required: p.required,
                scope: p.scope,
                image_inline: p.image_inline,
                image_pipe_id: None, // pipe handle gone after restart
                response_file: None, // file handle gone after restart
                timeout_secs: p.timeout_secs,
                on_dismiss: p.on_dismiss,
                enqueued_at,
                tombstoned: true,      // source pane is gone
                deliver_after: None,   // snooze is session-only
                origin_in_view: false, // session-only, re-derived each frame
            })
        })
        .collect();
    log::info!(
        "notify:persist: restored {} notification(s) from disk",
        restored.len()
    );
    restored
}

/// Which surface a notification entered the host through. Recorded on the
/// arrival trace so the log names the door, not just the notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NotifySource {
    /// `plexi notify` over the command socket.
    Cli,
    /// An app pane's `ShowNotification` command.
    App,
    /// A WASM guest's `Notify` host effect.
    Wasm,
    /// Raised by the host itself (config errors and the like).
    HostInternal,
}

impl NotifySource {
    fn as_str(self) -> &'static str {
        match self {
            NotifySource::Cli => "cli",
            NotifySource::App => "app",
            NotifySource::Wasm => "wasm",
            NotifySource::HostInternal => "host",
        }
    }
}

/// Core scope-match: whether a notification owned at `scope` — carrying its
/// `source_window_id`/`source_context_id` — is visible to a viewer sitting at
/// `active_window_id`/`active_context_id`. Plain scalar ids, not borrowed
/// `PendingNotification`/`PlexiApp`, so this can be called from a site that
/// already holds a conflicting borrow (see the `render.rs` call site).
///
/// Pure scope comparison only — does NOT consider snooze (`deliver_after`),
/// tombstone, or queue state. Callers that need the full visibility answer
/// combine this with their own checks (see `PlexiApp::notification_is_visible`).
pub(crate) fn notification_visible(
    scope: crate::app_protocol::NotifyScope,
    source_window_id: u64,
    source_context_id: u64,
    active_window_id: u64,
    active_context_id: u64,
) -> bool {
    match scope {
        crate::app_protocol::NotifyScope::Global => true,
        crate::app_protocol::NotifyScope::Window => source_window_id == active_window_id,
        crate::app_protocol::NotifyScope::Context => source_context_id == active_context_id,
    }
}

/// Whether a notification at `scope`/`source_context_id` counts toward the
/// badge for context `ctx_id`. `Global` never counts — it already renders
/// everywhere via [`notification_visible`], so counting it again on every
/// context's badge would double-report it. `Window`/`Context` count toward
/// their originating context's badge regardless of that context's snooze
/// state (badge counts have never consulted `deliver_after` — preserved as
/// existing behavior, not introduced here).
pub(crate) fn notification_counts_toward_context(
    scope: crate::app_protocol::NotifyScope,
    source_context_id: u64,
    ctx_id: u64,
) -> bool {
    matches!(
        scope,
        crate::app_protocol::NotifyScope::Window | crate::app_protocol::NotifyScope::Context
    ) && source_context_id == ctx_id
}

impl PlexiApp {
    /// Resolve app-notification provenance without consulting focus. A live
    /// sender pane supplies its window; a parked or gone sender has only the
    /// context stamped on the command, so window visibility narrows to that
    /// context rather than borrowing the currently active window.
    pub(crate) fn resolve_app_notification_provenance(
        &self,
        sender_pane_id: u64,
        scope: crate::app_protocol::NotifyScope,
    ) -> (crate::app_protocol::NotifyScope, u64) {
        if let Some((window_index, _)) = (sender_pane_id != 0)
            .then(|| self.find_pane_in_any_window(sender_pane_id))
            .flatten()
        {
            return (scope, self.windows[window_index].window_id);
        }
        if scope == crate::app_protocol::NotifyScope::Window {
            log::info!(
                "notify: app sender pane_id={} has no live window — narrowing window scope to context",
                sender_pane_id
            );
            return (crate::app_protocol::NotifyScope::Context, 0);
        }
        (scope, 0)
    }

    /// Remove a CLI notification only when the caller owns its stamped pane.
    /// This is deliberately a queue mutation rather than
    /// a UI-only close so sticky entries cannot survive a CLI dismissal.
    ///
    /// `resolved_pane_id` is the host-established sender pane — resolved by
    /// `handle_pane_ipc_request` from the socket peer's OS credential via
    /// `resolve_socket_peer_pane`, never from the request's own fields.
    /// Ownership is checked against `PendingNotification::dismiss_owner_pane_id`
    /// (stamped the same way at enqueue time), not by re-parsing `notify_id` —
    /// `notify_id` is client-generated (`plexi notify`'s `"cli:{pane_id}:{uuid}"`),
    /// so comparing it against a client-supplied pane id was comparing two
    /// attacker-controlled values against each other and proved nothing.
    /// `0` means either side has no verified owner and can never match.
    pub(crate) fn dismiss_notification_from_sender(
        &mut self,
        notify_id: &str,
        resolved_pane_id: u64,
    ) -> Result<(), &'static str> {
        let Some(position) = self
            .pending_notifications
            .iter()
            .position(|n| n.notify_id == notify_id)
        else {
            return Err("notification not found or not owned by caller");
        };
        if resolved_pane_id == 0
            || self.pending_notifications[position].dismiss_owner_pane_id != resolved_pane_id
        {
            return Err("notification not found or not owned by caller");
        }
        self.pending_notifications.remove(position);
        if self.current_notify_id.as_deref() == Some(notify_id) {
            self.current_notify_id = None;
        }
        if self.pending_notifications.is_empty() {
            self.show_notification_modal = false;
        }
        self.save_notifications();
        log::info!(
            "notify: dismissed id={} resolved_pane_id={}",
            notify_id,
            resolved_pane_id
        );
        Ok(())
    }

    /// The single choke point for every notification entering the host.
    ///
    /// Owns, in one place, the policy that used to be re-decided at each of the
    /// four call sites: the `notifications_enabled` master switch, the push
    /// onto the queue, persistence, the audible cue, the auto-open decision,
    /// and the `NotificationPosted` event-log emit. Call sites contribute only
    /// the notification's own data — none of them may keep a local copy of any
    /// of the above, and a fifth surface cannot skip the gate without deleting
    /// code here.
    ///
    /// Returns `true` when the notification was queued, `false` when the master
    /// switch dropped it.
    pub(crate) fn enqueue_notification(
        &mut self,
        source: NotifySource,
        mut notification: PendingNotification,
    ) -> bool {
        if !self.notifications_enabled {
            log::info!(
                "notify: dropped source={} title={:?} — notifications disabled",
                source.as_str(),
                notification.title
            );
            return false;
        }

        // Stamp the arrival-time in-view state so the very act of raising a
        // notification is never itself counted as a resurface — only a later
        // transition from out-of-view to in-view fires
        // `resurface_in_view_notifications`.
        notification.origin_in_view = self.pane_is_in_view(notification.sender_pane_id);

        let notify_id = notification.notify_id.clone();
        let title = notification.title.clone();

        // `source_context_id` is stamped on every notification regardless of
        // whether the sender pane is still live (CLI notifications may have
        // sender_pane_id == 0), so resolve routing from it via the router
        // rather than `origin_for_pane`, which needs a live pane.
        let context_root = self.context_root_for(notification.source_context_id);
        crate::host::event_log::emit_scoped(
            crate::host::event_log::HostEvent::NotificationPosted {
                id: notify_id.clone(),
                title: title.clone(),
                timestamp: crate::host::event_log::now_timestamp(),
            },
            context_root.as_deref(),
        );

        let is_visible = self.notification_is_visible(&notification);
        let may_interrupt = self.notification_may_interrupt(&notification);
        let cue_played = self.play_notification_cue(&notification);
        self.pending_notifications.push(notification);
        self.save_notifications();

        if may_interrupt {
            self.show_notification_modal = true;
            // Only pin the arrival as current when nothing is already pinned.
            if self.current_notify_id.is_none() {
                self.current_notify_id = Some(notify_id.clone());
            }
        }

        log::info!(
            "notify: queued source={} id={} title={:?} visible={} interrupt={} cue={}",
            source.as_str(),
            notify_id,
            title,
            is_visible,
            may_interrupt,
            cue_played
        );
        true
    }

    /// The one interruption decision for a notification, consumed by every
    /// surface that interrupts the user: the modal auto-open, the audible cue,
    /// and the snooze-wake reopen. They all answer the same question — "may
    /// this notification pull the user's attention right now?" — and splitting
    /// that predicate is exactly how the cue once fired for notifications the
    /// user could not even see. Gates, in order:
    ///   1. Visibility — Global always; Window/Context only when the source
    ///      matches the active window/context (and the entry is not snoozed).
    ///   2. focus_mode off — the global mute gate.
    ///
    /// A notification that fails either gate still queues and only updates the
    /// badge.
    pub(crate) fn notification_may_interrupt(&self, n: &PendingNotification) -> bool {
        self.notification_is_visible(n)
            && !self.notifications_focus_mode
    }

    /// The audible cue to play for an arriving notification, or `None` to stay
    /// silent. Split out from [`Self::enqueue_notification`] so the decision —
    /// and the parameters it resolves — are observable without touching an
    /// audio device.
    ///
    /// Silent when no `[notifications] sound` is configured, and whenever
    /// [`Self::notification_may_interrupt`] says the notification may not
    /// interrupt — the cue never fires for a notification the modal would not
    /// show. The `enabled = false` case never reaches here — the master switch
    /// drops the notification first.
    pub(crate) fn notification_cue_request(
        &self,
        notification: &PendingNotification,
    ) -> Option<crate::media::audio::PlaybackRequest> {
        if !self.notification_may_interrupt(notification) {
            return None;
        }
        let source = self.notifications_sound.as_ref()?;
        Some(crate::media::audio::PlaybackRequest {
            source: source.clone(),
            volume: 1.0,
        })
    }

    /// Play the configured arrival cue, if any. Returns whether a sound started.
    ///
    /// A cue that fails to play is logged and swallowed deliberately: a missing
    /// or undecodable sound file must never cost the user the notification
    /// itself.
    fn play_notification_cue(&mut self, notification: &PendingNotification) -> bool {
        let Some(request) = self.notification_cue_request(notification) else {
            return false;
        };
        let source = request.source.clone();
        match crate::media::audio::start_playback(request) {
            Ok(session) => {
                // Held on the app: dropping the session stops playback.
                self.notification_cue_playback = Some(session);
                true
            }
            Err(error) => {
                log::warn!("notify: cue playback failed source={source}: {error}");
                false
            }
        }
    }

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
    // selection logic so callers can't accidentally reach
    // past the end of the Vec or pick by stale offset.
    //
    // Sort order: `required DESC, arrival-index ASC`. Arrival index is the
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
            .is_some_and(|t| t > std::time::Instant::now())
        {
            return false;
        }
        notification_visible(
            n.scope,
            n.source_window_id,
            n.source_context_id,
            self.windows[self.active_window].window_id,
            self.router.active().context_id,
        )
    }

    /// Return ids of all *visible* notifications (for the current context),
    /// ordered by (required desc, arrival asc). Empty Vec when none visible.
    pub(crate) fn sorted_notification_ids(&self) -> Vec<String> {
        let mut indexed: Vec<(usize, bool, &str)> = self
            .pending_notifications
            .iter()
            .enumerate()
            .filter(|(_, n)| self.notification_is_visible(n))
            .map(|(i, n)| (i, n.required, n.notify_id.as_str()))
            .collect();
        // Required notifications stay first; otherwise the oldest arrival wins.
        indexed.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        indexed
            .into_iter()
            .map(|(_, _, id)| id.to_string())
            .collect()
    }

    /// Return the first *visible* notification by required then arrival order.
    pub(crate) fn select_next_notification(&self) -> Option<String> {
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
    /// (`direction = -1`) through the visible queue. No wrap
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
            // Queue has entries but nothing is current — pick the next entry.
            self.current_notify_id = sorted.into_iter().next();
            return;
        };
        let Some(pos) = sorted.iter().position(|id| id == current) else {
            // Current id not in visible queue any more (context switch or dismiss).
            // Fall back to the next visible entry.
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
    /// auto-reopens the modal when one may interrupt. Called once per
    /// second from `update()`.
    pub(crate) fn tick_notification_timeouts(&mut self) {
        let now = std::time::Instant::now();
        let mut expired_ids: Vec<String> = Vec::new();
        let mut woken_ids: Vec<String> = Vec::new();
        // Single mutable pass: wake snoozed entries, collect expired ids.
        for n in &mut self.pending_notifications {
            if let Some(t) = n.deliver_after {
                if t > now {
                    continue; // still snoozed — skip timeout check too
                }
                log::info!("notify:snooze: woke notify_id={}", n.notify_id);
                n.deliver_after = None;
                n.enqueued_at = now;
                woken_ids.push(n.notify_id.clone());
            }
            if let Some(timeout) = n.timeout_secs {
                if n.enqueued_at.elapsed() >= std::time::Duration::from_secs(timeout) {
                    expired_ids.push(n.notify_id.clone());
                }
            }
        }
        // A wake is a second arrival for interruption purposes, so it takes
        // the same single decision as enqueue — a snoozed notification from an
        // inactive context no longer reopens the modal where it isn't visible.
        let woken_notification_may_interrupt = self
            .pending_notifications
            .iter()
            .filter(|n| woken_ids.contains(&n.notify_id))
            .any(|n| self.notification_may_interrupt(n));
        if woken_notification_may_interrupt {
            self.show_notification_modal = true;
            if self.current_notify_id.is_none() {
                self.current_notify_id = self.select_next_notification();
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
            .filter(|n| notification_counts_toward_context(n.scope, n.source_context_id, ctx_id))
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
            .filter(|n| notification_counts_toward_context(n.scope, n.source_context_id, ctx_id))
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

    /// True when `pane_id` is a pane a person would actually be looking at
    /// right now: it sits in the currently-displayed context of the
    /// currently-focused window. This is a *layout* predicate — it does not
    /// consult focus, keyboard input, or paint/occlusion state, so it answers
    /// the same way on a hidden/occluded frame as on a visible one.
    ///
    /// `pane_id == 0` (CLI-raised notifications have no origin pane) is
    /// always out of view — there is no pane to resurface toward.
    pub(crate) fn pane_is_in_view(&self, pane_id: u64) -> bool {
        if pane_id == 0 {
            return false;
        }
        let win = &self.windows[self.active_window];
        if win.context_id != self.router.active().context_id {
            return false;
        }
        if let Some(zoomed_tile) = win.zoomed_pane {
            // Zoomed fullscreen hides every sibling pane from view — only the
            // zoomed pane itself counts.
            return Self::find_pane_in_tile(&win.tree, zoomed_tile) == Some(pane_id);
        }
        let Some(root) = win.tree.root else {
            return false;
        };
        crate::spatial::tiling::collect_pane_ids_spatial(&win.tree.tiles, root).contains(&pane_id)
    }

    /// Resurface any notification whose origin pane has just come into view
    /// (a rising edge on `pane_is_in_view`). Called once per frame from
    /// `update_preamble`, which runs from `App::logic` — this must never move
    /// into a `ui`-only path, or a hidden/occluded host would never resurface
    /// a notification for a pane the user just navigated to.
    ///
    /// One resurface per navigation-into-view: `origin_in_view` is the policy
    /// state itself, not just a cache. It only re-fires after the pane leaves
    /// view (edge goes low) and returns (edge goes high again), so an
    /// explicit dismissal while the pane stays in view is final.
    pub(crate) fn resurface_in_view_notifications(&mut self) {
        // Read-only pass first — collecting rising-edge ids avoids mutating
        // `self.pending_notifications` while borrowing `self` immutably for
        // `pane_is_in_view` / `notification_may_interrupt`.
        let mut rising_edges: Vec<(String, u64, bool)> = Vec::new();
        for n in &self.pending_notifications {
            let in_view = self.pane_is_in_view(n.sender_pane_id);
            let rose = in_view && !n.origin_in_view;
            rising_edges.push((n.notify_id.clone(), n.sender_pane_id, rose));
        }

        let active_window_id = self.windows[self.active_window].window_id;
        let active_context_id = self.router.active().context_id;

        for (notify_id, sender_pane_id, rose) in rising_edges {
            let Some(pos) = self
                .pending_notifications
                .iter()
                .position(|n| n.notify_id == notify_id)
            else {
                continue;
            };
            // Always track the latest in-view state, whether or not this pass
            // fires a resurface.
            let in_view = self.pane_is_in_view(sender_pane_id);
            self.pending_notifications[pos].origin_in_view = in_view;

            if !rose {
                continue;
            }
            if !self.notification_may_interrupt(&self.pending_notifications[pos]) {
                continue;
            }

            self.show_notification_modal = true;
            if self.current_notify_id.is_none() {
                self.current_notify_id = Some(notify_id.clone());
            }

            log::info!(
                "notify:resurface: notify_id={notify_id} title={:?} origin_pane={sender_pane_id} window_id={active_window_id} context_id={active_context_id}",
                self.pending_notifications[pos].title
            );
        }
    }
}
